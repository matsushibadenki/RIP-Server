use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_secs() as i64
}
pub(crate) const LEGACY_DIGEST: &str = "org.jrip.serde-json-v1";
pub(crate) fn selected_digest(headers: &HeaderMap) -> Result<&'static str, ApiError> {
    if headers.get_all("x-jrip-digest-profile").iter().count() > 1 {
        return Err(bad("invalid_digest_profile"));
    }
    match headers.get("x-jrip-digest-profile").map(|h| h.to_str()) {
        None => Ok(rip_digest::PROFILE),
        Some(Ok(rip_digest::PROFILE)) => Ok(rip_digest::PROFILE),
        Some(Ok(LEGACY_DIGEST)) => Ok(LEGACY_DIGEST),
        _ => Err(bad("invalid_digest_profile")),
    }
}
pub(crate) fn digest_with_profile(value: &Value, profile: &str) -> Result<String, ApiError> {
    match profile {
        rip_digest::PROFILE => {
            rip_digest::digest(value).map_err(|_| bad("canonical_input_unsupported"))
        }
        LEGACY_DIGEST => Ok(format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(value).map_err(internal)?)
        )),
        _ => Err(bad("invalid_digest_profile")),
    }
}
pub(crate) fn stored_digest_profile(
    payload: &Value,
    pipeline: bool,
) -> Result<&'static str, ApiError> {
    match (
        pipeline,
        payload["profile"].as_str(),
        payload["digestProfile"].as_str(),
    ) {
        (false, Some("org.jrip.settings-preflight-v1"), Some(LEGACY_DIGEST))
        | (true, Some("org.jrip.pipeline-preflight-v1"), Some(LEGACY_DIGEST)) => Ok(LEGACY_DIGEST),
        (false, Some("org.jrip.settings-preflight-v2"), Some(rip_digest::PROFILE))
        | (true, Some("org.jrip.pipeline-preflight-v2"), Some(rip_digest::PROFILE)) => {
            Ok(rip_digest::PROFILE)
        }
        _ => Err(bad("invalid_preflight_profile")),
    }
}
pub async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    Json(intent): Json<rip_acx::intent::PrintIntent>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&app, &headers, "jobs:submit")?;
    let digest_profile = selected_digest(&headers)?;
    let resolution = rip_acx::intent::resolve(intent);
    if resolution["executable"] != true {
        return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(resolution)));
    }
    let id = Uuid::new_v4();
    let issued = now();
    let ticket = &resolution["ticket"];
    let mut payload = json!({
        "acx":"0.1", "profile":if digest_profile==LEGACY_DIGEST {"org.jrip.settings-preflight-v1"}else{"org.jrip.settings-preflight-v2"},
        "preflightId":id,"capability":rip_acx::RIP_CAPABILITY_ID,
        "issuedAt":issued,"expiresAt":issued+300,
        "ticket":ticket,"requestHash":digest_with_profile(ticket,digest_profile)?,
        "digestProfile":digest_profile,
        "adjustments":resolution["adjustments"],
        "effects":["compute","persistent-storage","raster-output"],
        "authorizationGranted":false,"documentVerified":false,
        "commitAvailable":true,
        "messages":resolution["messages"]
    });
    let hash = digest_with_profile(&payload, digest_profile)?;
    payload["preflightDigest"] = json!(hash);
    app.repo.save_preflight(id, &payload, issued + 300).await?;
    Ok((StatusCode::CREATED, Json(payload)))
}
pub async fn get(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:submit")?;
    let payload = app.repo.get_preflight(id).await?;
    if matches!(
        payload["profile"].as_str(),
        Some("org.jrip.pipeline-preflight-v1" | "org.jrip.pipeline-preflight-v2")
    ) {
        pipeline::scopes(&app, &headers)?;
        if payload["actor"] != pipeline::actor(&app, &headers)? {
            return Err(ApiError(StatusCode::FORBIDDEN, "preflight_actor_mismatch"));
        }
    }
    if payload["expiresAt"].as_i64().unwrap_or(0) <= now() {
        return Err(ApiError(StatusCode::GONE, "preflight_expired"));
    }
    Ok(Json(payload))
}

pub async fn commit(
    State(app): State<App>,
    headers: HeaderMap,
    Path(preflight_id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&app, &headers, "jobs:submit")?;
    let payload = app.repo.get_preflight(preflight_id).await?;
    if matches!(
        payload["profile"].as_str(),
        Some("org.jrip.pipeline-preflight-v1" | "org.jrip.pipeline-preflight-v2")
    ) {
        return Err(bad("pipeline_commit_endpoint_required"));
    }
    let digest_profile = stored_digest_profile(&payload, false)?;
    let supplied = headers
        .get("x-jrip-preflight-digest")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let mut unsigned = payload.clone();
    unsigned
        .as_object_mut()
        .ok_or_else(|| bad("invalid_preflight"))?
        .remove("preflightDigest");
    if supplied != payload["preflightDigest"].as_str().unwrap_or("")
        || supplied != digest_with_profile(&unsigned, digest_profile)?
    {
        return Err(ApiError(StatusCode::CONFLICT, "preflight_digest_mismatch"));
    }
    // Completed commits stay idempotent even after their original approval window closes.
    if let Some(id) = app.repo.committed_job(preflight_id).await? {
        return Ok((
            StatusCode::OK,
            Json(
                json!({"job":app.repo.get(id).await?,"preflightId":preflight_id,"replayed":true,"receiptUrl":format!("/api/v1/jobs/{}/receipt",id)}),
            ),
        ));
    }
    if payload["expiresAt"].as_i64().unwrap_or(0) <= now() {
        return Err(ApiError(StatusCode::GONE, "preflight_expired"));
    }
    let manifest: Manifest =
        serde_json::from_value(payload["ticket"]["manifest"].clone()).map_err(internal)?;
    let expected = payload["ticket"]["document"]["sha256"]
        .as_str()
        .ok_or_else(|| bad("invalid_preflight"))?;
    let id = Uuid::new_v4();
    let directory = app.root.join("jobs").join(id.to_string());
    tokio::fs::create_dir(&directory).await.map_err(internal)?;
    let _writer = rip_jobs::upload_guard(&directory).map_err(internal)?;
    let result = async {
        stage_document(&directory, &mut multipart).await?;
        rip_jobs::publish_bound(
            &app.repo,
            &directory,
            id,
            manifest,
            Some((preflight_id, expected)),
        )
        .await
        .map_err(|e| match e {
            SubmitError::Invalid(code) => bad(code),
            SubmitError::Storage(e) => e.into(),
            SubmitError::Io(e) => internal(e),
        })
    }
    .await;
    match result {
        Ok(job) => Ok((
            StatusCode::CREATED,
            Json(
                json!({"job":job,"preflightId":preflight_id,"replayed":false,"receiptUrl":format!("/api/v1/jobs/{}/receipt",job.id)}),
            ),
        )),
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&directory).await;
            if e.0 == StatusCode::CONFLICT
                && let Some(winner) = app.repo.committed_job(preflight_id).await?
            {
                return Ok((
                    StatusCode::OK,
                    Json(
                        json!({"job":app.repo.get(winner).await?,"preflightId":preflight_id,"replayed":true,"receiptUrl":format!("/api/v1/jobs/{winner}/receipt")}),
                    ),
                ));
            }
            Err(e)
        }
    }
}

pub async fn receipt(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(app.repo.get_print_receipt(id).await?))
}

pub async fn result_receipt(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(app.repo.get_result_receipt(id).await?))
}

pub async fn artifacts(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "artifacts:read")?;
    Ok(Json(app.repo.artifacts(id).await?))
}

pub(crate) async fn stage_document(
    directory: &std::path::Path,
    multipart: &mut Multipart,
) -> Result<(), ApiError> {
    let mut received = false;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| bad("invalid_multipart"))?
    {
        if field.name() != Some("document") || received {
            return Err(bad("unexpected_multipart_field"));
        }
        let mut file = tokio::fs::File::create(directory.join("source.part"))
            .await
            .map_err(internal)?;
        let mut size = 0;
        while let Some(chunk) = field.chunk().await.map_err(|_| bad("invalid_multipart"))? {
            size += chunk.len();
            if size > MAX_DOCUMENT_BYTES {
                return Err(ApiError(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "document_too_large",
                ));
            }
            file.write_all(&chunk).await.map_err(internal)?;
        }
        file.sync_all().await.map_err(internal)?;
        received = true;
    }
    if !received {
        return Err(bad("missing_document"));
    }
    Ok(())
}

#[derive(serde::Deserialize)]
pub struct ReceiptKeysQuery {
    #[serde(default)]
    after: String,
    limit: Option<i64>,
}
pub async fn receipt_keys(
    State(app): State<App>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<ReceiptKeysQuery>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit)
        || (!query.after.is_empty()
            && (query.after.len() != 71
                || !query.after.starts_with("sha256:")
                || !query.after.as_bytes()[7..]
                    .iter()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))))
    {
        return Err(ApiError(
            axum::http::StatusCode::BAD_REQUEST,
            "invalid_receipt_key_cursor",
        ));
    }
    Ok(Json(app.repo.receipt_keys(&query.after, limit).await?))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptKeyRevocation {
    reason: String,
}
pub async fn revoke_receipt_key(
    State(app): State<App>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(req): Json<ReceiptKeyRevocation>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "admin")?;
    if !rip_storage::receipts::valid_key_id(&key_id)
        || !matches!(
            req.reason.as_str(),
            "compromised" | "retired" | "operator-request"
        )
    {
        return Err(ApiError(
            axum::http::StatusCode::BAD_REQUEST,
            "invalid_receipt_key_revocation",
        ));
    }
    let actor = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| app.access(t))
        .map(|(id, _)| id)
        .ok_or(ApiError(
            axum::http::StatusCode::UNAUTHORIZED,
            "unauthorized",
        ))?;
    Ok(Json(
        app.repo
            .revoke_receipt_key(&key_id, &req.reason, &actor)
            .await?,
    ))
}
