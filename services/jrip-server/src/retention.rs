use super::*;
use serde_json::Value;
pub(crate) async fn get(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(app.repo.artifact_retention_policy(id).await?))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Extension {
    expected_revision: i64,
    retain_until: i64,
    reason: String,
}
pub(crate) async fn extend(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<Extension>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "settings:write")?;
    let now = rip_storage::now();
    if req.expected_revision < 0
        || req.retain_until <= now
        || req.retain_until > now + 3650 * 86400
        || req.reason.trim().is_empty()
        || req.reason.len() > 1024
    {
        return Err(bad("invalid_artifact_retention"));
    }
    let actor = pipeline::actor(&app, &headers)?;
    Ok(Json(
        app.repo
            .extend_artifact_retention(
                id,
                req.expected_revision,
                req.retain_until,
                &actor,
                &req.reason,
            )
            .await?,
    ))
}
