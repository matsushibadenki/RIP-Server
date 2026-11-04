use crate::{ApiError, App, artifacts::verified_snapshot, authorize, internal};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use uuid::Uuid;

static DECODERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

pub async fn get(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, name)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, "artifacts:read")?;
    let decoder_permit = DECODERS
        .try_acquire()
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "artifact_budget_exhausted"))?;
    let (file, guard, artifact, budget) = verified_snapshot(&app, id, &name).await?;
    response((file, guard, artifact, budget), decoder_permit, None).await
}

pub async fn color(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, name)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, "artifacts:read")?;
    let decoder_permit = DECODERS
        .try_acquire()
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "artifact_budget_exhausted"))?;
    let snapshot = crate::color::verified_page(&app, id, &name).await?;
    let attempt = app.repo.color_attempt(id).await?;
    let profile = attempt["ticket"]["outputProfileSha256"]
        .as_str()
        .ok_or(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "preview_unsupported",
        ))?
        .to_owned();
    let color: rip_color::Color =
        serde_json::from_value(attempt["ticket"]["policy"]["outputColor"].clone())
            .map_err(internal)?;
    let mut response = response(snapshot, decoder_permit, Some((color, profile.clone()))).await?;
    response.headers_mut().insert(
        "x-jrip-color-attempt-id",
        id.to_string().parse().map_err(internal)?,
    );
    response.headers_mut().insert(
        "x-jrip-input-profile-sha256",
        profile.parse().map_err(internal)?,
    );
    Ok(response)
}

async fn response(
    snapshot: crate::artifacts::Snapshot,
    decoder_permit: tokio::sync::SemaphorePermit<'static>,
    expected: Option<(rip_color::Color, String)>,
) -> Result<Response, ApiError> {
    let (file, guard, artifact, budget) = snapshot;
    let mut file = file.into_std().await;
    // Keep the verified snapshot and quota alive for the entire blocking decode.
    let bytes = tokio::task::spawn_blocking(move || {
        let _resources = (guard, budget, decoder_permit);
        if let Some((color, profile)) = expected {
            let metadata = rip_color::raster::describe_tiff(&mut file)
                .map_err(|_| rip_raster::Error::Unsupported)?;
            if metadata.color_space != color
                || metadata.profile_sha256.as_deref() != Some(&profile)
                || metadata.profile_state != "VALIDATED_ENVELOPE"
            {
                return Err(rip_raster::Error::Unsupported);
            }
            std::io::Seek::rewind(&mut file)?;
        }
        rip_raster::preview::bmp(file)
    })
    .await
    .map_err(internal)?
    .map_err(|_| ApiError(StatusCode::UNPROCESSABLE_ENTITY, "preview_unsupported"))?;
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert("x-jrip-preview-color-space", "srgb".parse().unwrap());
    headers.insert("content-type", "image/bmp".parse().unwrap());
    headers.insert("cache-control", "private, no-store".parse().unwrap());
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert(
        "x-jrip-source-sha256",
        artifact.sha256.parse().map_err(internal)?,
    );
    Ok(response)
}
