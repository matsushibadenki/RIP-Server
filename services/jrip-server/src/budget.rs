use crate::*;
use serde_json::{Value, json};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reserve {
    request_id: Uuid,
    bytes: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Release {
    expected_bytes: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reconcile {
    expected_owner: String,
    expected_bytes: u64,
    cleanup_confirmed: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cursor {
    after: Option<Uuid>,
    #[serde(default = "limit")]
    limit: u32,
}
fn limit() -> u32 {
    50
}
pub async fn reserve(
    State(app): State<App>,
    headers: HeaderMap,
    Json(request): Json<Reserve>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "budgets:reserve")?;
    let owner = crate::pipeline::actor(&app, &headers)?;
    Ok(Json(
        app.repo
            .reserve_snapshot(request.request_id, &owner, request.bytes)
            .await?,
    ))
}
pub async fn release(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<Release>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "budgets:reserve")?;
    let owner = crate::pipeline::actor(&app, &headers)?;
    Ok(Json(
        app.repo
            .release_snapshot(id, &owner, request.expected_bytes)
            .await?,
    ))
}
pub async fn reconcile(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<Reconcile>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "admin")?;
    if !request.cleanup_confirmed {
        return Err(bad("snapshot_cleanup_required"));
    }
    let value = app
        .repo
        .release_snapshot(id, &request.expected_owner, request.expected_bytes)
        .await?;
    Ok(Json(json!({"reservation":value,"cleanupConfirmed":true})))
}
pub async fn status(
    State(app): State<App>,
    headers: HeaderMap,
    Query(cursor): Query<Cursor>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "admin")?;
    Ok(Json(
        app.repo
            .snapshot_budget_status(cursor.after, cursor.limit)
            .await?,
    ))
}
