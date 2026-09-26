use rip_core::{Job, JobState};
use rip_storage::{Repository, StorageError};
use uuid::Uuid;

/// Reconcile only a worker run owned by this dispatcher. A competing manual worker
/// or another run must never be failed because this child process exited.
pub async fn reconcile_worker_exit(
    repo: &Repository,
    job_id: Uuid,
    run_id: Uuid,
) -> Result<Job, StorageError> {
    let current = repo.get(job_id).await?;
    if current.worker_run_id != Some(run_id) {
        return Ok(current);
    }
    if !matches!(
        current.state,
        JobState::Ripping
            | JobState::ColorProcessing
            | JobState::Halftoning
            | JobState::Spooling
            | JobState::Printing
    ) {
        repo.clear_lease(job_id, run_id).await?;
        return Ok(current);
    }
    let result = match repo
        .transition(current, JobState::Failed, Some("WORKER_EXIT".into()))
        .await
    {
        Ok(job) => Ok(job),
        Err(StorageError::Conflict) => repo.get(job_id).await,
        Err(error) => Err(error),
    }?;
    repo.clear_lease(job_id, run_id).await?;
    Ok(result)
}
