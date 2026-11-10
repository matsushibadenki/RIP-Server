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
mod artifact_descriptor;
pub mod artifacts;
mod audit;
mod authorization;
mod budget;
pub mod color;
mod console;
pub mod delivery;
mod delivery_artifact;
pub mod dispatch;
mod event_stream;
mod font_install;
mod fonts;
pub mod inspection;
mod job_files;
pub mod maintenance;
pub mod network;
mod operation_approvals;
pub mod pipeline;
mod pipeline_preflight;
mod preflight;
mod preview;
pub mod print;
mod printer_observation;
mod printer_observation_stream;
pub mod printer_observer;
mod retention;
mod retry;
pub mod runtime;
pub mod shutdown;
mod source_descriptor;
pub mod source_validation;
use authorization::authorize;
pub use authorization::{Credential, CredentialValidity};

pub const MAX_DOCUMENT_BYTES: usize = rip_jobs::MAX_DOCUMENT_BYTES as usize;
#[derive(Clone)]
pub struct App {
    pub repo: Repository,
    pub root: PathBuf,
    pub token: Arc<str>,
    pub credentials: Vec<Credential>,
    pub printers: Vec<delivery::Printer>,
    pub require_print_approval: bool,
    pub require_delete_approval: bool,
    pub local_finder: bool,
    pub require_settings_approval: bool,
    pub printer_security: Vec<delivery::PrinterSecurity>,
    pub delivery_preparation: Arc<tokio::sync::Semaphore>,
    pub snapshot_coordinator: Option<rip_storage::budget_client::SnapshotCoordinator>,
    pub artifact_budget: Arc<tokio::sync::Semaphore>,
    pub event_streams: Arc<tokio::sync::Semaphore>,
}
pub async fn open(root: PathBuf, token: String) -> Result<App, Box<dyn std::error::Error>> {
    tokio::fs::create_dir_all(root.join("jobs")).await?;
    let root = tokio::fs::canonicalize(root).await?;
    let repo = Repository::open(&root.join("jobs.sqlite3")).await?;
    let capacity = match std::env::var("JRIP_SNAPSHOT_BUDGET_BYTES") {
        Ok(value) => value.parse()?,
        Err(std::env::VarError::NotPresent) => rip_storage::budget::DEFAULT_CAPACITY,
        Err(error) => return Err(error.into()),
    };
    repo.configure_snapshot_budget(capacity).await?;
    Ok(App {
        snapshot_coordinator: rip_storage::budget_client::SnapshotCoordinator::from_env()?,
        repo,
        printers: delivery::load()?,
        require_print_approval: match std::env::var("JRIP_REQUIRE_PRINT_APPROVAL").as_deref() {
            Ok("1") => true,
            Ok("0") | Err(_) => false,
            _ => return Err("JRIP_REQUIRE_PRINT_APPROVAL must be 0 or 1".into()),
        },
        require_delete_approval: approval_flag("JRIP_REQUIRE_DELETE_APPROVAL")?,
        local_finder: approval_flag("JRIP_LOCAL_FINDER")?,
        require_settings_approval: approval_flag("JRIP_REQUIRE_SETTINGS_APPROVAL")?,
        printer_security: delivery::load_security()?,
        delivery_preparation: Arc::new(tokio::sync::Semaphore::new(1)),
        root,
        token: token.into(),
        credentials: authorization::load()?,
        artifact_budget: Arc::new(tokio::sync::Semaphore::new(1024)),
        event_streams: Arc::new(tokio::sync::Semaphore::new(64)),
    })
}
fn approval_flag(name: &str) -> Result<bool, Box<dyn std::error::Error>> {
    match std::env::var(name).as_deref() {
        Ok("1") => Ok(true),
        Ok("0") | Err(std::env::VarError::NotPresent) => Ok(false),
        _ => Err(format!("{name} must be 0 or 1").into()),
    }
}
pub fn router(app: App) -> Router {
    Router::new()
        .route("/console", get(console::index))
        .route("/console/", get(console::index))
        .route("/console/app.js", get(console::script))
        .route("/console/tabs.js", get(console::tabs_script))
        .route("/console/fonts.js", get(console::fonts_script))
        .route("/console/job-actions.js", get(console::job_actions_script))
        .route(
            "/console/output-layout.js",
            get(console::output_layout_script),
        )
        .route("/console/delivery.js", get(console::delivery_script))
        .route(
            "/console/printer-stream.js",
            get(console::printer_stream_script),
        )
        .route(
            "/console/printer-dashboard.js",
            get(console::printer_dashboard_script),
        )
        .route(
            "/console/printer-status.js",
            get(console::printer_status_script),
        )
        .route("/console/style.css", get(console::style))
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
        .route("/api/v1/session", get(authorization::context))
        .route("/api/v1/approval-policy", get(operation_approvals::policy))
        .route(
            "/api/v1/operation-approvals",
            post(operation_approvals::issue).layer(DefaultBodyLimit::max(32768)),
        )
        .route(
            "/api/v1/operation-approvals/{id}/revoke",
            post(operation_approvals::revoke),
        )
        .route("/api/v1/approvals", post(approvals::create))
        .route("/api/v1/approvals/{id}/revoke", post(approvals::revoke))
        .route("/api/v1/audit", get(audit::list))
        .route("/api/v1/budgets/snapshots", get(budget::status))
        .route(
            "/api/v1/budgets/snapshots/reservations",
            post(budget::reserve).layer(DefaultBodyLimit::max(2048)),
        )
        .route(
            "/api/v1/budgets/snapshots/reservations/{id}/release",
            post(budget::release).layer(DefaultBodyLimit::max(2048)),
        )
        .route(
            "/api/v1/budgets/snapshots/reservations/{id}/reconcile",
            post(budget::reconcile).layer(DefaultBodyLimit::max(2048)),
        )
        .route(
            "/api/v1/jobs/{id}/retention",
            get(retention::get)
                .put(retention::extend)
                .layer(DefaultBodyLimit::max(4096)),
        )
        .route("/api/v1/color/capabilities", get(color::capabilities))
        .route(
            "/api/v1/jobs/{id}/color-input-profile",
            get(color::input_profile::inspect),
        )
        .route(
            "/api/v1/color/policies",
            get(color::policies::list)
                .post(color::policies::create)
                .layer(DefaultBodyLimit::max(16384)),
        )
        .route("/api/v1/color/policies/{digest}", get(color::policies::get))
        .route(
            "/api/v1/jobs/{id}/color-attempts/from-policy",
            post(color::policies::submit).layer(DefaultBodyLimit::max(2048)),
        )
        .route(
            "/api/v1/color/profiles",
            post(color::register).layer(DefaultBodyLimit::max(4 * 1024 * 1024)),
        )
        .route("/api/v1/color/profiles/{digest}", get(color::describe))
        .route(
            "/api/v1/jobs/{id}/color-attempts",
            get(color::list).post(color::create),
        )
        .route("/api/v1/color/attempts/{id}", get(color::detail))
        .route("/api/v1/color/attempts/{id}/events", get(color::events))
        .route("/api/v1/color/attempts/{id}/cancel", post(color::cancel))
        .route(
            "/api/v1/color/attempts/{id}/artifacts/{name}",
            get(color::download),
        )
        .route(
            "/api/v1/color/attempts/{id}/artifacts/{name}/preview",
            get(preview::color),
        )
        .route("/api/v1/print/submissions", post(submit_pipeline))
        .route("/api/v1/jobs/{id}/pipelines", post(pipeline::create))
        .route("/api/v1/pipelines/{id}", get(pipeline::detail))
        .route(
            "/api/v1/pipelines/{id}/timeline/stream",
            get(event_stream::pipeline),
        )
        .route("/api/v1/pipelines/{id}/ticket", get(pipeline::ticket))
        .route("/api/v1/pipelines/{id}/events", get(pipeline::events))
        .route(
            "/api/v1/pipelines/{id}/timeline",
            get(pipeline::unified_events),
        )
        .route("/api/v1/pipelines/{id}/resume", post(pipeline::resume))
        .route("/api/v1/pipelines/{id}/cancel", post(pipeline::cancel))
        .route("/api/v1/printers", get(delivery::list))
        .route("/api/v1/printers/{id}", get(delivery::capabilities))
        .route(
            "/api/v1/printers/{id}/observations",
            get(printer_observation::list),
        )
        .route(
            "/api/v1/printers/{id}/observations/stream",
            get(printer_observation_stream::connect),
        )
        .route(
            "/api/v1/printers/{id}/observations/{sequence}",
            get(printer_observation::event),
        )
        .route("/api/v1/jobs", get(list).post(upload))
        .route("/api/v1/print/resolve", post(resolve_intent))
        .route("/api/v1/print/capabilities", get(print::capabilities))
        .route(
            "/api/v1/print/validate-ticket",
            post(print::validate_ticket),
        )
        .route("/api/v1/jobs/{id}/ticket", get(print::ticket))
        .route(
            "/api/v1/print/document-inspection-requests/{id}/events/stream",
            get(event_stream::inspection),
        )
        .route(
            "/api/v1/print/document-inspection-requests/{id}/events",
            get(inspection::events),
        )
        .route(
            "/api/v1/print/document-inspection-requests/{id}/cancel",
            post(inspection::cancel),
        )
        .route(
            "/api/v1/print/document-inspection-requests/{id}",
            post(inspection::submit).get(inspection::detail),
        )
        .route(
            "/api/v1/print/document-inspections/{id}",
            get(pipeline_preflight::inspection),
        )
        .route(
            "/api/v1/print/pipeline-preflights",
            post(pipeline_preflight::create).layer(DefaultBodyLimit::max(16384)),
        )
        .route(
            "/api/v1/print/pipeline-preflights/{id}",
            get(preflight::get),
        )
        .route(
            "/api/v1/print/pipeline-preflights/{id}/commit",
            post(pipeline_preflight::commit),
        )
        .route("/api/v1/print/preflights", post(preflight::create))
        .route("/api/v1/print/preflights/{id}", get(preflight::get))
        .route(
            "/api/v1/print/preflights/{id}/commit",
            post(preflight::commit),
        )
        .route("/api/v1/console/actions", get(job_files::capabilities))
        .route(
            "/api/v1/fonts",
            get(fonts::get)
                .post(font_install::install)
                .layer(DefaultBodyLimit::max(rip_font::MAX_BYTES)),
        )
        .route("/api/v1/jobs/{id}/reveal", post(job_files::reveal))
        .route("/api/v1/jobs/{id}", get(detail).delete(delete))
        .route("/api/v1/jobs/{id}/events", get(events))
        .route("/api/v1/jobs/{id}/retry", post(retry::retry))
        .route(
            "/api/v1/jobs/{id}/deliveries",
            get(delivery::source_list).post(delivery::prepare),
        )
        .route(
            "/api/v1/jobs/{id}/delivery-preview",
            post(delivery::preview),
        )
        .route(
            "/api/v1/deliveries/{id}/result-receipt",
            get(delivery::result_receipt),
        )
        .route("/api/v1/deliveries/{id}/ticket", get(delivery::contract))
        .route("/api/v1/deliveries/{id}", get(delivery::detail))
        .route(
            "/api/v1/deliveries/{id}/artifact/descriptor",
            get(delivery_artifact::describe),
        )
        .route(
            "/api/v1/deliveries/{id}/artifact",
            get(delivery_artifact::download),
        )
        .route("/api/v1/deliveries/{id}/cancel", post(delivery::cancel))
        .route(
            "/api/v1/deliveries/{id}/reconcile",
            post(delivery::reconcile),
        )
        .route("/api/v1/print/receipt-keys", get(preflight::receipt_keys))
        .route(
            "/api/v1/print/receipt-keys/{key_id}/revoke",
            post(preflight::revoke_receipt_key),
        )
        .route("/api/v1/jobs/{id}/receipt", get(preflight::receipt))
        .route("/api/v1/jobs/{id}/artifacts", get(preflight::artifacts))
        .route(
            "/api/v1/jobs/{id}/source/descriptor",
            get(source_descriptor::describe),
        )
        .route("/api/v1/jobs/{id}/source", get(source_descriptor::download))
        .route(
            "/api/v1/jobs/{id}/artifacts/{name}/descriptor",
            get(artifact_descriptor::raster),
        )
        .route(
            "/api/v1/color/attempts/{id}/artifacts/{name}/descriptor",
            get(artifact_descriptor::color),
        )
        .route(
            "/api/v1/jobs/{id}/artifacts/{name}/preview",
            get(preview::get),
        )
        .route(
            "/api/v1/jobs/{id}/source-validation",
            get(preflight::source_validation),
        )
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
            operation_approvals::enforce,
        ))
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
        "color_profile_inspection_limit" => (
            "Inspect at most 32 pages, 64 MiB per page and 256 MiB total",
            "32ページ・1ページ64 MiB・合計256 MiB以内の原稿を使用してください",
            "请使用最多32页、每页64 MiB、总计256 MiB的原稿",
        ),
        "invalid_artifact_retention" => (
            "Provide a revision, a future retainUntil within 3650 days, and a reason",
            "revision・3650日以内の将来のretainUntil・理由を指定してください",
            "请提供revision、3650天以内的未来retainUntil及原因",
        ),
        "invalid_color_policy_definition" => (
            "Provide version 1, three nonempty labels and a reason",
            "version 1・3言語の名称・変更理由を指定してください",
            "请提供version 1、三种语言名称及变更原因",
        ),
        "invalid_color_policy_page" => (
            "Use a policy digest cursor and a limit from 1 to 100",
            "policy digestのcursorと1〜100件の上限を指定してください",
            "请使用policy digest游标及1至100的数量限制",
        ),
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
        "delivery_page_count_mismatch" => (
            "The delivery page count differs from the selected range",
            "送信ページ数が指定範囲と一致しません",
            "投递页数与所选范围不一致",
        ),
        "delivery_artifact_mismatch" | "delivery_contract_mismatch" => (
            "Delivery bytes or contract do not match the recorded output; do not resend",
            "送信データまたは契約が保存済み出力と一致しません。再送せず確認してください",
            "投递数据或契约与登记输出不匹配，请勿重发",
        ),
        "invalid_printer_observation_query" => (
            "Use a nonnegative observation cursor, a positive sequence and a limit from 1 to 100",
            "観測cursorは0以上、sequenceは正整数、limitは1〜100で指定してください",
            "观察cursor须为非负整数，sequence须为正整数，limit须为1至100",
        ),
        "invalid_pwg_raster" => (
            "Delivery artifact is not a supported bounded PWG Raster stream",
            "送信成果物が対応範囲・容量内の正常なPWG Rasterではありません",
            "投递产物不是受支持且大小受限的有效PWG Raster",
        ),
        "source_unavailable" => (
            "Source bytes are unavailable; check retention and storage",
            "原稿データを取得できません。保持状態とストレージを確認してください",
            "原稿数据不可用，请检查保留状态和存储",
        ),
        "source_descriptor_mismatch" => (
            "Source bytes, format or validation evidence do not match the registered document",
            "原稿のbytes・形式または検証証跡が登録文書と一致しません",
            "原稿容量、格式或验证证据与登记文档不匹配",
        ),
        "artifact_layout_unsupported" => (
            "Artifact metadata requires a valid bounded single-page 8-bit RGB, CMYK or gray TIFF",
            "成果物の情報取得には、容量制限内の正常な単ページ8bit RGB・CMYK・グレーTIFFが必要です",
            "产物描述需要大小受限且有效的单页8位RGB、CMYK或灰度TIFF",
        ),
        "artifact_color_contract_mismatch" => (
            "The artifact color space or ICC does not match the recorded color ticket",
            "成果物の色空間またはICCが保存済み色変換ticketと一致しません",
            "产物色彩空间或ICC与保存的颜色ticket不匹配",
        ),
        "preview_unsupported" => (
            "Preview requires a supported 8-bit TIFF and a matching valid ICC for CMYK",
            "プレビューには対応する8ビットTIFFと、CMYKでは一致する有効なICCが必要です",
            "预览需要支持的8位TIFF，CMYK还需要匹配且有效的ICC",
        ),
        "invalid_page_ranges" => (
            "Specify ascending non-overlapping page ranges from 1 to 100",
            "ページ範囲は1〜100の昇順・重複なしで指定してください",
            "请按升序指定1至100内不重叠的页码范围",
        ),
        "page_range_out_of_bounds" => (
            "The selected range exceeds the source page count",
            "指定範囲が原稿のページ数を超えています",
            "所选范围超出原稿页数",
        ),
        "page_not_rendered" => (
            "The requested source page was not rendered",
            "指定された原稿ページは描画されていません",
            "请求的原稿页尚未渲染",
        ),
        "invalid_page_mapping" => (
            "The source page mapping is invalid",
            "原稿ページと成果物の対応が不正です",
            "原稿页与产物的映射无效",
        ),
        "page_selection_requires_pdf" => (
            "Render-time page selection requires PDF",
            "描画時のページ選択にはPDFが必要です",
            "渲染时选择页面需要PDF",
        ),
        "invalid_delivery_query" => (
            "Use a limit of 1–100 and a returned delivery cursor",
            "件数は1〜100、返却された配信カーソルを指定してください",
            "请指定1至100条和返回的投递游标",
        ),
        "invalid_color_query" => (
            "Use a limit of 1–100, a returned cursor and a valid color-attempt state",
            "件数は1〜100、返却されたカーソル、有効な色変換状態を指定してください",
            "请指定1至100条、返回的游标以及有效的色彩转换状态",
        ),
        "invalid_copies" => (
            "Specify 1–999 copies",
            "部数は1〜999で指定してください",
            "请指定1至999份",
        ),
        "unsupported_sides_or_sheet_back" => (
            "Requested sides or sheet-back mode is unsupported",
            "指定の片面・両面または裏面方式に対応していません",
            "不支持请求的单双面或背面方式",
        ),
        "unsupported_copies" => (
            "The printer does not advertise support for this copy count",
            "指定部数はプリンターの対応範囲外か、対応を確認できません",
            "打印机不支持此份数或未公布支持范围",
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
        "preflight_actor_mismatch" => (
            "Use the identity that created this preflight",
            "このpreflightを作成した主体で操作してください",
            "请使用创建此预检的身份操作",
        ),
        "preflight_actor_required" => (
            "Create a new owner-bound preflight under the current authorization policy",
            "現在の認可ポリシーで主体に紐付くpreflightを新規作成してください",
            "请按当前授权策略重新创建绑定身份的预检",
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
        "snapshot_coordinator_unavailable" => (
            "Snapshot coordinator unavailable; no local fallback was used",
            "分散予算を確認できません。ローカル予算への切替は行いません",
            "无法确认分布式预算，未回退到本地预算",
        ),
        "snapshot_cleanup_required" => (
            "Confirm snapshot cleanup before releasing the reservation",
            "予約の解放前にスナップショットの清掃を確認してください",
            "释放预留前请确认快照已清理",
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
            StorageError::BudgetExhausted => {
                Self(StatusCode::SERVICE_UNAVAILABLE, "artifact_budget_exhausted")
            }
            StorageError::NotFound => Self(StatusCode::NOT_FOUND, "not_found"),
            StorageError::Conflict => Self(StatusCode::CONFLICT, "state_conflict"),
            StorageError::ReceiptKeyRevoked => Self(
                StatusCode::SERVICE_UNAVAILABLE,
                "receipt_signing_key_revoked",
            ),
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
    let _writer = rip_jobs::upload_guard(&directory).map_err(internal)?;
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
    let (manifest, _) = stage_upload(directory, multipart, false).await?;
    let job = publish_staged(&app.repo, directory, id, manifest)
        .await
        .map_err(submit_error)?;
    tracing::info!(job_id=%id,stage="queued","job uploaded");
    Ok(job)
}
fn submit_error(error: SubmitError) -> ApiError {
    match error {
        SubmitError::Invalid(code) => bad(code),
        SubmitError::Storage(error) => error.into(),
        SubmitError::Io(error) => internal(error),
    }
}
async fn stage_upload(
    directory: &std::path::Path,
    multipart: &mut Multipart,
    allow_pipeline: bool,
) -> Result<(Manifest, Option<pipeline::Request>), ApiError> {
    let mut pipeline = None;
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
            Some("pipeline") if allow_pipeline && pipeline.is_none() => {
                let mut bytes = Vec::new();
                while let Some(chunk) = field.chunk().await.map_err(|_| bad("invalid_multipart"))? {
                    if bytes.len() + chunk.len() > 16384 {
                        return Err(bad("pipeline_too_large"));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                let request: pipeline::Request =
                    serde_json::from_slice(&bytes).map_err(|_| bad("invalid_pipeline"))?;
                if request.expected_revision != 0 {
                    return Err(bad("initial_revision_required"));
                }
                pipeline = Some(request);
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
    Ok((manifest, pipeline))
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

async fn submit_pipeline(
    State(app): State<App>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    pipeline::scopes(&app, &headers)?;
    let id = Uuid::new_v4();
    let directory = app.root.join("jobs").join(id.to_string());
    tokio::fs::create_dir(&directory).await.map_err(internal)?;
    let _writer = rip_jobs::upload_guard(&directory).map_err(internal)?;
    let mut committed = false;
    let result = async {
        let (manifest, request) = stage_upload(&directory, &mut multipart, true).await?;
        let request = request.ok_or(bad("missing_pipeline"))?;
        let (job, ticket) = rip_jobs::prepare_staged(&directory, id, manifest, None).await.map_err(submit_error)?;
        let workflow_ticket = pipeline::registration_ticket(&app, &headers, &job, &request)?;
        match app.repo.insert_with_pipeline(&job, None, Some(&ticket), Some((request.request_id, workflow_ticket.clone()))).await {
            Ok(()) => {
                committed = true;
                Ok(serde_json::json!({"jobId":job.id,"pipeline":app.repo.pipeline(request.request_id).await?}))
            },
            Err(error) => {
                match app.repo.pipeline(request.request_id).await {
                    Ok(existing) if existing["ticket"] == workflow_ticket => {
                        // Return original acceptance after revisions advance; source metadata may have been retained separately.
                        Ok(serde_json::json!({"jobId":existing["sourceJob"],"pipeline":existing}))
                    }
                    Ok(_) => Err(ApiError(StatusCode::CONFLICT, "pipeline_request_conflict")),
                    Err(StorageError::NotFound) => Err(error.into()),
                    Err(error) => Err(error.into()),
                }
            }
        }
    }.await;
    if !committed {
        let _ = tokio::fs::remove_dir_all(&directory).await;
    }
    result.map(|value| (StatusCode::ACCEPTED, Json(value)))
}
