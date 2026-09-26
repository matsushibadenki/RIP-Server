use rip_core::JobState;
use rip_storage::Repository;
use std::{path::PathBuf, time::Duration};
use tokio::process::Command;
use uuid::Uuid;

/// Experimental single-host dispatcher. The worker's revision-checked state transition
/// prevents two dispatchers from rendering the same queued job.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().json().init();
    let once = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--once") => true,
        Some(_) => return Err("Usage: jrip-dispatcher [--once]".into()),
    };
    let root =
        std::fs::canonicalize(std::env::var("JRIP_DATA_ROOT").unwrap_or_else(|_| "./data".into()))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("dispatcher.lock"))?;
    lock.try_lock()?;
    let worker = if let Ok(path) = std::env::var("JRIP_WORKER_BIN") {
        PathBuf::from(path)
    } else {
        std::env::current_exe()?
            .with_file_name(format!("jrip-worker{}", std::env::consts::EXE_SUFFIX))
    };
    let repo = Repository::open(&root.join("jobs.sqlite3")).await?;
    let mut shutdown = Box::pin(tokio::signal::ctrl_c());
    loop {
        for id in repo.recover_expired_leases(rip_storage::now()).await? {
            tracing::warn!(job_id=%id, "expired worker lease recovered as failed");
        }
        if let Some(job) = repo.next_queued().await? {
            tracing::info!(job_id=%job.id, "dispatching queued job");
            let run_id = Uuid::new_v4();
            let mut child = Command::new(&worker)
                .arg(job.id.to_string())
                .env("JRIP_DATA_ROOT", &root)
                .env("JRIP_WORKER_RUN_ID", run_id.to_string())
                .spawn()?;
            let mut stop = false;
            let status = tokio::select! {
                status = child.wait() => status?,
                _ = &mut shutdown => {
                    stop = true;
                    tracing::info!(job_id=%job.id, "waiting for current worker before shutdown");
                    child.wait().await?
                }
            };
            let current =
                jrip_server::dispatch::reconcile_worker_exit(&repo, job.id, run_id).await?;
            tracing::info!(job_id=%job.id, ?status, state=?current.state, "worker exited");
            if current.state == JobState::Queued {
                return Err("Worker exited without claiming the queued job".into());
            }
            if once || stop {
                break;
            }
        } else if once {
            break;
        } else {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(2)) => {},
                _ = &mut shutdown => break,
            }
        }
    }
    Ok(())
}
