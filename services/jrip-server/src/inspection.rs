//! HTTP stages bounded sources; only the trusted inspection worker runs Docker.
use super::*;
use serde_json::{Value, json};
fn manifest() -> Manifest {
    Manifest {
        name: "inspection".into(),
        format: rip_core::DocumentFormat::Pdf,
        dpi: 72,
        color_mode: rip_core::ColorMode::Rgb,
        priority: 50,
        engine: rip_core::EnginePreference::MuPdf,
    }
}
pub(crate) async fn events(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(cursor): Query<EventCursor>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:submit")?;
    if cursor.after < 0 || !(1..=100).contains(&cursor.limit) {
        return Err(bad("invalid_event_cursor"));
    }
    Ok(Json(
        app.repo
            .inspection_events(
                id,
                &pipeline::actor(&app, &headers)?,
                cursor.after,
                cursor.limit,
            )
            .await?,
    ))
}
fn directory_lock(path: &std::path::Path) -> Result<std::fs::File, std::io::Error> {
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.join("upload.lock"))
}
pub(crate) async fn submit(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&app, &headers, "jobs:submit")?;
    let actor = pipeline::actor(&app, &headers)?;
    let _budget =
        artifacts::reserve_shared(&app.root, rip_jobs::MAX_DOCUMENT_BYTES).map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "inspection_upload_budget_exhausted",
            )
        })?;
    let root = app.root.join("inspection-queue");
    tokio::fs::create_dir_all(&root).await.map_err(internal)?;
    if tokio::fs::canonicalize(&root).await.map_err(internal)? != root {
        return Err(bad("invalid_inspection_root"));
    }
    let directory_id = Uuid::new_v4();
    let staging = root.join(format!(".upload-{directory_id}"));
    tokio::fs::create_dir(&staging).await.map_err(internal)?;
    let lock = directory_lock(&staging).map_err(internal)?;
    lock.lock().map_err(internal)?;
    let published = root.join(directory_id.to_string());
    let mut registered = false;
    let result = async {
        preflight::stage_document(&staging, &mut multipart).await?;
        let (job, ticket) = rip_jobs::prepare_staged(&staging, directory_id, manifest(), None)
            .await
            .map_err(submit_error)?;
        tokio::fs::rename(&staging, &published)
            .await
            .map_err(internal)?;
        registered = app
            .repo
            .enqueue_inspection(
                id,
                &actor,
                &job.source_sha256,
                ticket.document.bytes.ok_or(bad("invalid_document"))?,
                directory_id,
            )
            .await?;
        app.repo
            .inspection_request(id, &actor)
            .await
            .map_err(ApiError::from)
    }
    .await;
    if !registered {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        let _ = tokio::fs::remove_dir_all(&published).await;
    }
    result.map(|value| (StatusCode::ACCEPTED, Json(value)))
}
pub(crate) async fn detail(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:submit")?;
    Ok(Json(
        app.repo
            .inspection_request(id, &pipeline::actor(&app, &headers)?)
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
    let actor = pipeline::actor(&app, &headers)?;
    let value = app.repo.inspection_request(id, &actor).await?;
    if value["state"] != "QUEUED" || value["revision"] != req.expected_revision {
        return Err(ApiError(StatusCode::CONFLICT, "inspection_not_queued"));
    }
    app.repo
        .set_inspection_state(
            id,
            req.expected_revision,
            "CANCELLED",
            Some("INSPECTION_CANCELLED_BEFORE_START"),
            None,
        )
        .await?;
    Ok(Json(app.repo.inspection_request(id, &actor).await?))
}
fn authorized(app: &App, actor: &str) -> bool {
    (app.credentials.is_empty() && actor == "legacy-local")
        || app.credentials.iter().any(|c| {
            c.identity == actor
                && c.scopes
                    .iter()
                    .any(|scope| scope == "admin" || scope == "jobs:submit")
        })
}
pub async fn tick(app: &App) -> Result<bool, Box<dyn std::error::Error>> {
    let worker = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(app.root.join("inspection-worker.lock"))?;
    match worker.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(false),
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    let work = app.repo.inspection_work().await?;
    let mut worked = false;
    for (id, actor, directory, value) in work {
        let path = app
            .root
            .join("inspection-queue")
            .join(directory.to_string());
        let state = value["state"].as_str().ok_or("invalid inspection state")?;
        let revision = value["revision"]
            .as_i64()
            .ok_or("invalid inspection revision")?;
        if state == "RUNNING" {
            // Holding the one-host worker lock proves no surviving worker owns this attempt.
            rip_interpreter::cleanup_inspection(id).await?;
            app.repo
                .set_inspection_state(
                    id,
                    revision,
                    "FAILED",
                    Some("INSPECTION_WORKER_INTERRUPTED"),
                    None,
                )
                .await?;
        } else if state == "QUEUED" {
            app.repo
                .set_inspection_state(id, revision, "RUNNING", None, None)
                .await?;
            let result = async {
                if !authorized(app, &actor) {
                    return Err("INSPECTION_AUTHORIZATION_REVOKED".to_string());
                }
                let source = path.join("document");
                let metadata = tokio::fs::symlink_metadata(&source)
                    .await
                    .map_err(|_| "INSPECTION_SOURCE_MISSING".to_string())?;
                if !metadata.is_file()
                    || metadata.len() != value["sourceBytes"].as_u64().unwrap_or(0)
                {
                    return Err("INSPECTION_SOURCE_CHANGED".into());
                }
                let _budget = artifacts::reserve_shared(&app.root, rip_jobs::MAX_DOCUMENT_BYTES)
                    .map_err(|_| "INSPECTION_SNAPSHOT_BUDGET_EXHAUSTED".to_string())?;
                // Never interpret a mutable queue file: seal a private snapshot first.
                let snapshot = path.join("snapshot");
                tokio::fs::create_dir(&snapshot)
                    .await
                    .map_err(|_| "INSPECTION_SNAPSHOT_FAILED".to_string())?;
                let mut input = tokio::fs::File::open(&source)
                    .await
                    .map_err(|_| "INSPECTION_SOURCE_MISSING".to_string())?;
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut output = tokio::fs::File::create(snapshot.join("source.part"))
                    .await
                    .map_err(|_| "INSPECTION_SNAPSHOT_FAILED".to_string())?;
                let copied = tokio::io::copy(
                    &mut (&mut input).take(rip_jobs::MAX_DOCUMENT_BYTES + 1),
                    &mut output,
                )
                .await
                .map_err(|_| "INSPECTION_SNAPSHOT_FAILED".to_string())?;
                output
                    .flush()
                    .await
                    .map_err(|_| "INSPECTION_SNAPSHOT_FAILED".to_string())?;
                output
                    .sync_all()
                    .await
                    .map_err(|_| "INSPECTION_SNAPSHOT_FAILED".to_string())?;
                if copied > rip_jobs::MAX_DOCUMENT_BYTES {
                    return Err("INSPECTION_SOURCE_CHANGED".into());
                }
                let (sealed, _) = rip_jobs::prepare_staged(
                    &snapshot,
                    id,
                    manifest(),
                    value["sourceSha256"].as_str(),
                )
                .await
                .map_err(|_| "INSPECTION_SOURCE_CHANGED".to_string())?;
                let mut report = match rip_interpreter::inspect_pdf_attempt(
                    &snapshot.join("document"),
                    id,
                )
                .await
                {
                    Ok(report) => report,
                    Err(error) => {
                        if matches!(
                            error,
                            rip_interpreter::InterpreterError::Rejected("container_cleanup_failed")
                        ) {
                            return Err("INSPECTION_CLEANUP_FAILED".into());
                        }
                        return Err("PDF_INSPECTION_FAILED".into());
                    }
                };
                report["inspectionId"] = json!(id);
                report["sourceSha256"] = json!(sealed.source_sha256);
                report["sourceBytes"] = value["sourceBytes"].clone();
                report["createdAt"] = json!(rip_storage::now());
                Ok(report)
            }
            .await;
            match result {
                Ok(report) => {
                    app.repo
                        .set_inspection_state(id, revision + 1, "COMPLETED", None, Some(report))
                        .await?
                }
                Err(reason) => {
                    app.repo
                        .set_inspection_state(id, revision + 1, "FAILED", Some(&reason), None)
                        .await?
                }
            }
            worked = true;
        }
        // Cleanup is durable: state/report commit precedes source deletion; repeat after crashes.
        rip_interpreter::cleanup_inspection(id).await?;
        if tokio::fs::try_exists(&path).await? {
            tokio::fs::remove_dir_all(&path).await?;
        }
        app.repo.finish_inspection_cleanup(id).await?;
        if worked {
            break;
        }
    }
    reclaim_orphans(app).await?;
    reclaim_cli_snapshots(app).await?;
    Ok(worked)
}
async fn reclaim_orphans(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let root = app.root.join("inspection-queue");
    if !tokio::fs::try_exists(&root).await? {
        return Ok(());
    }
    let mut entries = tokio::fs::read_dir(&root).await?;
    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(".upload-")
            && entry
                .metadata()
                .await?
                .modified()?
                .elapsed()
                .unwrap_or_default()
                .as_secs()
                < 3600
        {
            continue;
        }
        let id = name.strip_prefix(".upload-").unwrap_or(name);
        let Ok(id) = Uuid::parse_str(id) else {
            continue;
        };
        if app.repo.inspection_directory_registered(id).await? {
            continue;
        }
        let lock = directory_lock(&entry.path())?;
        if lock.try_lock().is_ok() {
            tokio::fs::remove_dir_all(entry.path()).await?;
        }
    }
    Ok(())
}

async fn reclaim_cli_snapshots(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let mut entries = tokio::fs::read_dir(&app.root).await?;
    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_dir()
            || !entry
                .file_name()
                .to_string_lossy()
                .starts_with("inspection-")
        {
            continue;
        }
        let marker = entry.path().join("inspection-attempt");
        let Ok(bytes) = tokio::fs::read(&marker).await else {
            continue;
        };
        if bytes.len() != 36 {
            continue;
        }
        let Ok(marker_text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let Ok(id) = Uuid::parse_str(marker_text) else {
            continue;
        };
        let lock = directory_lock(&entry.path())?;
        if lock.try_lock().is_err() {
            continue;
        }
        rip_interpreter::cleanup_inspection(id).await?;
        tokio::fs::remove_dir_all(entry.path()).await?;
    }
    Ok(())
}
