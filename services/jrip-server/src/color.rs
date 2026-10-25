//! API performs bounded Rust checks; only the trusted worker invokes Docker/LCMS.
use super::*;
use rip_color::{Policy, registry::Registry};
use serde_json::{Value, json};
pub(crate) mod policies;
pub mod sandbox;
pub(crate) async fn capabilities(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    use sha2::{Digest, Sha256};
    authorize(&app, &headers, "jobs:read")?;
    let model = json!({"schemaVersion":1,"id":"org.jrip.color.icc-transform","inputMime":["image/tiff"],"outputMime":["image/tiff"],"bitDepths":[8],"colorSpaces":["rgb","cmyk","gray"],"intents":["perceptual","relative_colorimetric","absolute_colorimetric","saturation"],"blackPointCompensation":true,"blackPreservation":["none","k_only","k_plane"],"constraints":{"blackPreservation":"CMYK to CMYK; perceptual, relative_colorimetric or saturation","deviceLink":"explicit input hash, no output hash, header intent must match, no additional BPC or black preservation","tac":"CMYK output only; 100-400 percent"},"limits":{"profileBytes":4194304,"pageSampleBytes":67108864,"pages":100,"artifactBytes":1073741824,"dpiMin":72,"dpiMax":2400},"policyRegistry":{"profile":"org.jrip.color-policy-v1","digestProfile":"org.jrip.jcs-safe-v1","endpoint":"/api/v1/color/policies","submitTemplate":"/api/v1/jobs/{id}/color-attempts/from-policy","immutable":true},"physicalCompletion":false,"workerAvailability":"not-probed","labels":{"en":"Explicit ICC color processing","ja":"明示的ICC色処理","zh-CN":"显式ICC颜色处理"}});
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&model).map_err(internal)?)
    );
    Ok(Json(
        json!({"capability":model,"capabilityRevision":digest,"observedAt":rip_storage::now(),"expiresAt":rip_storage::now()+30}),
    ))
}

pub async fn tick(app: &App) -> Result<bool, Box<dyn std::error::Error>> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(app.root.join("color-worker.lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(false),
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    for id in app.repo.pending_color_cleanup().await? {
        sandbox::cleanup_attempt(id)
            .await
            .map_err(std::io::Error::other)?;
        app.repo.finish_color_cleanup(id).await?;
    }
    for id in app.repo.expired_colors().await? {
        sandbox::cleanup_attempt(id)
            .await
            .map_err(std::io::Error::other)?;
        let value = app.repo.color_attempt(id).await?;
        app.repo
            .update_color(
                id,
                value["revision"].as_i64().ok_or("invalid color revision")?,
                "FAILED",
                value["completedPages"].as_u64().unwrap_or(0) as u32,
                None,
                Some("COLOR_WORKER_LEASE_EXPIRED"),
            )
            .await?;
    }
    let Some(id) = app.repo.next_color().await? else {
        return Ok(false);
    };
    let mut attempt = app
        .repo
        .update_color(id, 0, "RUNNING", 0, None, None)
        .await?;
    match process(app, id, &mut attempt).await {
        Ok(result) => {
            app.repo
                .update_color(
                    id,
                    attempt["revision"]
                        .as_i64()
                        .ok_or("invalid color revision")?,
                    "COMPLETED",
                    attempt["completedPages"].as_u64().unwrap_or(0) as u32,
                    Some(result),
                    None,
                )
                .await?;
        }
        Err(error) => {
            tracing::error!(%id,%error,"color attempt failed");
            app.repo
                .update_color(
                    id,
                    attempt["revision"]
                        .as_i64()
                        .ok_or("invalid color revision")?,
                    "FAILED",
                    attempt["completedPages"].as_u64().unwrap_or(0) as u32,
                    None,
                    Some(error.code()),
                )
                .await?;
        }
    }
    Ok(true)
}
enum ProcessError {
    Api(ApiError),
    Worker(String),
    Other(Box<dyn std::error::Error>),
}
impl ProcessError {
    fn code(&self) -> &'static str {
        match self {
            Self::Api(ApiError(_, "artifact_integrity_mismatch")) => {
                "SOURCE_ARTIFACT_INTEGRITY_MISMATCH"
            }
            Self::Worker(code) if code == "color_worker_timeout" => "COLOR_WORKER_TIMEOUT",
            Self::Worker(code)
                if matches!(
                    code.as_str(),
                    "color_cleanup_failed" | "color_cleanup_timeout"
                ) =>
            {
                "COLOR_CLEANUP_FAILED"
            }
            Self::Worker(_) => "COLOR_TRANSFORM_FAILED",
            _ => "COLOR_ARTIFACT_FAILED",
        }
    }
}
impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api(e) => write!(f, "{}", e.1),
            Self::Worker(e) => write!(f, "{e}"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}
impl<E: std::error::Error + 'static> From<E> for ProcessError {
    fn from(e: E) -> Self {
        Self::Other(Box::new(e))
    }
}
async fn process(app: &App, id: Uuid, attempt: &mut Value) -> Result<Value, ProcessError> {
    use sha2::{Digest, Sha256};
    let ticket = attempt["ticket"].clone();
    let source = Uuid::parse_str(ticket["sourceJob"].as_str().unwrap_or(""))?;
    let input_digest = ticket["inputProfileSha256"]
        .as_str()
        .ok_or_else(|| ProcessError::Worker("invalid_color_ticket".into()))?;
    let profiles = registry(app).map_err(ProcessError::Api)?;
    let input_icc = profiles.read(input_digest)?;
    let output_icc = ticket["outputProfileSha256"]
        .as_str()
        .map(|digest| profiles.read(digest))
        .transpose()?;
    let policy: Policy = serde_json::from_value(ticket["policy"].clone())?;
    let policy_bytes = serde_json::to_vec(&policy)?;
    let policy_digest = format!("{:x}", Sha256::digest(&policy_bytes));
    let artifacts: Vec<rip_core::Artifact> =
        serde_json::from_value(ticket["sourceArtifacts"].clone())?;
    let dpi = ticket["dpi"]
        .as_u64()
        .ok_or_else(|| ProcessError::Worker("invalid_color_ticket".into()))? as u32;
    let staging_root = app.root.join("color-staging");
    std::fs::create_dir_all(&staging_root)?;
    if std::fs::canonicalize(&staging_root)? != staging_root {
        return Err(ProcessError::Worker("invalid_color_root".into()));
    }
    let work = tempfile::Builder::new()
        .prefix("color-stage-")
        .tempdir_in(&staging_root)?;
    let writer = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .read(true)
        .open(work.path().join("writer.lock"))?;
    writer.try_lock()?;
    let staging = tempfile::tempdir_in(work.path())?;
    let mut reports = Vec::new();
    let mut total = 0u64;
    for (index, artifact) in artifacts.iter().enumerate() {
        if artifact.page != index as u32 + 1 {
            return Err(ProcessError::Worker("invalid_color_ticket".into()));
        }
        let snapshot = artifacts::verified_snapshot(app, source, &artifact.name)
            .await
            .map_err(ProcessError::Api)?;
        if snapshot.2.sha256 != artifact.sha256 || snapshot.2.bytes != artifact.bytes {
            return Err(ProcessError::Worker("source_artifact_changed".into()));
        }
        let inputs = tempfile::tempdir_in(work.path())?;
        std::fs::copy(snapshot.1.path(), inputs.path().join("source.tiff"))?;
        std::fs::write(inputs.path().join("input.icc"), &input_icc)?;
        if let Some(bytes) = &output_icc {
            std::fs::write(inputs.path().join("output.icc"), bytes)?;
        }
        std::fs::write(inputs.path().join("policy.json"), &policy_bytes)?;
        let output = tempfile::tempdir_in(work.path())?;
        let converted = sandbox::convert(inputs.path(), output.path(), id, dpi)
            .await
            .map_err(ProcessError::Worker)?;
        let source_metadata =
            rip_color::raster::describe_tiff(std::fs::File::open(snapshot.1.path())?)?;
        let output_metadata =
            rip_color::raster::describe_tiff(std::fs::File::open(&converted.raster)?)?;
        if source_metadata.width != output_metadata.width
            || source_metadata.height != output_metadata.height
            || output_metadata.color_space != policy.output_color
            || output_metadata.profile_sha256.as_deref() != ticket["outputProfileSha256"].as_str()
            || converted.report["pixels"].as_u64()
                != Some(u64::from(output_metadata.width) * u64::from(output_metadata.height))
        {
            return Err(ProcessError::Worker("color_geometry_mismatch".into()));
        }
        if converted.report["schemaVersion"] != 1
            || converted.report["inputProfileSha256"] != input_digest
            || converted.report["outputProfileSha256"] != ticket["outputProfileSha256"]
            || converted.report["policySha256"] != policy_digest
        {
            return Err(ProcessError::Worker("color_report_mismatch".into()));
        }
        let page = staging.path().join(&artifact.name);
        std::fs::rename(&converted.raster, &page)?;
        total = total
            .checked_add(std::fs::metadata(&page)?.len())
            .ok_or_else(|| ProcessError::Worker("color_output_limit".into()))?;
        if total > 1024 * 1024 * 1024 {
            return Err(ProcessError::Worker("color_output_limit".into()));
        }
        let source_profile_evidence = if source_metadata.profile_sha256.is_some() {
            "embedded-match"
        } else {
            "explicit-unembedded"
        };
        let warnings = if source_metadata.profile_sha256.is_none() {
            vec![
                json!({"code":"SOURCE_PROFILE_UNEMBEDDED","messages":{"en":"Source raster has no embedded ICC; the explicit input profile is an operator assumption","ja":"原本ラスターにICCがなく、指定入力profileは操作者の判断です","zh-CN":"原始光栅没有内嵌ICC，指定输入profile属于操作者判断"}}),
            ]
        } else {
            vec![]
        };
        reports.push(json!({"page":artifact.page,"sourceArtifactSha256":artifact.sha256,"sourceProfileEvidence":source_profile_evidence,"warnings":warnings,"metadata":output_metadata,"colorReport":converted.report}));
        *attempt = app
            .repo
            .update_color(
                id,
                attempt["revision"].as_i64().unwrap_or(-1),
                "RUNNING",
                index as u32 + 1,
                None,
                None,
            )
            .await?;
    }
    let paths: Vec<_> = artifacts
        .iter()
        .map(|a| staging.path().join(&a.name))
        .collect();
    let output_artifacts = artifacts::hash_pages(&paths).await?;
    for (report, artifact) in reports.iter_mut().zip(&output_artifacts) {
        report["outputArtifact"] = serde_json::to_value(artifact)?;
    }
    let mut result = json!({"schemaVersion":1,"kind":"ColorManagedArtifact","sourceJob":source,"attemptId":id,"policySha256":policy_digest,"inputProfileSha256":input_digest,"outputProfileSha256":ticket["outputProfileSha256"],"dpi":dpi,"colorSpace":policy.output_color,"pageCount":output_artifacts.len(),"artifacts":output_artifacts,"reports":reports});
    if let Some(reference) = ticket.get("policyReference") {
        result["policyReference"] = reference.clone();
    }
    std::fs::write(
        staging.path().join("color-report.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    let parent = app.root.join("jobs").join(source.to_string()).join("color");
    std::fs::create_dir_all(&parent)?;
    if std::fs::canonicalize(&parent)? != parent {
        return Err(ProcessError::Worker("invalid_color_root".into()));
    }
    let destination = parent.join(id.to_string());
    if destination.exists() {
        return Err(ProcessError::Worker("color_output_exists".into()));
    }
    std::fs::rename(staging.path(), &destination)?;
    Ok(result)
}

fn registry(app: &App) -> Result<Registry, ApiError> {
    Registry::open(app.root.join("color-profiles")).map_err(internal)
}
pub(crate) async fn register(
    State(app): State<App>,
    headers: HeaderMap,
    bytes: axum::body::Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&app, &headers, "settings:write")?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(ApiError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "invalid_icc_profile",
        ));
    }
    let root = app.root.clone();
    let record = tokio::task::spawn_blocking(move || {
        Registry::open(root.join("color-profiles"))?.register(&bytes)
    })
    .await
    .map_err(internal)?
    .map_err(|_| bad("invalid_icc_profile"))?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(record).map_err(internal)?),
    ))
}
pub(crate) async fn describe(
    State(app): State<App>,
    headers: HeaderMap,
    Path(digest): Path<String>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    let registry = registry(&app)?;
    let record = tokio::task::spawn_blocking(move || registry.describe(&digest))
        .await
        .map_err(internal)?
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "not_found"))?;
    Ok(Json(serde_json::to_value(record).map_err(internal)?))
}
pub(crate) fn validate_settings(
    app: &App,
    policy: &Policy,
    input_digest: &str,
    output_digest: Option<&str>,
) -> Result<(), ApiError> {
    rip_color::validate_policy(policy).map_err(|_| bad("unsupported_color_policy"))?;
    let input = registry(app)?
        .describe(input_digest)
        .map_err(|_| bad("invalid_icc_profile"))?;
    let expected = match policy.input_color {
        rip_color::Color::Rgb => "RGB",
        rip_color::Color::Cmyk => "CMYK",
        rip_color::Color::Gray => "GRAY",
    };
    if input.color_space != expected || input.device_link != output_digest.is_none() {
        return Err(bad("unsupported_color_policy"));
    }
    if let Some(digest) = output_digest {
        let output = registry(app)?
            .describe(digest)
            .map_err(|_| bad("invalid_icc_profile"))?;
        let expected = match policy.output_color {
            rip_color::Color::Rgb => "RGB",
            rip_color::Color::Cmyk => "CMYK",
            rip_color::Color::Gray => "GRAY",
        };
        if output.device_link || output.color_space != expected {
            return Err(bad("unsupported_color_policy"));
        }
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub request_id: Uuid,
    pub expected_revision: i64,
    pub input_profile_sha256: String,
    pub output_profile_sha256: Option<String>,
    pub policy: Policy,
}
pub(crate) async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<Request>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    create_inner(app, headers, id, req, None).await
}
pub(crate) async fn create_inner(
    app: App,
    headers: HeaderMap,
    id: Uuid,
    req: Request,
    policy_reference: Option<String>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&app, &headers, "jobs:submit")?;
    authorize(&app, &headers, "artifacts:read")?;
    validate_settings(
        &app,
        &req.policy,
        &req.input_profile_sha256,
        req.output_profile_sha256.as_deref(),
    )?;
    let job = app.repo.get(id).await?;
    if job.state != JobState::Completed || job.revision != req.expected_revision {
        return Err(ApiError(StatusCode::CONFLICT, "revision_conflict"));
    }
    let source_color = match job.manifest.color_mode {
        rip_core::ColorMode::Rgb => rip_color::Color::Rgb,
        rip_core::ColorMode::Cmyk => rip_color::Color::Cmyk,
        rip_core::ColorMode::Gray => rip_color::Color::Gray,
    };
    if req.policy.input_color != source_color {
        return Err(bad("unsupported_color_policy"));
    }
    let pages: Vec<rip_core::Artifact> =
        serde_json::from_value(app.repo.artifacts(id).await?).map_err(internal)?;
    if pages.is_empty()
        || pages.len() > 100
        || pages.iter().any(|p| p.bytes > 64 * 1024 * 1024)
        || pages
            .iter()
            .try_fold(0u64, |sum, p| sum.checked_add(p.bytes))
            .is_none_or(|sum| sum > 1024 * 1024 * 1024)
    {
        return Err(bad("invalid_artifacts"));
    }
    let mut ticket = json!({"sourceJob":id,"sourceRevision":req.expected_revision,"inputProfileSha256":req.input_profile_sha256,"outputProfileSha256":req.output_profile_sha256,"policy":req.policy,"dpi":job.manifest.dpi,"sourceArtifacts":pages});
    if let Some(reference) = policy_reference {
        ticket["policyReference"] = json!(reference);
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(
            app.repo
                .create_color(req.request_id, id, req.expected_revision, ticket)
                .await?,
        ),
    ))
}
pub(crate) async fn detail(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(app.repo.color_attempt(id).await?))
}
pub(crate) async fn verified_page(
    app: &App,
    id: Uuid,
    name: &str,
) -> Result<artifacts::Snapshot, ApiError> {
    let attempt = app.repo.color_attempt(id).await?;
    if attempt["state"] != "COMPLETED" {
        return Err(ApiError(StatusCode::CONFLICT, "state_conflict"));
    }
    let pages: Vec<rip_core::Artifact> =
        serde_json::from_value(attempt["result"]["artifacts"].clone()).map_err(internal)?;
    let artifact = pages
        .into_iter()
        .find(|artifact| artifact.name == name)
        .ok_or(ApiError(StatusCode::NOT_FOUND, "not_found"))?;
    let source = Uuid::parse_str(attempt["sourceJob"].as_str().unwrap_or("")).map_err(internal)?;
    let path = app
        .root
        .join("jobs")
        .join(source.to_string())
        .join("color")
        .join(id.to_string())
        .join(name);
    artifacts::snapshot_file(app, path, artifact).await
}
pub(crate) async fn download(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, name)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, "artifacts:read")?;
    let snapshot = verified_page(&app, id, &name).await?;
    artifacts::snapshot_response(headers, snapshot).await
}
pub(crate) async fn events(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(cursor): Query<EventCursor>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(
        app.repo
            .color_events(id, cursor.after, cursor.limit)
            .await?,
    ))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Revision {
    pub expected_revision: i64,
}
pub(crate) async fn cancel(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<Revision>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:control")?;
    Ok(Json(
        app.repo
            .update_color(
                id,
                req.expected_revision,
                "CANCELLED",
                0,
                None,
                Some("CANCELED_BY_USER"),
            )
            .await?,
    ))
}
