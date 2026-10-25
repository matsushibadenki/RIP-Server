//! New policies are explicit immutable settings, never a mutable global default.
use super::*;
#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Definition {
    schema_version: u32,
    labels: Labels,
    input_profile_sha256: String,
    output_profile_sha256: Option<String>,
    policy: Policy,
}
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Labels {
    en: String,
    ja: String,
    #[serde(rename = "zh-CN")]
    zh: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    definition: Definition,
    reason: String,
}
pub(crate) async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<Save>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&app, &headers, "settings:write")?;
    let def = &req.definition;
    if def.schema_version != 1
        || [&def.labels.en, &def.labels.ja, &def.labels.zh]
            .iter()
            .any(|s| s.trim().is_empty() || s.len() > 256)
        || req.reason.trim().is_empty()
        || req.reason.len() > 1024
    {
        return Err(bad("invalid_color_policy_definition"));
    }
    validate_settings(
        &app,
        &def.policy,
        &def.input_profile_sha256,
        def.output_profile_sha256.as_deref(),
    )?;
    let actor = crate::pipeline::actor(&app, &headers)?;
    let payload = app
        .repo
        .save_color_policy(
            serde_json::to_value(def).map_err(internal)?,
            &actor,
            &req.reason,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(payload)))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Page {
    #[serde(default)]
    after: String,
    #[serde(default = "default_limit")]
    limit: i64,
}
fn default_limit() -> i64 {
    50
}
pub(crate) async fn list(
    State(app): State<App>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    if (!page.after.is_empty() && !rip_storage::color_policy::valid_digest(&page.after))
        || !(1..=100).contains(&page.limit)
    {
        return Err(bad("invalid_color_policy_page"));
    }
    Ok(Json(
        app.repo.color_policies(&page.after, page.limit).await?,
    ))
}
pub(crate) async fn get(
    State(app): State<App>,
    headers: HeaderMap,
    Path(digest): Path<String>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(app.repo.color_policy(&digest).await?))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Submit {
    request_id: Uuid,
    expected_revision: i64,
    policy_digest: String,
}
pub(crate) async fn submit(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<Submit>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&app, &headers, "jobs:submit")?;
    authorize(&app, &headers, "artifacts:read")?;
    let stored = app.repo.color_policy(&req.policy_digest).await?;
    let def: Definition = serde_json::from_value(stored["definition"].clone()).map_err(internal)?;
    create_inner(
        app,
        headers,
        id,
        super::Request {
            request_id: req.request_id,
            expected_revision: req.expected_revision,
            input_profile_sha256: def.input_profile_sha256,
            output_profile_sha256: def.output_profile_sha256,
            policy: def.policy,
        },
        Some(req.policy_digest),
    )
    .await
}
