use rip_core::Artifact;
use sha2::{Digest, Sha256};
use std::{io, path::PathBuf};
use tokio::io::AsyncReadExt;

pub async fn hash_pages(pages: &[PathBuf]) -> io::Result<Vec<Artifact>> {
    hash_pages_with_progress(pages, |_, _| async { Ok(()) }).await
}

/// Reports only pages whose complete bytes have been read and hashed.
pub async fn hash_pages_with_progress<F, Fut>(
    pages: &[PathBuf],
    mut progress: F,
) -> io::Result<Vec<Artifact>>
where
    F: FnMut(u32, u32) -> Fut,
    Fut: std::future::Future<Output = io::Result<()>>,
{
    if pages.is_empty() || pages.len() > 100 {
        return Err(io::Error::other("invalid page count"));
    }
    let mut result = Vec::new();
    let mut total = 0u64;
    for (i, path) in pages.iter().enumerate() {
        let page = i as u32 + 1;
        let name = format!("page-{page:06}.tiff");
        if path.file_name().and_then(|n| n.to_str()) != Some(&name)
            || !tokio::fs::symlink_metadata(path)
                .await?
                .file_type()
                .is_file()
        {
            return Err(io::Error::other("invalid artifact"));
        }
        let mut file = tokio::fs::File::open(path).await?;
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            total += n as u64;
            if total > 1024 * 1024 * 1024 {
                return Err(io::Error::other("artifact budget exceeded"));
            }
            hash.update(&buffer[..n]);
        }
        if bytes == 0 {
            return Err(io::Error::other("empty artifact"));
        }
        result.push(Artifact {
            page,
            name,
            media_type: "image/tiff".into(),
            bytes,
            sha256: format!("{:x}", hash.finalize()),
        });
        progress(page, pages.len() as u32).await?;
    }
    Ok(result)
}

use crate::{ApiError, App, authorize, internal};
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use uuid::Uuid;

/// Own the snapshot and release remote quota only after unlink succeeds.
pub(crate) struct SnapshotFile {
    file: Option<tempfile::NamedTempFile>,
    reservation: Option<rip_storage::budget_client::Reservation>,
}
impl SnapshotFile {
    pub fn path(&self) -> &std::path::Path {
        self.file.as_ref().unwrap().path()
    }
}
impl Drop for SnapshotFile {
    fn drop(&mut self) {
        let cleaned = self.file.take().is_none_or(|file| file.close().is_ok());
        if let Some(reservation) = self.reservation.take() {
            if cleaned {
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    runtime.spawn(async move {
                        let id = reservation.id;
                        if let Err(error) = reservation.release().await {
                            tracing::warn!(reservation_id=%id, %error, "snapshot release unconfirmed; quota retained");
                        }
                    });
                }
            } else {
                tracing::warn!(reservation_id=%reservation.id, "snapshot cleanup failed; quota retained");
            }
        }
    }
}

pub(crate) struct SnapshotBudget {
    _local: tokio::sync::OwnedSemaphorePermit,
    _shared: Vec<std::fs::File>,
}
// 64 OS-locked 16 MiB slots bound all processes sharing the same data root.
// Closing handles (including process exit) releases reservations without stale counters.
pub fn reserve_shared(root: &std::path::Path, bytes: u64) -> io::Result<Vec<std::fs::File>> {
    let directory = std::fs::canonicalize(root)?.join("artifact-budget");
    std::fs::create_dir_all(&directory)?;
    if std::fs::canonicalize(&directory)? != directory {
        return Err(io::Error::other("invalid budget root"));
    }
    let slots = bytes.div_ceil(16 * 1024 * 1024).max(1);
    if slots > 64 {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "budget exhausted",
        ));
    }
    let mut locks = Vec::new();
    for index in 0..64 {
        let path = directory.join(format!("slot-{index:02}"));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        if std::fs::canonicalize(&path)? != path {
            return Err(io::Error::other("invalid budget slot"));
        }
        match file.try_lock() {
            Ok(()) => locks.push(file),
            Err(std::fs::TryLockError::WouldBlock) => continue,
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
        if locks.len() as u64 == slots {
            return Ok(locks);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "budget exhausted",
    ))
}
// Verify into a private snapshot before responding; a later source modification cannot
// change the bytes sent. Snapshot storage is bounded to the registered size / 1 GiB.
pub(crate) async fn verified_snapshot(
    app: &App,
    id: Uuid,
    name: &str,
) -> Result<(tokio::fs::File, SnapshotFile, Artifact, SnapshotBudget), ApiError> {
    let artifacts: Vec<Artifact> =
        serde_json::from_value(app.repo.artifacts(id).await?).map_err(internal)?;
    let artifact = artifacts
        .into_iter()
        .find(|a| a.name == name)
        .ok_or(ApiError(StatusCode::NOT_FOUND, "not_found"))?;
    if name.contains('/')
        || name.contains('\\')
        || name != format!("page-{:06}.tiff", artifact.page)
        || artifact.bytes == 0
        || artifact.bytes > 1024 * 1024 * 1024
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "artifact_integrity_mismatch",
        ));
    }
    let path = app
        .root
        .join("jobs")
        .join(id.to_string())
        .join("raster")
        .join(name);
    snapshot_file(app, path, artifact).await
}
pub(crate) type Snapshot = (tokio::fs::File, SnapshotFile, Artifact, SnapshotBudget);
pub(crate) async fn snapshot_file(
    app: &App,
    path: PathBuf,
    artifact: Artifact,
) -> Result<Snapshot, ApiError> {
    if artifact.bytes == 0
        || artifact.bytes > 1024 * 1024 * 1024
        || artifact.name != format!("page-{:06}.tiff", artifact.page)
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "artifact_integrity_mismatch",
        ));
    }
    let canonical = tokio::fs::canonicalize(&path)
        .await
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "not_found"))?;
    if canonical != path {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "artifact_integrity_mismatch",
        ));
    }
    let mut source = tokio::fs::File::open(path).await.map_err(internal)?;
    let metadata = source.metadata().await.map_err(internal)?;
    if !metadata.is_file() || metadata.len() != artifact.bytes {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "artifact_integrity_mismatch",
        ));
    }
    let permits = artifact.bytes.div_ceil(1024 * 1024).max(1) as u32;
    let permit = app
        .artifact_budget
        .clone()
        .try_acquire_many_owned(permits)
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "artifact_budget_exhausted"))?;
    let root = app.root.clone();
    let bytes = artifact.bytes;
    let shared = tokio::task::spawn_blocking(move || reserve_shared(&root, bytes))
        .await
        .map_err(internal)?
        .map_err(|error| {
            if error.kind() == io::ErrorKind::WouldBlock {
                ApiError(StatusCode::SERVICE_UNAVAILABLE, "artifact_budget_exhausted")
            } else {
                internal(error)
            }
        })?;
    let permit = SnapshotBudget {
        _local: permit,
        _shared: shared,
    };
    let snapshots = app.root.join("artifact-snapshots");
    tokio::fs::create_dir_all(&snapshots)
        .await
        .map_err(internal)?;
    if tokio::fs::canonicalize(&snapshots)
        .await
        .map_err(internal)?
        != snapshots
    {
        return Err(internal("invalid snapshot root"));
    }
    let reservation = if let Some(coordinator) = &app.snapshot_coordinator {
        Some(coordinator.reserve(artifact.bytes).await.map_err(|error| {
            tracing::warn!(%error,"distributed snapshot admission failed");
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                if error.kind() == io::ErrorKind::WouldBlock {
                    "artifact_budget_exhausted"
                } else {
                    "snapshot_coordinator_unavailable"
                },
            )
        })?)
    } else {
        None
    };
    let prefix = reservation.as_ref().map_or_else(
        || "artifact-snapshot-".to_string(),
        |reservation| format!("artifact-snapshot-{}-", reservation.id),
    );
    let mut guard = SnapshotFile {
        file: None,
        reservation,
    };
    guard.file = Some(
        tempfile::Builder::new()
            .prefix(&prefix)
            .suffix(".part")
            .tempfile_in(snapshots)
            .map_err(internal)?,
    );
    let tempfile = guard.file.as_ref().unwrap();
    tempfile.as_file().try_lock().map_err(internal)?;
    let mut snapshot = tokio::fs::File::from_std(tempfile.reopen().map_err(internal)?);
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let n = source.read(&mut buffer).await.map_err(internal)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > artifact.bytes {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "artifact_integrity_mismatch",
            ));
        }
        hash.update(&buffer[..n]);
        snapshot.write_all(&buffer[..n]).await.map_err(internal)?;
    }
    if size != artifact.bytes || format!("{:x}", hash.finalize()) != artifact.sha256 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "artifact_integrity_mismatch",
        ));
    }
    snapshot.flush().await.map_err(internal)?;
    snapshot.rewind().await.map_err(internal)?;
    Ok((snapshot, guard, artifact, permit))
}

pub async fn download(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, name)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, "artifacts:read")?;
    let snapshot = verified_snapshot(&app, id, &name).await?;
    snapshot_response(headers, snapshot).await
}
pub(crate) async fn snapshot_response(
    headers: HeaderMap,
    snapshot: Snapshot,
) -> Result<Response, ApiError> {
    let (mut file, guard, artifact, permit) = snapshot;
    let etag = format!("\"sha256:{}\"", artifact.sha256);
    let requested = if headers
        .get("if-range")
        .is_none_or(|v| v.as_bytes() == etag.as_bytes())
    {
        headers.get("range")
    } else {
        None
    };
    let range = if let Some(value) = requested {
        let parsed = value
            .to_str()
            .ok()
            .and_then(|v| parse_range(v, artifact.bytes));
        let Some(range) = parsed else {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            response.headers_mut().insert(
                "content-range",
                format!("bytes */{}", artifact.bytes)
                    .parse()
                    .map_err(internal)?,
            );
            return Ok(response);
        };
        Some(range)
    } else {
        None
    };
    let (start, end) = range.unwrap_or((0, artifact.bytes - 1));
    let length = end - start + 1;
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(internal)?;
    let stream = futures_util::stream::try_unfold(
        (file, guard, permit, length),
        |(mut file, guard, permit, remaining)| async move {
            if remaining == 0 {
                return Ok::<_, io::Error>(None);
            }
            let mut buffer = vec![0u8; remaining.min(65536) as usize];
            let n = file.read(&mut buffer).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "snapshot truncated",
                ));
            }
            buffer.truncate(n);
            Ok(Some((buffer, (file, guard, permit, remaining - n as u64))))
        },
    );
    let mut response = Body::from_stream(stream).into_response();
    if range.is_some() {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    }
    let headers = response.headers_mut();
    headers.insert("accept-ranges", "bytes".parse().unwrap());
    if range.is_some() {
        headers.insert(
            "content-range",
            format!("bytes {start}-{end}/{}", artifact.bytes)
                .parse()
                .map_err(internal)?,
        );
    }
    headers.insert("content-type", "image/tiff".parse().unwrap());
    headers.insert(
        "content-length",
        length.to_string().parse().map_err(internal)?,
    );
    headers.insert(
        "content-disposition",
        format!("attachment; filename=\"{}\"", artifact.name)
            .parse()
            .map_err(internal)?,
    );
    headers.insert(
        "etag",
        format!("\"sha256:{}\"", artifact.sha256)
            .parse()
            .map_err(internal)?,
    );
    headers.insert("cache-control", "private, no-store".parse().unwrap());
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    Ok(response)
}
pub async fn verify(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, name)): Path<(Uuid, String)>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "artifacts:read")?;
    let (_, _, artifact, _) = verified_snapshot(&app, id, &name).await?;
    Ok(Json(
        json!({"verified":true,"jobId":id,"artifact":artifact}),
    ))
}

fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    let text = value.strip_prefix("bytes=")?;
    if size == 0 || text.contains(',') || text.chars().any(char::is_whitespace) {
        return None;
    }
    let (start, end) = text.split_once('-')?;
    let number = |s: &str| {
        if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
            s.parse::<u64>().ok()
        } else {
            None
        }
    };
    if start.is_empty() {
        let suffix = number(end)?;
        if suffix == 0 {
            return None;
        }
        return Some((size.saturating_sub(suffix), size - 1));
    }
    let start = number(start)?;
    let end = if end.is_empty() {
        size - 1
    } else {
        number(end)?.min(size - 1)
    };
    if start >= size || start > end {
        None
    } else {
        Some((start, end))
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn independent_reservations_share_capacity_and_release_on_drop() {
        let root = tempfile::tempdir().unwrap();
        let full = reserve_shared(root.path(), 1024 * 1024 * 1024).unwrap();
        assert_eq!(
            reserve_shared(root.path(), 1).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(full);
        let small = reserve_shared(root.path(), 1).unwrap();
        assert_eq!(small.len(), 1);
        assert_eq!(
            reserve_shared(root.path(), 1024 * 1024 * 1024)
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        drop(small);
        assert_eq!(
            reserve_shared(root.path(), 1024 * 1024 * 1024)
                .unwrap()
                .len(),
            64
        );
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    #[tokio::test]
    async fn reports_only_complete_pages_and_stops_when_observer_fails() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("page-000001.tiff");
        let second = root.path().join("page-000002.tiff");
        tokio::fs::write(&first, b"page bytes").await.unwrap();
        tokio::fs::write(&second, b"").await.unwrap();
        let mut observed = Vec::new();
        let result = hash_pages_with_progress(&[first.clone(), second.clone()], |page, total| {
            observed.push((page, total));
            async { Ok(()) }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(observed, vec![(1, 2)]);
        observed.clear();
        let result = hash_pages_with_progress(&[first, second], |page, total| {
            observed.push((page, total));
            async { Err(io::Error::other("lease lost")) }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(observed, vec![(1, 2)]);
    }
}
