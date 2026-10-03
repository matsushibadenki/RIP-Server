//! Common, bounded job publication for REST and IPP frontends.
use rip_core::{DocumentFormat, Job, JobState, Manifest};
use rip_storage::{Repository, StorageError, now};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

pub const MAX_DOCUMENT_BYTES: u64 = 32 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Storage(#[from] StorageError),
}
pub async fn publish_staged(
    repo: &Repository,
    directory: &Path,
    id: Uuid,
    manifest: Manifest,
) -> Result<Job, SubmitError> {
    publish_bound(repo, directory, id, manifest, None).await
}
pub async fn publish_bound(
    repo: &Repository,
    directory: &Path,
    id: Uuid,
    manifest: Manifest,
    binding: Option<(Uuid, &str)>,
) -> Result<Job, SubmitError> {
    manifest.validate().map_err(|e| SubmitError::Invalid(e.0))?;
    let mut file = tokio::fs::File::open(directory.join("source.part")).await?;
    let mut data = [0u8; 65536];
    let mut prefix = Vec::with_capacity(100);
    let mut hash = Sha256::new();
    let mut size = 0u64;
    loop {
        let n = file.read(&mut data).await?;
        if n == 0 {
            break;
        }
        size = size
            .checked_add(n as u64)
            .ok_or(SubmitError::Invalid("document_too_large"))?;
        if size > MAX_DOCUMENT_BYTES {
            return Err(SubmitError::Invalid("document_too_large"));
        }
        let keep = 100usize.saturating_sub(prefix.len()).min(n);
        prefix.extend_from_slice(&data[..keep]);
        hash.update(&data[..n]);
    }
    if size == 0 {
        return Err(SubmitError::Invalid("missing_document"));
    }
    let detected = DocumentFormat::detect(&prefix).map_err(|e| SubmitError::Invalid(e.0))?;
    if detected != manifest.format {
        return Err(SubmitError::Invalid("format_mismatch"));
    }
    drop(file);
    let now = now();
    let job = Job {
        id,
        manifest,
        state: JobState::Queued,
        source_sha256: format!("{:x}", hash.finalize()),
        created_at: now,
        updated_at: now,
        revision: 0,
        error_code: None,
        selected_engine: None,
        worker_run_id: None,
    };
    if binding.is_some_and(|(_, expected)| expected != job.source_sha256) {
        return Err(SubmitError::Invalid("source_hash_mismatch"));
    }
    tokio::fs::rename(directory.join("source.part"), directory.join("document")).await?;
    repo.insert_with_preflight(&job, binding.map(|(id, _)| id))
        .await?;
    Ok(job)
}
pub async fn submit_bytes(
    repo: &Repository,
    root: &Path,
    manifest: Manifest,
    document: &[u8],
) -> Result<Job, SubmitError> {
    if document.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(SubmitError::Invalid("document_too_large"));
    }
    let id = Uuid::new_v4();
    let dir = root.join("jobs").join(id.to_string());
    tokio::fs::create_dir(&dir).await?;
    let result = async {
        let mut file = tokio::fs::File::create(dir.join("source.part")).await?;
        file.write_all(document).await?;
        file.sync_all().await?;
        publish_staged(repo, &dir, id, manifest).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
    result
}
