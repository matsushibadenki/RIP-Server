use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rip_core::{Job, JobState, Manifest};
use rip_jobs::{SubmitError, publish_staged};
use rip_storage::{Repository, StorageError};
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

mod approvals;
pub mod artifacts;
mod audit;
mod authorization;
pub mod color;
pub mod delivery;
pub mod dispatch;
pub mod maintenance;
mod preflight;
mod retry;
pub use authorization::Credential;
use authorization::authorize;

pub const MAX_DOCUMENT_BYTES: usize = rip_jobs::MAX_DOCUMENT_BYTES as usize;
#[derive(Clone)]
pub struct App {
    pub repo: Repository,
    pub root: PathBuf,
    pub token: Arc<str>,
    pub credentials: Vec<Credential>,
    pub printers: Vec<delivery::Printer>,
    pub require_print_approval: bool,
    pub printer_security: Vec<delivery::PrinterSecurity>,
    pub delivery_preparation: Arc<tokio::sync::Semaphore>,
    pub artifact_budget: Arc<tokio::sync::Semaphore>,
}
pub async fn open(root: PathBuf, token: String) -> Result<App, Box<dyn std::error::Error>> {
    tokio::fs::create_dir_all(root.join("jobs")).await?;
    let root = tokio::fs::canonicalize(root).await?;
    let repo = Repository::open(&root.join("jobs.sqlite3")).await?;
    Ok(App {
        repo,
        printers: delivery::load()?,
        require_print_approval: match std::env::var("JRIP_REQUIRE_PRINT_APPROVAL").as_deref() {
            Ok("1") => true,
            Ok("0") | Err(_) => false,
            _ => return Err("JRIP_REQUIRE_PRINT_APPROVAL must be 0 or 1".into()),
        },
        printer_security: delivery::load_security()?,
        delivery_preparation: Arc::new(tokio::sync::Semaphore::new(1)),
        root,
        token: token.into(),
        credentials: authorization::load()?,
        artifact_budget: Arc::new(tokio::sync::Semaphore::new(1024)),
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
        .route(
            "/.well-known/acx.json",
            get(|| async { Json(rip_acx::manifest()) }),
        )
        .route("/api/v1/approvals", post(approvals::create))
        .route("/api/v1/approvals/{id}/revoke", post(approvals::revoke))
        .route("/api/v1/audit", get(audit::list))
        .route("/api/v1/color/capabilities", get(color::capabilities))
        .route(
            "/api/v1/color/profiles",
            post(color::register).layer(DefaultBodyLimit::max(4 * 1024 * 1024)),
        )
        .route("/api/v1/color/profiles/{digest}", get(color::describe))
        .route("/api/v1/jobs/{id}/color-attempts", post(color::create))
        .route("/api/v1/color/attempts/{id}", get(color::detail))
        .route("/api/v1/color/attempts/{id}/events", get(color::events))
        .route("/api/v1/color/attempts/{id}/cancel", post(color::cancel))
        .route(
            "/api/v1/color/attempts/{id}/artifacts/{name}",
            get(color::download),
        )
        .route("/api/v1/printers", get(delivery::list))
        .route("/api/v1/printers/{id}", get(delivery::capabilities))
        .route("/api/v1/jobs", get(list).post(upload))
        .route("/api/v1/print/resolve", post(resolve_intent))
        .route("/api/v1/print/preflights", post(preflight::create))
        .route("/api/v1/print/preflights/{id}", get(preflight::get))
        .route(
            "/api/v1/print/preflights/{id}/commit",
            post(preflight::commit),
        )
        .route("/api/v1/jobs/{id}", get(detail).delete(delete))
        .route("/api/v1/jobs/{id}/events", get(events))
        .route("/api/v1/jobs/{id}/retry", post(retry::retry))
        .route("/api/v1/jobs/{id}/deliveries", post(delivery::prepare))
        .route("/api/v1/deliveries/{id}", get(delivery::detail))
        .route("/api/v1/deliveries/{id}/cancel", post(delivery::cancel))
        .route(
            "/api/v1/deliveries/{id}/reconcile",
            post(delivery::reconcile),
        )
        .route("/api/v1/jobs/{id}/receipt", get(preflight::receipt))
        .route("/api/v1/jobs/{id}/artifacts", get(preflight::artifacts))
        .route(
            "/api/v1/jobs/{id}/artifacts/{name}",
            get(artifacts::download),
        )
        .route(
            "/api/v1/jobs/{id}/artifacts/{name}/verify",
            post(artifacts::verify),
        )
        .route(
            "/api/v1/jobs/{id}/result-receipt",
            get(preflight::result_receipt),
        )
        .route("/api/v1/jobs/{id}/{action}", post(action))
        .layer(DefaultBodyLimit::max(MAX_DOCUMENT_BYTES + 64 * 1024))
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            audit::record,
        ))
        .with_state(app)
}
#[derive(Debug)]
pub struct ApiError(StatusCode, &'static str);
async fn resolve_intent(
    State(app): State<App>,
    headers: HeaderMap,
    Json(intent): Json<rip_acx::intent::PrintIntent>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&app, &headers, "jobs:submit")?;
    Ok(Json(rip_acx::intent::resolve(intent)))
}
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
        "invalid_icc_profile" => (
            "Invalid or unavailable ICC profile",
            "ICCが不正または利用できません",
            "ICC无效或不可用",
        ),
        "unsupported_color_policy" => (
            "Unsupported color policy; specify matching profiles and supported settings",
            "ICCと対応設定が一致する色処理ポリシーを指定してください",
            "请指定匹配ICC及受支持设置的颜色策略",
        ),
        "invalid_color_attempt" => (
            "Completed color attempt must match the source revision",
            "完了した色処理attemptと原稿revisionが一致する必要があります",
            "已完成颜色尝试须匹配原稿revision",
        ),
        "printer_color_target_required" => (
            "Configure the printer output profile before sending color-managed output",
            "色成果物の送信前にプリンターの出力profileを設定してください",
            "发送颜色成果物前请配置打印机输出profile",
        ),
        "printer_color_profile_mismatch" => (
            "Color profile does not match the configured printer target",
            "色profileがプリンター設定と一致しません",
            "颜色profile与打印机目标配置不符",
        ),
        "color_management_required" => (
            "This printer requires an explicit completed color attempt",
            "このプリンターは明示的に完了した色処理attemptが必要です",
            "此打印机须使用显式完成的颜色尝试",
        ),
        "approval_required" => (
            "Approval required for this exact delivery",
            "この出力要求に対応する承認が必要です",
            "此投递请求需要对应审批",
        ),
        "audit_unavailable" => (
            "Audit storage unavailable",
            "監査記録を保存できません",
            "无法保存审计记录",
        ),
        "forbidden" => (
            "Insufficient permission",
            "操作権限がありません",
            "操作权限不足",
        ),
        "unauthorized" => ("Authentication required", "認証が必要です", "需要身份验证"),
        "invalid_event_cursor" => (
            "after must be nonnegative and limit must be between 1 and 100",
            "afterは0以上、limitは1から100で指定してください",
            "after必须为非负数，limit必须在1到100之间",
        ),
        "artifact_budget_exhausted" => (
            "Temporary artifact budget exhausted; retry after current downloads finish",
            "成果物用の一時容量予算が不足しています。進行中の取得が完了してから再試行してください",
            "成果物临时容量预算不足，请等待当前下载完成后重试",
        ),
        "artifact_integrity_mismatch" => (
            "Artifact size or hash does not match the recorded output",
            "成果物のサイズまたはhashが記録と一致しません",
            "成果物大小或hash与记录不一致",
        ),
        "preflight_expired" => (
            "Preflight expired; create a new one",
            "事前確認の期限が切れました。再作成してください",
            "预检已过期，请重新创建",
        ),
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
#[derive(Deserialize)]
struct EventCursor {
    #[serde(default)]
    after: i64,
    #[serde(default = "page_limit")]
    limit: i64,
}
async fn events(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(cursor): Query<EventCursor>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    if cursor.after < 0 || !(1..=100).contains(&cursor.limit) {
        return Err(bad("invalid_event_cursor"));
    }
    Ok(Json(app.repo.events(id, cursor.after, cursor.limit).await?))
}
async fn list(
    State(app): State<App>,
    headers: HeaderMap,
    Query(p): Query<Pagination>,
) -> Result<Json<Vec<Job>>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(app.repo.list(p.limit, p.offset).await?))
}
async fn detail(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Job>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(app.repo.get(id).await?))
}
async fn upload(
    State(app): State<App>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Job>), ApiError> {
    authorize(&app, &headers, "jobs:submit")?;
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
    let mut document_received = false;
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
            Some("document") if !document_received => {
                let mut file = tokio::fs::File::create(directory.join("source.part"))
                    .await
                    .map_err(internal)?;
                let mut size = 0;
                while let Some(chunk) = field.chunk().await.map_err(|_| bad("invalid_multipart"))? {
                    size += chunk.len();
                    if size > MAX_DOCUMENT_BYTES {
                        return Err(ApiError(
                            StatusCode::PAYLOAD_TOO_LARGE,
                            "document_too_large",
                        ));
                    }
                    file.write_all(&chunk).await.map_err(internal)?;
                }
                file.sync_all().await.map_err(internal)?;
                document_received = true;
            }
            _ => return Err(bad("unexpected_multipart_field")),
        }
    }
    let manifest = manifest.ok_or(bad("missing_manifest"))?;
    if !document_received {
        return Err(bad("missing_document"));
    }
    let job = publish_staged(&app.repo, directory, id, manifest)
        .await
        .map_err(|e| match e {
            SubmitError::Invalid(code) => bad(code),
            SubmitError::Storage(error) => ApiError::from(error),
            SubmitError::Io(error) => internal(error),
        })?;
    tracing::info!(job_id=%id,stage="queued","job uploaded");
    Ok(job)
}
async fn action(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, action)): Path<(Uuid, String)>,
) -> Result<Json<Job>, ApiError> {
    authorize(&app, &headers, "jobs:control")?;
    let job = app.repo.get(id).await?;
    if let Some(expected) = headers.get("x-jrip-expected-revision") {
        let expected = expected
            .to_str()
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v >= 0)
            .ok_or_else(|| bad("invalid_revision"))?;
        if expected != job.revision {
            return Err(ApiError(StatusCode::CONFLICT, "state_conflict"));
        }
    }
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
    authorize(&app, &headers, "admin")?;
    let job = app.repo.get(id).await?;
    if let Some(expected) = headers.get("x-jrip-expected-revision") {
        let expected = expected
            .to_str()
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v >= 0)
            .ok_or_else(|| bad("invalid_revision"))?;
        if expected != job.revision {
            return Err(ApiError(StatusCode::CONFLICT, "state_conflict"));
        }
    }
    app.repo.delete(job).await?;
    Ok(StatusCode::NO_CONTENT)
}
