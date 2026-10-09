use super::*;
use sha2::{Digest, Sha256};
fn profile_hash(app: &App, id: &str) -> Result<String, ApiError> {
    let printer = app
        .printers
        .iter()
        .find(|printer| printer.id == id)
        .ok_or(bad("invalid_approval"))?;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(printer).map_err(internal)?)
    ))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    identity: String,
    job_id: Uuid,
    expected_revision: i64,
    request_id: Uuid,
    printer_id: String,
    media: String,
    reason: String,
    color_attempt_id: Option<Uuid>,
}
fn actor(app: &App, headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .and_then(|token| app.access(token))
        .map(|(identity, _)| identity)
        .ok_or(ApiError(StatusCode::UNAUTHORIZED, "unauthorized"))
}
pub(crate) async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<Request>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    authorize(&app, &headers, "admin")?;
    if req.reason.trim().is_empty()
        || req.reason.len() > 512
        || req.reason.chars().any(char::is_control)
        || !app
            .credentials
            .iter()
            .any(|credential| credential.identity == req.identity)
        || !app.printers.iter().any(|printer| {
            printer.id == req.printer_id
                && printer.media.iter().any(|media| media.name == req.media)
        })
    {
        return Err(bad("invalid_approval"));
    }
    let job = app.repo.get(req.job_id).await?;
    if job.revision != req.expected_revision || job.state != JobState::Completed {
        return Err(ApiError(StatusCode::CONFLICT, "state_conflict"));
    }
    if let Some(color) = req.color_attempt_id {
        let attempt = app.repo.color_attempt(color).await?;
        if attempt["state"] != "COMPLETED" || attempt["sourceJob"] != req.job_id.to_string() {
            return Err(bad("invalid_approval"));
        }
    }
    let id = Uuid::new_v4();
    let expires = rip_storage::now() + 300;
    let payload = serde_json::json!({"id":id,"identity":req.identity,"jobId":req.job_id,
        "expectedRevision":req.expected_revision,"requestId":req.request_id,"printerId":req.printer_id,
        "profileHash":profile_hash(&app,&req.printer_id)?,"media":req.media,"colorAttemptId":req.color_attempt_id,"expiresAt":expires,"issuer":actor(&app,&headers)?,"reason":req.reason});
    app.repo.save_approval(id, &payload, expires).await?;
    Ok((StatusCode::CREATED, Json(payload)))
}
pub(crate) async fn revoke(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    authorize(&app, &headers, "admin")?;
    app.repo.revoke_approval(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(crate) async fn check_delivery(
    app: &App,
    headers: &HeaderMap,
    job: Uuid,
    revision: i64,
    request: Uuid,
    printer: &str,
    media_color: (&str, Option<Uuid>),
) -> Result<Option<Uuid>, ApiError> {
    if !app.require_print_approval {
        return Ok(None);
    }
    let id = headers
        .get("x-jrip-approval-id")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or(ApiError(StatusCode::FORBIDDEN, "approval_required"))?;
    let approval = app
        .repo
        .approval(id)
        .await
        .map_err(|_| ApiError(StatusCode::FORBIDDEN, "approval_required"))?;
    if approval["identity"] != actor(app, headers)?
        || approval["jobId"] != job.to_string()
        || approval["expectedRevision"] != revision
        || approval["requestId"] != request.to_string()
        || approval["printerId"] != printer
        || approval["media"] != media_color.0
        || approval["colorAttemptId"] != serde_json::json!(media_color.1)
        || approval["profileHash"] != profile_hash(app, printer)?
    {
        return Err(ApiError(StatusCode::FORBIDDEN, "approval_required"));
    }
    Ok(Some(id))
}
