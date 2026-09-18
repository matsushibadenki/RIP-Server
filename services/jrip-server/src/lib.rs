use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rip_core::{DocumentFormat, Job, JobState, Manifest};
use rip_storage::{Repository, StorageError, now};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;
#[derive(Clone)]
pub struct App {
    pub repo: Repository,
    pub root: PathBuf,
    pub token: Arc<str>,
}
pub async fn open(root: PathBuf, token: String) -> Result<App, Box<dyn std::error::Error>> {
    tokio::fs::create_dir_all(root.join("jobs")).await?;
    let root = tokio::fs::canonicalize(root).await?;
    let repo = Repository::open(&root.join("jobs.sqlite3")).await?;
    Ok(App {
        repo,
        root,
        token: token.into(),
    })
}
pub fn router(app: App) -> Router {
    Router::new()
        .route(
            "/health",
            get(|| async {
                Json(serde_json::json!({"status":"ok","version":env!("CARGO_PKG_VERSION")}))
            }),
        )
        .route("/api/v1/jobs", get(list).post(upload))
        .route("/api/v1/jobs/{id}", get(detail).delete(delete))
        .route("/api/v1/jobs/{id}/{action}", post(action))
        .layer(DefaultBodyLimit::max(MAX_DOCUMENT_BYTES + 64 * 1024))
        .with_state(app)
}
#[derive(Debug)]
pub struct ApiError(StatusCode, &'static str);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(serde_json::json!({"error":{"code":self.1,"messages":messages(self.1)}})),
        )
            .into_response()
    }
}
fn messages(code: &str) -> serde_json::Value {
    let (en, ja, zh) = match code {
        "unauthorized" => ("Authentication required", "認証が必要です", "需要身份验证"),
        "not_found" => ("Job not found", "ジョブが見つかりません", "未找到作业"),
        "state_conflict" => (
            "The job state has changed or this action is not allowed",
            "状態が変更されたか、操作できない状態です",
            "作业状态已更改或不允许此操作",
        ),
        "internal_error" => (
            "Internal server error",
            "内部エラーが発生しました",
            "服务器内部错误",
        ),
        _ => (
            "Invalid or unsupported request",
            "無効または未対応のリクエストです",
            "请求无效或暂不支持",
        ),
    };
    serde_json::json!({"en":en,"ja":ja,"zh-CN":zh})
}
impl From<StorageError> for ApiError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::NotFound => Self(StatusCode::NOT_FOUND, "not_found"),
            StorageError::Conflict => Self(StatusCode::CONFLICT, "state_conflict"),
            e => {
                tracing::error!(error=%e,"storage failure");
                Self(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
        }
    }
}
fn bad(code: &'static str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, code)
}
fn internal(e: impl std::fmt::Display) -> ApiError {
    tracing::error!(error=%e,"request failure");
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
}
fn auth(app: &App, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = format!("Bearer {}", app.token);
    // Compare all equal-length bytes without data-dependent early exit.
    let supplied = headers
        .get("authorization")
        .map(|v| v.as_bytes())
        .unwrap_or_default();
    let difference = supplied
        .iter()
        .zip(expected.as_bytes())
        .fold(0u8, |v, (a, b)| v | (a ^ b));
    if app.token.is_empty() || supplied.len() != expected.len() || difference != 0 {
        Err(ApiError(StatusCode::UNAUTHORIZED, "unauthorized"))
    } else {
        Ok(())
    }
}
#[derive(Deserialize)]
struct Pagination {
    #[serde(default = "page_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}
fn page_limit() -> i64 {
    50
}
async fn list(
    State(app): State<App>,
    headers: HeaderMap,
    Query(p): Query<Pagination>,
) -> Result<Json<Vec<Job>>, ApiError> {
    auth(&app, &headers)?;
    Ok(Json(app.repo.list(p.limit, p.offset).await?))
}
async fn detail(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Job>, ApiError> {
    auth(&app, &headers)?;
    Ok(Json(app.repo.get(id).await?))
}
async fn upload(
    State(app): State<App>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Job>), ApiError> {
    auth(&app, &headers)?;
    let id = Uuid::new_v4();
    let directory = app.root.join("jobs").join(id.to_string());
    tokio::fs::create_dir(&directory).await.map_err(internal)?;
    let result = receive(&app, id, &directory, &mut multipart).await;
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&directory).await;
    }
    result.map(|job| (StatusCode::CREATED, Json(job)))
}
async fn receive(
    app: &App,
    id: Uuid,
    directory: &std::path::Path,
    multipart: &mut Multipart,
) -> Result<Job, ApiError> {
    let mut manifest = None;
    let mut detected = None;
    let mut hash = None;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| bad("invalid_multipart"))?
    {
        match field.name() {
            Some("manifest") if manifest.is_none() => {
                let mut bytes = Vec::new();
                while let Some(chunk) = field.chunk().await.map_err(|_| bad("invalid_multipart"))? {
                    if bytes.len() + chunk.len() > 16384 {
                        return Err(bad("manifest_too_large"));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                let value: Manifest =
                    serde_json::from_slice(&bytes).map_err(|_| bad("invalid_manifest"))?;
                value.validate().map_err(|e| bad(e.0))?;
                manifest = Some(value);
            }
            Some("document") if detected.is_none() => {
                let mut file = tokio::fs::File::create(directory.join("source.part"))
                    .await
                    .map_err(internal)?;
                let mut size = 0;
                let mut prefix = Vec::new();
                let mut digest = Sha256::new();
                while let Some(chunk) = field.chunk().await.map_err(|_| bad("invalid_multipart"))? {
                    size += chunk.len();
                    if size > MAX_DOCUMENT_BYTES {
                        return Err(ApiError(
                            StatusCode::PAYLOAD_TOO_LARGE,
                            "document_too_large",
                        ));
                    }
                    let needed = 100usize.saturating_sub(prefix.len());
                    prefix.extend_from_slice(&chunk[..chunk.len().min(needed)]);
                    digest.update(&chunk);
                    file.write_all(&chunk).await.map_err(internal)?;
                }
                file.sync_all().await.map_err(internal)?;
                detected = Some(DocumentFormat::detect(&prefix).map_err(|e| bad(e.0))?);
                hash = Some(format!("{:x}", digest.finalize()));
            }
            _ => return Err(bad("unexpected_multipart_field")),
        }
    }
    let manifest = manifest.ok_or(bad("missing_manifest"))?;
    if detected != Some(manifest.format) {
        return Err(bad("format_mismatch"));
    }
    let job = Job {
        id,
        manifest,
        state: JobState::Queued,
        source_sha256: hash.ok_or(bad("missing_document"))?,
        created_at: now(),
        updated_at: now(),
        revision: 0,
        error_code: None,
        selected_engine: None,
    };
    tokio::fs::rename(directory.join("source.part"), directory.join("document"))
        .await
        .map_err(internal)?;
    // The database is the authoritative manifest. Source publication precedes queue visibility.
    app.repo.insert(&job).await?;
    tracing::info!(job_id=%id,stage="queued","job uploaded");
    Ok(job)
}
async fn action(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, action)): Path<(Uuid, String)>,
) -> Result<Json<Job>, ApiError> {
    auth(&app, &headers)?;
    let job = app.repo.get(id).await?;
    let next = match action.as_str() {
        "hold" => JobState::Held,
        "release" if job.state == JobState::Held => JobState::Queued,
        "cancel" => JobState::Cancelled,
        _ => return Err(bad("unsupported_action")),
    };
    Ok(Json(app.repo.transition(job, next, None).await?))
}
async fn delete(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    auth(&app, &headers)?;
    app.repo.delete(app.repo.get(id).await?).await?;
    Ok(StatusCode::NO_CONTENT)
}
