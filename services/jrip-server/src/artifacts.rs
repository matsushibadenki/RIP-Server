use rip_core::Artifact;
use sha2::{Digest, Sha256};
use std::{io, path::PathBuf};
use tokio::io::AsyncReadExt;

pub async fn hash_pages(pages: &[PathBuf]) -> io::Result<Vec<Artifact>> {
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
    }
    Ok(result)
}

use crate::{ApiError, App, auth, internal};
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

// Verify into a private snapshot before responding; a later source modification cannot
// change the bytes sent. Snapshot storage is bounded to the registered size / 1 GiB.
async fn verified_snapshot(
    app: &App,
    id: Uuid,
    name: &str,
) -> Result<
    (
        tokio::fs::File,
        tempfile::NamedTempFile,
        Artifact,
        tokio::sync::OwnedSemaphorePermit,
    ),
    ApiError,
> {
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
    let guard = tempfile::NamedTempFile::new().map_err(internal)?;
    let mut snapshot = tokio::fs::File::from_std(guard.reopen().map_err(internal)?);
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
    auth(&app, &headers)?;
    let (mut file, guard, artifact, permit) = verified_snapshot(&app, id, &name).await?;
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
    auth(&app, &headers)?;
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
