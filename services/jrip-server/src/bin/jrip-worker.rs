use rip_core::JobState;
use rip_interpreter::{DocumentInterpreter, HybridInterpreter};
use rip_storage::Repository;
use uuid::Uuid;

const LEASE_SECONDS: i64 = 30;
const HEARTBEAT_SECONDS: u64 = 10;

/// Explicit single-job worker. Supervisors can run separate instances for distinct job IDs.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().json().init();
    let id: Uuid = std::env::args()
        .nth(1)
        .ok_or("Usage: jrip-worker <job-uuid>")?
        .parse()?;
    let root =
        std::fs::canonicalize(std::env::var("JRIP_DATA_ROOT").unwrap_or_else(|_| "./data".into()))?;
    let repo = Repository::open(&root.join("jobs.sqlite3")).await?;
    let mut job = repo.get(id).await?;
    if job.state != JobState::Queued {
        return Err("Job must be QUEUED".into());
    }
    let run_id = match std::env::var("JRIP_WORKER_RUN_ID") {
        Ok(value) => Uuid::parse_str(&value)?,
        Err(std::env::VarError::NotPresent) => Uuid::new_v4(),
        Err(error) => return Err(error.into()),
    };
    job.selected_engine = Some(job.manifest.engine.resolve(job.manifest.format)?);
    let job = repo
        .claim_queued(job, run_id, rip_storage::now() + LEASE_SECONDS)
        .await?;
    let heartbeat_repo = repo.clone();
    let heartbeat = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(HEARTBEAT_SECONDS)).await;
            if heartbeat_repo
                .renew_lease(id, run_id, rip_storage::now() + LEASE_SECONDS)
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let directory = root.join("jobs").join(id.to_string());
    let staging = directory.join(format!("raster-{}.part", Uuid::new_v4()));
    let mut engine = HybridInterpreter::default();
    if let Ok(image) = std::env::var("JRIP_GS_IMAGE") {
        engine.ghostscript.sandbox.image = image;
    }
    if let Ok(image) = std::env::var("JRIP_MUPDF_IMAGE") {
        engine.mupdf.sandbox.image = image;
    }
    let rendered = engine
        .render(&directory.join("document"), &staging, &job.manifest)
        .await;
    heartbeat.abort();
    let current = repo.get(id).await?;
    // A concurrent cancel wins. The bounded interpreter finishes before its output is discarded.
    if current.state == JobState::Cancelled {
        let _ = tokio::fs::remove_dir_all(staging).await;
        tracing::info!(job_id=%id,"cancelled output discarded");
        repo.clear_lease(id, run_id).await?;
        return Ok(());
    }
    match rendered {
        Ok(rendered) => {
            let result = async {
                // Publish only complete, validated output sets.
                tokio::fs::rename(&staging, directory.join("raster")).await?;
                Ok::<_, std::io::Error>(())
            }
            .await;
            if let Err(error) = result {
                repo.transition(current, JobState::Failed, Some("SPOOL_ERROR".into()))
                    .await?;
                repo.clear_lease(id, run_id).await?;
                return Err(error.into());
            }
            let spooling = repo.transition(current, JobState::Spooling, None).await?;
            repo.transition(spooling, JobState::Completed, None).await?;
            repo.clear_lease(id, run_id).await?;
            tracing::info!(job_id=%id,pages=rendered.pages.len(),engine=?job.selected_engine,output="TIFF", "file export completed");
        }
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(staging).await;
            repo.transition(current, JobState::Failed, Some(error.to_string()))
                .await?;
            repo.clear_lease(id, run_id).await?;
            return Err(error.into());
        }
    }
    Ok(())
}
