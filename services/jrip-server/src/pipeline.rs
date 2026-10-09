//! Orchestration that waits for RIP completion before creating child attempts. No credentials are persisted.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub request_id: Uuid,
    pub expected_revision: i64,
    pub printer_id: String,
    pub media: String,
    pub color: Option<ColorRequest>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColorRequest {
    pub input_profile_sha256: String,
    pub output_profile_sha256: Option<String>,
    pub policy: rip_color::Policy,
}
pub(crate) fn actor(app: &App, headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .and_then(|token| app.access(token))
        .map(|(actor, _)| actor)
        .ok_or(ApiError(StatusCode::UNAUTHORIZED, "unauthorized"))
}
pub(crate) fn scopes(app: &App, headers: &HeaderMap) -> Result<(), ApiError> {
    for scope in [
        "jobs:read",
        "jobs:submit",
        "artifacts:read",
        "printers:print",
    ] {
        authorize(app, headers, scope)?;
    }
    Ok(())
}
fn config_hash(printer: &delivery::Printer) -> Result<String, ApiError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(printer).map_err(internal)?)
    ))
}
pub(crate) async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    Path(job): Path<Uuid>,
    Json(req): Json<Request>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    scopes(&app, &headers)?;
    match app.repo.pipeline(req.request_id).await {
        Ok(existing) => {
            if existing["sourceJob"] != job.to_string()
                || existing["ticket"]["actor"] != actor(&app, &headers)?
                || existing["ticket"]["request"] != serde_json::to_value(&req).map_err(internal)?
            {
                return Err(ApiError(StatusCode::CONFLICT, "pipeline_request_conflict"));
            }
            return Ok((StatusCode::ACCEPTED, Json(existing)));
        }
        Err(StorageError::NotFound) => {}
        Err(e) => return Err(e.into()),
    }
    let source = app.repo.get(job).await?;
    if source.revision != req.expected_revision
        || matches!(source.state, JobState::Failed | JobState::Cancelled)
    {
        return Err(ApiError(StatusCode::CONFLICT, "revision_conflict"));
    }
    let ticket = registration_ticket(&app, &headers, &source, &req)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(
            app.repo
                .create_pipeline(req.request_id, job, req.expected_revision, ticket)
                .await?,
        ),
    ))
}
pub(crate) fn registration_ticket(
    app: &App,
    headers: &HeaderMap,
    source: &Job,
    req: &Request,
) -> Result<Value, ApiError> {
    scopes(app, headers)?;
    let printer = app
        .printers
        .iter()
        .find(|p| p.id == req.printer_id)
        .ok_or(bad("printer_not_configured"))?;
    if !printer.media.iter().any(|m| m.name == req.media) {
        return Err(bad("media_not_configured"));
    }
    if let Some(color) = &req.color {
        rip_color::validate_policy(&color.policy).map_err(|_| bad("unsupported_color_policy"))?;
        if !rip_core::print::valid_digest(&color.input_profile_sha256)
            || color
                .output_profile_sha256
                .as_ref()
                .is_some_and(|d| !rip_core::print::valid_digest(d))
        {
            return Err(bad("invalid_icc_profile"));
        }
        let target = printer
            .color_target
            .as_ref()
            .ok_or(bad("printer_color_target_required"))?;
        if target.output_profile_sha256 != color.output_profile_sha256
            || (target.output_profile_sha256.is_none()
                && target.device_link_sha256.as_deref() != Some(&color.input_profile_sha256))
        {
            return Err(bad("printer_color_profile_mismatch"));
        }
    } else if printer.color_target.is_some() {
        return Err(bad("color_management_required"));
    }
    let ticket = json!({"schemaVersion":1,"request":req,"actor":actor(app,headers)?,"printerConfigSha256":config_hash(printer)?,"sourceSha256":source.source_sha256,"sourceManifest":source.manifest});
    Ok(ticket)
}
pub(crate) async fn detail(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(app.repo.pipeline(id).await?))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resume {
    pub expected_revision: i64,
    pub approval_id: Option<Uuid>,
}
pub(crate) async fn resume(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<Resume>,
) -> Result<Json<Value>, ApiError> {
    scopes(&app, &headers)?;
    let value = app.repo.pipeline(id).await?;
    if value["ticket"]["actor"] != actor(&app, &headers)?
        || !matches!(
            value["state"].as_str(),
            Some("WAITING_APPROVAL" | "BLOCKED")
        )
    {
        return Err(ApiError(StatusCode::CONFLICT, "state_conflict"));
    }
    Ok(Json(
        app.repo
            .update_pipeline(id, req.expected_revision, "QUEUED", None, req.approval_id)
            .await?,
    ))
}
pub(crate) async fn cancel(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<color::Revision>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:control")?;
    authorize(&app, &headers, "printers:print")?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(app.root.join("pipeline-worker.lock"))
        .map_err(internal)?;
    lock.try_lock()
        .map_err(|_| ApiError(StatusCode::CONFLICT, "pipeline_busy"))?;
    let value = app.repo.pipeline(id).await?;
    if value["ticket"]["actor"] != actor(&app, &headers)?
        || value["revision"] != req.expected_revision
        || matches!(value["state"].as_str(), Some("COMPLETED" | "FAILED"))
    {
        return Err(ApiError(StatusCode::CONFLICT, "state_conflict"));
    }
    match app.repo.delivery(id).await {
        Ok(_) => {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "cancel_delivery_attempt_required",
            ));
        }
        Err(StorageError::NotFound) => {}
        Err(e) => return Err(e.into()),
    }
    match app.repo.color_attempt(id).await {
        Ok(attempt) if attempt["state"] == "QUEUED" => {
            let _ = color::cancel(
                State(app.clone()),
                headers.clone(),
                Path(id),
                Json(color::Revision {
                    expected_revision: attempt["revision"]
                        .as_i64()
                        .ok_or(bad("invalid_pipeline"))?,
                }),
            )
            .await?;
        }
        Ok(attempt) if attempt["state"] == "RUNNING" => {
            return Err(ApiError(StatusCode::CONFLICT, "color_attempt_running"));
        }
        Ok(_) | Err(StorageError::NotFound) => {}
        Err(e) => return Err(e.into()),
    }
    Ok(Json(
        app.repo
            .update_pipeline(
                id,
                req.expected_revision,
                "CANCELLED",
                Some("CANCELLED_BEFORE_DELIVERY"),
                None,
            )
            .await?,
    ))
}
fn delegated_headers(app: &App, identity: &str) -> Result<HeaderMap, ApiError> {
    let token = if app.credentials.is_empty() && identity == "legacy-local" {
        Some(app.token.as_ref())
    } else {
        app.credentials
            .iter()
            .filter(|c| c.identity == identity)
            .find(|c| {
                [
                    "jobs:read",
                    "jobs:submit",
                    "artifacts:read",
                    "printers:print",
                ]
                .iter()
                .all(|scope| c.scopes.iter().any(|s| s == scope || s == "admin"))
            })
            .map(|c| c.token.as_str())
    }
    .ok_or(ApiError(
        StatusCode::FORBIDDEN,
        "pipeline_authorization_revoked",
    ))?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        format!("Bearer {token}").parse().map_err(internal)?,
    );
    scopes(app, &headers)?;
    Ok(headers)
}
pub async fn tick(app: &App) -> Result<bool, Box<dyn std::error::Error>> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(app.root.join("pipeline-worker.lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(false),
        Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
    }
    let ids = app.repo.active_pipelines().await?;
    let worked = !ids.is_empty();
    for id in ids {
        let value = app.repo.pipeline(id).await?;
        let revision = value["revision"]
            .as_i64()
            .ok_or("invalid pipeline revision")?;
        let approval = value["approvalId"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok());
        let result = advance(app, id, &value).await;
        let (state, reason) = match result {
            Ok(state) => (state, None),
            Err(ApiError(status, code)) if status == StatusCode::FORBIDDEN => (
                if code == "approval_required" {
                    "WAITING_APPROVAL"
                } else {
                    "BLOCKED"
                },
                Some(code),
            ),
            Err(ApiError(status, code)) if status == StatusCode::INTERNAL_SERVER_ERROR => {
                ("BLOCKED", Some(code))
            }
            Err(ApiError(_, code)) => ("FAILED", Some(code)),
        };
        if value["state"] != state || value["reason"] != json!(reason) {
            match app
                .repo
                .update_pipeline(id, revision, state, reason, approval)
                .await
            {
                Ok(_) => {}
                Err(StorageError::Conflict) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(worked)
}
async fn advance(app: &App, id: Uuid, value: &Value) -> Result<&'static str, ApiError> {
    let mut request: Request =
        serde_json::from_value(value["ticket"]["request"].clone()).map_err(internal)?;
    let job = Uuid::parse_str(value["sourceJob"].as_str().ok_or(bad("invalid_pipeline"))?)
        .map_err(internal)?;
    let mut headers = delegated_headers(
        app,
        value["ticket"]["actor"]
            .as_str()
            .ok_or(bad("invalid_pipeline"))?,
    )?;
    if let Ok(delivery) = app.repo.delivery(id).await {
        if delivery.ticket.source_job != job
            || delivery.ticket.printer_id != request.printer_id
            || delivery.ticket.media != request.media
            || delivery.ticket.color_attempt_id != request.color.as_ref().map(|_| id)
        {
            return Err(bad("pipeline_attempt_conflict"));
        }
        use rip_storage::delivery::DeliveryState::*;
        return Ok(match delivery.state {
            Completed => "COMPLETED",
            Unknown => "UNKNOWN",
            Cancelled | Aborted | Rejected => "FAILED",
            _ => "DELIVERY",
        });
    }
    let source = app.repo.get(job).await?;
    if !value["ticket"]["sourceSha256"].is_null() {
        if value["ticket"]["sourceSha256"] != source.source_sha256
            || value["ticket"]["sourceManifest"]
                != serde_json::to_value(&source.manifest).map_err(internal)?
        {
            return Err(bad("pipeline_source_changed"));
        }
        match source.state {
            JobState::Failed => return Err(bad("pipeline_rip_failed")),
            JobState::Cancelled => return Err(bad("pipeline_rip_cancelled")),
            JobState::Completed => request.expected_revision = source.revision,
            _ => return Ok("RIP"),
        }
    }
    let printer = app
        .printers
        .iter()
        .find(|p| p.id == request.printer_id)
        .ok_or(bad("printer_not_configured"))?;
    if value["ticket"]["printerConfigSha256"] != config_hash(printer)? {
        return Err(ApiError(StatusCode::FORBIDDEN, "pipeline_printer_changed"));
    }
    if let Some(color) = request.color {
        match app.repo.color_attempt(id).await {
            Ok(attempt) => {
                if attempt["sourceJob"] != job.to_string()
                    || attempt["ticket"]["sourceRevision"] != request.expected_revision
                    || attempt["ticket"]["inputProfileSha256"] != color.input_profile_sha256
                    || attempt["ticket"]["outputProfileSha256"]
                        != json!(color.output_profile_sha256)
                    || attempt["ticket"]["policy"]
                        != serde_json::to_value(&color.policy).map_err(internal)?
                {
                    return Err(bad("pipeline_attempt_conflict"));
                }
                match attempt["state"].as_str() {
                    Some("COMPLETED") => {}
                    Some("QUEUED" | "RUNNING") => return Ok("COLOR"),
                    _ => return Err(bad("pipeline_color_failed")),
                }
            }
            Err(StorageError::NotFound) => {
                let _ = color::create(
                    State(app.clone()),
                    headers,
                    Path(job),
                    Json(color::Request {
                        request_id: id,
                        expected_revision: request.expected_revision,
                        input_profile_sha256: color.input_profile_sha256,
                        output_profile_sha256: color.output_profile_sha256,
                        policy: color.policy,
                    }),
                )
                .await?;
                return Ok("COLOR");
            }
            Err(e) => return Err(e.into()),
        }
    }
    if let Some(approval) = value["approvalId"].as_str() {
        headers.insert("x-jrip-approval-id", approval.parse().map_err(internal)?);
    }
    let req=serde_json::from_value(json!({"requestId":id,"printerId":request.printer_id,"media":request.media,"expectedRevision":request.expected_revision,"colorAttemptId":if value["ticket"]["request"]["color"].is_null(){None}else{Some(id)}})).map_err(internal)?;
    let _ = delivery::prepare(State(app.clone()), headers, Path(job), Json(req)).await?;
    Ok("DELIVERY")
}

pub(crate) async fn events(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(cursor): Query<EventCursor>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    if cursor.after < 0 || !(1..=100).contains(&cursor.limit) {
        return Err(bad("invalid_event_cursor"));
    }
    Ok(Json(
        app.repo
            .pipeline_events(id, cursor.after, cursor.limit)
            .await?,
    ))
}

pub(crate) async fn unified_events(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(cursor): Query<EventCursor>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    if cursor.after < 0 || !(1..=100).contains(&cursor.limit) {
        return Err(bad("invalid_event_cursor"));
    }
    Ok(Json(
        app.repo
            .unified_pipeline_events(id, cursor.after, cursor.limit)
            .await?,
    ))
}
