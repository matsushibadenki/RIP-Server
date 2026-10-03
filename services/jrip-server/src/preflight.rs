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
// Experimental profile: compact serde_json Value with lexicographically sorted object keys.
// This is not RFC 8785 and is explicitly scoped to this provider's ticket vocabulary.
fn digest(value: &Value) -> Result<String, ApiError> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(internal)?)
    ))
}
pub async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    Json(intent): Json<rip_acx::intent::PrintIntent>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth(&app, &headers)?;
    let resolution = rip_acx::intent::resolve(intent);
    if resolution["executable"] != true {
        return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(resolution)));
    }
    let id = Uuid::new_v4();
    let issued = now();
    let ticket = &resolution["ticket"];
    let mut payload = json!({
        "acx":"0.1", "profile":"org.jrip.settings-preflight-v1",
        "preflightId":id,"capability":rip_acx::RIP_CAPABILITY_ID,
        "issuedAt":issued,"expiresAt":issued+300,
        "ticket":ticket,"requestHash":digest(ticket)?,
        "digestProfile":"org.jrip.serde-json-v1",
        "adjustments":resolution["adjustments"],
        "effects":["compute","persistent-storage","raster-output"],
        "authorizationGranted":false,"documentVerified":false,
        "commitAvailable":true,
        "messages":resolution["messages"]
    });
    let hash = digest(&payload)?;
    payload["preflightDigest"] = json!(hash);
    app.repo.save_preflight(id, &payload, issued + 300).await?;
    Ok((StatusCode::CREATED, Json(payload)))
}
pub async fn get(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    auth(&app, &headers)?;
    let payload = app.repo.get_preflight(id).await?;
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
    auth(&app, &headers)?;
    let payload = app.repo.get_preflight(preflight_id).await?;
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
        || supplied != digest(&unsigned)?
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
    let result = async {
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
    auth(&app, &headers)?;
    Ok(Json(app.repo.get_print_receipt(id).await?))
}

pub async fn result_receipt(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    auth(&app, &headers)?;
    Ok(Json(app.repo.get_result_receipt(id).await?))
}

pub async fn artifacts(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    auth(&app, &headers)?;
    Ok(Json(app.repo.artifacts(id).await?))
}
