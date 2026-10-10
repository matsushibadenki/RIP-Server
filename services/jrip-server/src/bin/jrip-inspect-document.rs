//! Trusted, authenticated local source inspection; no job is queued.
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: jrip-inspect-document source.pdf")?;
    let token = std::env::var("JRIP_API_TOKEN")?;
    let app = jrip_server::open(
        std::env::var("JRIP_DATA_ROOT")
            .unwrap_or("data".into())
            .into(),
        token.clone(),
    )
    .await?;
    let (actor, scopes) = app.access(&token).ok_or("unauthorized")?;
    if !scopes
        .iter()
        .any(|scope| scope == "admin" || scope == "jobs:submit")
    {
        return Err("jobs:submit required".into());
    }
    // Reserve the maximum bounded input before copying; share the cross-process 1 GiB budget.
    let _budget = jrip_server::artifacts::reserve_shared(&app.root, rip_jobs::MAX_DOCUMENT_BYTES)?;
    let directory = tempfile::Builder::new()
        .prefix("inspection-")
        .tempdir_in(&app.root)?;
    let id = uuid::Uuid::new_v4();
    let lease = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.path().join("upload.lock"))?;
    lease.lock()?;
    std::fs::write(directory.path().join("inspection-attempt"), id.to_string())?;
    let mut source = tokio::fs::File::open(path)
        .await?
        .take(rip_jobs::MAX_DOCUMENT_BYTES + 1);
    let mut file = tokio::fs::File::create(directory.path().join("source.part")).await?;
    let bytes = tokio::io::copy(&mut source, &mut file).await?;
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    if bytes > rip_jobs::MAX_DOCUMENT_BYTES {
        return Err("document_too_large".into());
    }
    let manifest = rip_core::Manifest {
        name: "inspection".into(),
        format: rip_core::DocumentFormat::Pdf,
        dpi: 72,
        color_mode: rip_core::ColorMode::Rgb,
        priority: 50,
        engine: rip_core::EnginePreference::MuPdf,
    };
    let (job, _) = rip_jobs::prepare_staged(directory.path(), id, manifest, None).await?;
    let mut report =
        match rip_interpreter::inspect_pdf_attempt(&directory.path().join("document"), id).await {
            Ok(value) => value,
            Err(error) => {
                if matches!(
                    &error,
                    rip_interpreter::InterpreterError::Rejected("container_cleanup_failed")
                ) {
                    eprintln!("snapshot retained: {}", directory.keep().display());
                }
                return Err(error.into());
            }
        };
    report["inspectionId"] = serde_json::json!(id);
    report["sourceSha256"] = serde_json::json!(job.source_sha256);
    report["sourceBytes"] = serde_json::json!(bytes);
    report["createdAt"] = serde_json::json!(rip_storage::now());
    app.repo
        .save_document_inspection(id, &actor, &report)
        .await?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
