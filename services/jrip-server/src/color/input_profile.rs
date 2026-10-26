//! Read-only evidence. Never guesses a default or registers untrusted embedded ICC.
use super::*;
pub(crate) async fn inspect(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    authorize(&app, &headers, "artifacts:read")?;
    let job = app.repo.get(id).await?;
    if job.state != JobState::Completed {
        return Err(ApiError(StatusCode::CONFLICT, "revision_conflict"));
    }
    let pages: Vec<rip_core::Artifact> =
        serde_json::from_value(app.repo.artifacts(id).await?).map_err(internal)?;
    if pages.is_empty()
        || pages.len() > 32
        || pages.iter().any(|p| p.bytes > 64 * 1024 * 1024)
        || pages
            .iter()
            .try_fold(0u64, |sum, p| sum.checked_add(p.bytes))
            .is_none_or(|n| n > 256 * 1024 * 1024)
    {
        return Err(bad("color_profile_inspection_limit"));
    }
    let registry = registry(&app)?;
    let mut observations = Vec::new();
    let mut candidates = std::collections::BTreeSet::new();
    let mut all_registered = true;
    for (index, page) in pages.iter().enumerate() {
        if page.page != index as u32 + 1 {
            return Err(bad("invalid_artifacts"));
        }
        let snapshot = artifacts::verified_snapshot(&app, id, &page.name).await?;
        if snapshot.2.sha256 != page.sha256 || snapshot.2.bytes != page.bytes {
            return Err(ApiError(StatusCode::CONFLICT, "revision_conflict"));
        }
        let file = std::fs::File::open(snapshot.1.path()).map_err(internal)?;
        let metadata = tokio::task::spawn_blocking(move || rip_color::raster::describe_tiff(file))
            .await
            .map_err(internal)?
            .map_err(|_| bad("invalid_artifacts"))?;
        let registered = metadata.profile_state == "VALIDATED_ENVELOPE"
            && metadata
                .profile_sha256
                .as_ref()
                .is_some_and(|hash| registry.describe(hash).is_ok());
        let manifest_matches = matches!(
            (job.manifest.color_mode, metadata.color_space),
            (rip_core::ColorMode::Rgb, rip_color::Color::Rgb)
                | (rip_core::ColorMode::Cmyk, rip_color::Color::Cmyk)
                | (rip_core::ColorMode::Gray, rip_color::Color::Gray)
        );
        all_registered &= registered && manifest_matches;
        if metadata.profile_state == "VALIDATED_ENVELOPE" {
            candidates.insert(
                metadata
                    .profile_sha256
                    .clone()
                    .ok_or(bad("invalid_icc_profile"))?,
            );
        }
        observations.push(json!({"page":page.page,"artifactSha256":page.sha256,"metadata":metadata,"registered":registered,"manifestColorMatches":manifest_matches}));
    }
    let current = app.repo.get(id).await?;
    if current.revision != job.revision
        || app.repo.artifacts(id).await? != serde_json::to_value(&pages).map_err(internal)?
    {
        return Err(ApiError(StatusCode::CONFLICT, "revision_conflict"));
    }
    let mixed = candidates.len() > 1;
    let available = all_registered && candidates.len() == 1;
    let candidate = if available {
        candidates.first().cloned()
    } else {
        None
    };
    let definition = json!({"profile":"org.jrip.color-input-profile-v1","jobId":id,"sourceRevision":job.revision,"pages":observations});
    let digest = rip_digest::digest(&definition).map_err(internal)?;
    Ok(Json(
        json!({"evidence":definition,"digestProfile":rip_digest::PROFILE,"evidenceDigest":digest,
        "status":if available {"EXPLICIT_PROFILE_AVAILABLE"}else{"EXPLICIT_SELECTION_REQUIRED"},"candidateProfileSha256":candidate,"mixedProfiles":mixed,
        "automaticSelection":false,"validationMeaning":"bounded-icc-envelope-not-colorimetric-validation",
        "messages":{"en":"Review embedded ICC evidence and explicitly select an input profile","ja":"埋め込みICCの証跡を確認し、入力profileを明示指定してください","zh-CN":"请检查嵌入ICC证据并显式选择输入profile"}}),
    ))
}
