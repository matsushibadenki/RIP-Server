//! Verified small artifact contracts; large samples stay in the bounded snapshot.
use crate::{ApiError, App, artifacts, authorize, internal};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use uuid::Uuid;

pub async fn raster(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, name)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    authorize(&app, &headers, "artifacts:read")?;
    let permit = crate::preview::DECODERS
        .try_acquire()
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "artifact_budget_exhausted"))?;
    let snapshot = artifacts::verified_snapshot(&app, id, &name).await?;
    let job = app.repo.get(id).await?;
    let source = json!({"jobId":id,"sourceSha256":job.source_sha256,"engine":job.selected_engine});
    let result = describe(&app, id, None, snapshot, permit, source, None).await?;
    Ok(reply(result))
}
pub async fn color(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, name)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    authorize(&app, &headers, "artifacts:read")?;
    let permit = crate::preview::DECODERS
        .try_acquire()
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "artifact_budget_exhausted"))?;
    let snapshot = crate::color::verified_page(&app, id, &name).await?;
    let attempt = app.repo.color_attempt(id).await?;
    let job_id = Uuid::parse_str(
        attempt["sourceJob"]
            .as_str()
            .ok_or_else(|| internal("invalid_color_attempt"))?,
    )
    .map_err(internal)?;
    let job = app.repo.get(job_id).await?;
    let source = json!({"jobId":job_id,"sourceSha256":job.source_sha256,"colorAttemptId":id,"policySha256":attempt["result"]["policySha256"],"inputProfileSha256":attempt["ticket"]["inputProfileSha256"],"outputProfileSha256":attempt["ticket"]["outputProfileSha256"],"policyReference":attempt["ticket"]["policyReference"]});
    let expected = Some((
        attempt["ticket"]["policy"]["outputColor"].clone(),
        attempt["ticket"]["outputProfileSha256"].clone(),
    ));
    Ok(reply(
        describe(&app, job_id, Some(id), snapshot, permit, source, expected).await?,
    ))
}
async fn describe(
    app: &App,
    job: Uuid,
    color: Option<Uuid>,
    snapshot: artifacts::Snapshot,
    permit: tokio::sync::SemaphorePermit<'static>,
    provenance: Value,
    expected: Option<(Value, Value)>,
) -> Result<Value, ApiError> {
    let (file, guard, artifact, budget) = snapshot;
    let file = file.into_std().await;
    let facts = tokio::task::spawn_blocking(move || {
        let _resources = (guard, budget, permit);
        rip_raster::describe::tiff(file)
    })
    .await
    .map_err(internal)?
    .map_err(|_| {
        ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "artifact_layout_unsupported",
        )
    })?;
    let facts = serde_json::to_value(facts).map_err(internal)?;
    if let Some((space, profile)) = expected
        && (facts["colorSpace"] != space
            || facts["profileSha256"] != profile
            || facts["profileState"]
                != if profile.is_null() {
                    "MISSING"
                } else {
                    "VALIDATED_ENVELOPE"
                })
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "artifact_color_contract_mismatch",
        ));
    }
    let retention = app.repo.artifact_retention_policy(job).await?;
    let base = color
        .map(|id| format!("/api/v1/color/attempts/{id}/artifacts"))
        .unwrap_or_else(|| format!("/api/v1/jobs/{job}/artifacts"));
    Ok(
        json!({"schemaVersion":1,"profile":rip_core::print::RASTER_ARTIFACT_DESCRIPTOR_PROFILE,
        "kind":if color.is_some(){"ColorManagedArtifact"}else{"RasterArtifact"},
        "contentId":format!("sha256:{}",artifact.sha256),"sha256":artifact.sha256,"bytes":artifact.bytes,
        "mediaType":"image/tiff","pageCount":1,"page":artifact.page,"sourcePage":artifact.source_page,
        "sourcePageCount":artifact.source_page_count,"raster":facts,"provenance":provenance,
        "integrityVerified":true,"reference":format!("{base}/{}",artifact.name),
        "retention":{"reference":format!("/api/v1/jobs/{job}/retention"),"profile":retention["profile"],
            "revision":retention["revision"],"retainUntil":retention["retainUntil"],"meaning":"minimum-retention-not-automatic-deletion"},
        "expiresAt":null,"authorizationGranted":false}),
    )
}

fn reply(value: Value) -> Response {
    (
        [(axum::http::header::CACHE_CONTROL, "private, no-store")],
        Json(value),
    )
        .into_response()
}
