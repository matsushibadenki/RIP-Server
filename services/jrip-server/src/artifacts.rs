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
) -> Result<(tokio::fs::File, tempfile::NamedTempFile, Artifact), ApiError> {
    let artifacts: Vec<Artifact> =
        serde_json::from_value(app.repo.artifacts(id).await?).map_err(internal)?;
    let artifact = artifacts
        .into_iter()
        .find(|a| a.name == name)
        .ok_or(ApiError(StatusCode::NOT_FOUND, "not_found"))?;
    if name.contains('/')
        || name.contains('\\')
        || name != format!("page-{:06}.tiff", artifact.page)
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
    Ok((snapshot, guard, artifact))
}

pub async fn download(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, name)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    auth(&app, &headers)?;
    let (file, guard, artifact) = verified_snapshot(&app, id, &name).await?;
    let stream = futures_util::stream::try_unfold((file, guard), |(mut file, guard)| async move {
        let mut buffer = vec![0u8; 65536];
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            return Ok::<_, io::Error>(None);
        }
        buffer.truncate(n);
        Ok(Some((buffer, (file, guard))))
    });
    let mut response = Body::from_stream(stream).into_response();
    let headers = response.headers_mut();
    headers.insert("content-type", "image/tiff".parse().unwrap());
    headers.insert(
        "content-length",
        artifact.bytes.to_string().parse().map_err(internal)?,
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
    let (_, _, artifact) = verified_snapshot(&app, id, &name).await?;
    Ok(Json(
        json!({"verified":true,"jobId":id,"artifact":artifact}),
    ))
}
