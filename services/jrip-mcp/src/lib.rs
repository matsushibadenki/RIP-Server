use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use base64::Engine as _;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;
use uuid::Uuid;
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub mod http;
pub mod rate_limit;
const MAX_SUBSCRIPTIONS: usize = 32;
pub struct Session {
    router: Router,
    audit: Option<rip_storage::Repository>,
    token: String,
    initialized: bool,
    ready: bool,
    identity: String,
    scopes: Vec<String>,
    input_root: Option<PathBuf>,
    subscriptions: BTreeMap<String, i64>,
}
impl Session {
    pub fn new(router: Router, token: String) -> Self {
        Self {
            router,
            audit: None,
            token,
            initialized: false,
            ready: false,
            identity: "legacy-local".into(),
            scopes: vec!["admin".into()],
            input_root: None,
            subscriptions: BTreeMap::new(),
        }
    }
    pub fn with_access(
        mut self,
        identity: String,
        scopes: Vec<String>,
        input_root: Option<PathBuf>,
    ) -> Self {
        self.identity = identity;
        self.scopes = scopes;
        self.input_root = input_root;
        self
    }
    pub fn with_audit(mut self, repo: rip_storage::Repository) -> Self {
        self.audit = Some(repo);
        self
    }
    async fn audited_call(&self, name: &str, args: &Value) -> Result<Value, Value> {
        let target = args["jobId"]
            .as_str()
            .or_else(|| args["deliveryId"].as_str())
            .or_else(|| args["colorAttemptId"].as_str())
            .or_else(|| args["pipelineId"].as_str())
            .or_else(|| args["inspectionId"].as_str())
            .and_then(|id| Uuid::parse_str(id).ok());
        let sequence = if let Some(repo) = &self.audit {
            Some(
                repo.begin_audit(Uuid::new_v4(), &self.identity, name, target)
                    .await
                    .map_err(|_| json!({"code":"audit_unavailable"}))?,
            )
        } else {
            None
        };
        let result = self.call(name, args).await;
        if let (Some(repo), Some(sequence)) = (&self.audit, sequence) {
            let status = if result.is_ok() {
                200
            } else if !self.tool_allowed(name) {
                403
            } else {
                400
            };
            if let Err(error) = repo.finish_audit(sequence, status).await {
                eprintln!("MCP audit completion failed: {error}");
            }
        }
        result
    }
    fn allowed(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope || s == "admin")
    }
    fn tool_allowed(&self, name: &str) -> bool {
        scopes(name).iter().all(|s| self.allowed(s))
            && (!matches!(
                name,
                "rip_submit"
                    | "print_submit_pipeline"
                    | "print_commit_pipeline"
                    | "print_submit_document_inspection"
            ) || self.input_root.is_some())
    }
    async fn api(&self, request: Request<Body>) -> Result<Value, Value> {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let success = response.status().is_success();
        let no_content = response.status() == axum::http::StatusCode::NO_CONTENT;
        let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .map_err(|_| json!({"code":"response_too_large"}))?;
        if success && no_content {
            return Ok(json!({"deleted":true}));
        }
        let value =
            serde_json::from_slice(&bytes).map_err(|_| json!({"code":"invalid_api_response"}))?;
        if success { Ok(value) } else { Err(value) }
    }
    fn builder(&self, method: &str, path: &str) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {}", self.token))
    }
    async fn get(&self, path: &str) -> Result<Value, Value> {
        let request = self
            .builder("GET", path)
            .body(Body::empty())
            .map_err(|_| json!({"code":"invalid_credential"}))?;
        self.api(request).await
    }
    async fn call(&self, name: &str, args: &Value) -> Result<Value, Value> {
        eprintln!(
            "MCP identity={} operation={name} allowed={}",
            self.identity,
            self.tool_allowed(name)
        );
        if !self.tool_allowed(name) {
            return Err(
                json!({"code":"forbidden","messages":{"en":"Insufficient permission or input root unavailable","ja":"権限不足または原稿rootが未設定です","zh-CN":"权限不足或未设置原稿目录"}}),
            );
        }
        if name == "print_revoke_receipt_key" {
            let path = format!(
                "/api/v1/print/receipt-keys/{}/revoke",
                args["keyId"].as_str().unwrap()
            );
            let mut builder = self.builder("POST", &path);
            if let Some(id) = args["approvalId"].as_str() {
                builder = builder.header("x-jrip-approval-id", id);
            }
            let request = builder
                .header("content-type", "application/json")
                .body(Body::from(json!({"reason":args["reason"]}).to_string()))
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if matches!(
            name,
            "print_create_pipeline" | "print_resume_pipeline" | "print_cancel_pipeline"
        ) {
            let path = if name == "print_create_pipeline" {
                format!("/api/v1/jobs/{}/pipelines", args["jobId"].as_str().unwrap())
            } else {
                format!(
                    "/api/v1/pipelines/{}/{}",
                    args["pipelineId"].as_str().unwrap(),
                    if name == "print_resume_pipeline" {
                        "resume"
                    } else {
                        "cancel"
                    }
                )
            };
            let body = if name == "print_create_pipeline" {
                json!({"requestId":args["requestId"],"expectedRevision":args["expectedRevision"],"printerId":args["printerId"],"media":args["media"],"copies":args["copies"].as_u64().unwrap_or(1),"color":args["color"]})
            } else if name == "print_resume_pipeline" {
                json!({"expectedRevision":args["expectedRevision"],"approvalId":args["approvalId"]})
            } else {
                json!({"expectedRevision":args["expectedRevision"]})
            };
            let request = self
                .builder("POST", &path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if name == "color_apply" || name == "color_cancel_attempt" {
            let path = if name == "color_apply" {
                format!(
                    "/api/v1/jobs/{}/color-attempts",
                    args["jobId"].as_str().unwrap()
                )
            } else {
                format!(
                    "/api/v1/color/attempts/{}/cancel",
                    args["colorAttemptId"].as_str().unwrap()
                )
            };
            let body = if name == "color_apply" {
                json!({"requestId":args["requestId"],"expectedRevision":args["expectedRevision"],"inputProfileSha256":args["inputProfileSha256"],"outputProfileSha256":args["outputProfileSha256"],"policy":args["policy"]})
            } else {
                json!({"expectedRevision":args["expectedRevision"]})
            };
            let request = self
                .builder("POST", &path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if matches!(
            name,
            "print_deliver_job" | "print_preview_delivery" | "print_cancel_delivery"
        ) {
            let path = if name == "print_preview_delivery" {
                format!(
                    "/api/v1/jobs/{}/delivery-preview",
                    args["jobId"].as_str().unwrap()
                )
            } else if name == "print_deliver_job" {
                format!(
                    "/api/v1/jobs/{}/deliveries",
                    args["jobId"].as_str().unwrap()
                )
            } else {
                format!(
                    "/api/v1/deliveries/{}/cancel",
                    args["deliveryId"].as_str().unwrap()
                )
            };
            let body = if name != "print_cancel_delivery" {
                json!({"requestId":args["requestId"],"printerId":args["printerId"],"media":args["media"],"copies":args["copies"].as_u64().unwrap_or(1),"expectedRevision":args["expectedRevision"],"colorAttemptId":args["colorAttemptId"],"expectedTicketSha256":args["expectedTicketSha256"]})
            } else {
                json!({"expectedRevision":args["expectedRevision"]})
            };
            let mut builder = self.builder("POST", &path);
            if let Some(id) = args["approvalId"].as_str() {
                builder = builder.header("x-jrip-approval-id", id);
            }
            let request = builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if name == "rip_delete_job" {
            let mut builder = self.builder(
                "DELETE",
                &format!("/api/v1/jobs/{}", args["jobId"].as_str().unwrap()),
            );
            if let Some(id) = args["approvalId"].as_str() {
                builder = builder.header("x-jrip-approval-id", id);
            }
            let request = builder
                .header(
                    "x-jrip-expected-revision",
                    args["expectedRevision"].as_i64().unwrap().to_string(),
                )
                .body(Body::empty())
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if matches!(
            name,
            "rip_submit"
                | "print_submit_pipeline"
                | "print_commit_pipeline"
                | "print_submit_document_inspection"
        ) {
            let inspecting = name == "print_submit_document_inspection";
            let atomic = name == "print_submit_pipeline";
            let physical_commit = name == "print_commit_pipeline";
            let root = self.input_root.as_ref().unwrap();
            let name = args["sourceName"].as_str().unwrap();
            if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
                return Err(json!({"code":"invalid_source_name"}));
            }
            let canonical_root = tokio::fs::canonicalize(root)
                .await
                .map_err(|_| json!({"code":"input_root_unavailable"}))?;
            let path = canonical_root.join(name);
            if tokio::fs::canonicalize(&path)
                .await
                .map_err(|_| json!({"code":"source_not_found"}))?
                != path
            {
                return Err(json!({"code":"invalid_source"}));
            }
            let file = tokio::fs::File::open(&path)
                .await
                .map_err(|_| json!({"code":"source_not_found"}))?;
            let metadata = file
                .metadata()
                .await
                .map_err(|_| json!({"code":"source_not_found"}))?;
            if metadata.len() > jrip_server::MAX_DOCUMENT_BYTES as u64 {
                return Err(json!({"code":"document_too_large"}));
            }
            if !metadata.is_file() {
                return Err(json!({"code":"invalid_source"}));
            }
            let mut document = Vec::new();
            file.take(jrip_server::MAX_DOCUMENT_BYTES as u64 + 1)
                .read_to_end(&mut document)
                .await
                .map_err(|_| json!({"code":"source_read_failed"}))?;
            if document.len() > jrip_server::MAX_DOCUMENT_BYTES {
                return Err(json!({"code":"document_too_large"}));
            }
            let boundary = format!("jrip-{}", Uuid::new_v4());
            let mut body=format!("--{boundary}\r\nContent-Disposition: form-data; name=\"document\"; filename=\"source\"\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
            body.extend_from_slice(&document);
            drop(document);
            if inspecting {
                body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
                let request = self
                    .builder(
                        "POST",
                        &format!(
                            "/api/v1/print/document-inspection-requests/{}",
                            args["inspectionId"].as_str().unwrap()
                        ),
                    )
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .map_err(|_| json!({"code":"invalid_request"}))?;
                return self.api(request).await;
            }
            if atomic {
                for key in ["manifest", "pipeline"] {
                    body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"{key}\"\r\n\r\n{}",args[key]).as_bytes());
                }
                body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
                let request = self
                    .builder("POST", "/api/v1/print/submissions")
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .map_err(|_| json!({"code":"invalid_request"}))?;
                return self.api(request).await;
            }
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            let request = self
                .builder(
                    "POST",
                    &format!(
                        "/api/v1/print/{}/{}/commit",
                        if physical_commit {
                            "pipeline-preflights"
                        } else {
                            "preflights"
                        },
                        args["preflightId"].as_str().unwrap()
                    ),
                )
                .header(
                    "x-jrip-preflight-digest",
                    args["preflightDigest"].as_str().unwrap(),
                )
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if name == "print_cancel_document_inspection" {
            let request = self
                .builder(
                    "POST",
                    &format!(
                        "/api/v1/print/document-inspection-requests/{}/cancel",
                        args["inspectionId"].as_str().unwrap()
                    ),
                )
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"expectedRevision":args["expectedRevision"]}).to_string(),
                ))
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if name == "print_get_document_inspection_request" {
            let request = self
                .builder(
                    "GET",
                    &format!(
                        "/api/v1/print/document-inspection-requests/{}",
                        args["inspectionId"].as_str().unwrap()
                    ),
                )
                .body(Body::empty())
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if name == "print_get_document_inspection" {
            let request = self
                .builder(
                    "GET",
                    &format!(
                        "/api/v1/print/document-inspections/{}",
                        args["inspectionId"].as_str().unwrap()
                    ),
                )
                .body(Body::empty())
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if name == "print_create_pipeline_preflight" {
            let mut payload = args.clone();
            payload.as_object_mut().unwrap().remove("digestProfile");
            let request = self
                .builder("POST", "/api/v1/print/pipeline-preflights")
                .header(
                    "x-jrip-digest-profile",
                    args["digestProfile"]
                        .as_str()
                        .unwrap_or("org.jrip.jcs-safe-v1"),
                )
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if name == "print_validate_ticket" {
            let request = self
                .builder("POST", "/api/v1/print/validate-ticket")
                .header("content-type", "application/json")
                .body(Body::from(args["ticket"].to_string()))
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if matches!(name, "print_create_preflight" | "print_resolve_intent") {
            let request = self
                .builder(
                    "POST",
                    if name == "print_resolve_intent" {
                        "/api/v1/print/resolve"
                    } else {
                        "/api/v1/print/preflights"
                    },
                )
                .header("content-type", "application/json")
                .header(
                    "x-jrip-digest-profile",
                    args["digestProfile"]
                        .as_str()
                        .unwrap_or("org.jrip.jcs-safe-v1"),
                )
                .body(Body::from(args["intent"].to_string()))
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        let path = tool_path(name, args);
        if name == "rip_retry_job" {
            let request=self.builder("POST",&path).header("content-type","application/json")
                .body(Body::from(json!({"requestId":args["requestId"],"expectedRevision":args["expectedRevision"]}).to_string())).map_err(|_|json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if matches!(name, "rip_hold_job" | "rip_release_job" | "rip_cancel_job") {
            let request = self
                .builder("POST", &path)
                .header(
                    "x-jrip-expected-revision",
                    args["expectedRevision"].as_i64().unwrap().to_string(),
                )
                .body(Body::empty())
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        self.get(&path).await
    }
    pub async fn handle(&mut self, message: Value) -> Option<Value> {
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        if !message.is_object()
            || message["jsonrpc"] != "2.0"
            || !message["method"].is_string()
            || message
                .get("id")
                .is_some_and(|id| !(id.is_string() || id.is_number()))
        {
            return Some(error(id, -32600, "Invalid Request"));
        }
        let method = message["method"].as_str().unwrap();
        if message.get("id").is_none() {
            if method == "notifications/initialized" && self.initialized {
                self.ready = true;
            }
            return None;
        }
        let params = &message["params"];
        let result = match method {
            "initialize" if !self.initialized => {
                if !params["protocolVersion"].is_string()
                    || !params["clientInfo"].is_object()
                    || !params["capabilities"].is_object()
                {
                    return Some(error(id, -32602, "Invalid initialize params"));
                }
                self.initialized = true;
                json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{},"resources":{"subscribe":true}},"serverInfo":{"name":"jrip-mcp","version":env!("CARGO_PKG_VERSION")},"instructions":"RIP completion means TIFF export. Physical delivery and color processing are separate attempts / RIP完了はTIFF出力で、実機出力と色処理は別attemptです / RIP完成代表TIFF导出，实体投递及颜色处理为独立尝试"})
            }
            "ping" => json!({}),
            _ if !self.ready => return Some(error(id, -32000, "Initialize the session first")),
            "tools/list" => {
                if params.get("cursor").is_some() {
                    return Some(error(id, -32602, "Invalid cursor"));
                }
                json!({"tools":tools().into_iter().filter(|t|self.tool_allowed(t["name"].as_str().unwrap())).collect::<Vec<_>>()})
            }
            "tools/call" => {
                let Some(name) = params["name"].as_str() else {
                    return Some(error(id, -32602, "Tool name required"));
                };
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                if let Err(reason) = validate(name, &args) {
                    return Some(error(id, -32602, reason));
                }
                match self.audited_call(name, &args).await {
                    Ok(value) => {
                        json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
                    }
                    Err(value) => {
                        json!({"content":[{"type":"text","text":value.to_string()}],"isError":true})
                    }
                }
            }
            "resources/list" => {
                let resources = if self.allowed("jobs:read") || self.allowed("settings:write") {
                    vec![
                        json!({"uri":"jrip://approval-policy","name":"Approval policy / 承認ポリシー / 审批策略","mimeType":"application/json"}),
                    ]
                } else {
                    vec![]
                };
                json!({"resources":resources})
            }
            "resources/templates/list" => {
                let mut templates = vec![];
                for (suffix, scope, label) in [
                    ("", "jobs:read", "Job / ジョブ / 任务"),
                    ("/events", "jobs:read", "Events / 履歴 / 历史"),
                    ("/manifest", "jobs:read", "Manifest / 設定 / 设置"),
                    (
                        "/source-validation",
                        "jobs:read",
                        "Source validation / 原稿検証 / 原稿验证",
                    ),
                    ("/artifacts", "artifacts:read", "Artifacts / 成果物 / 产物"),
                ] {
                    if self.allowed(scope) {
                        templates.push(json!({"uriTemplate":format!("jrip://jobs/{{jobId}}{suffix}"),"name":label,"mimeType":"application/json"}));
                    }
                }
                if self.allowed("artifacts:read") {
                    templates.push(json!({"uriTemplate":"jrip://jobs/{jobId}/artifacts/{name}/chunks/{offset}/{length}","name":"Artifact chunk (1–32768 bytes) / 成果物の範囲取得 / 产物分块读取","mimeType":"application/octet-stream"}));
                }
                if self.allowed("jobs:read") {
                    templates.push(json!({"uriTemplate":"jrip://printers/{printerId}","name":"Printer / プリンター / 打印机","mimeType":"application/json"}));
                    templates.push(json!({"uriTemplate":"jrip://pipelines/{pipelineId}/timeline","name":"Unified timeline / 全工程履歴 / 统一阶段历史","mimeType":"application/json"}));
                }
                if self.allowed("jobs:submit") {
                    templates.push(json!({"uriTemplate":"jrip://inspections/{inspectionId}/events","name":"Inspection events / 検査履歴 / 检查历史","mimeType":"application/json"}));
                }
                json!({"resourceTemplates":templates})
            }
            "resources/read" => {
                let Some(uri) = params["uri"].as_str() else {
                    return Some(error(id, -32602, "URI required"));
                };
                if uri == "jrip://approval-policy" {
                    if !self.allowed("jobs:read") && !self.allowed("settings:write") {
                        return Some(error(id, -32001, "Insufficient permission"));
                    }
                    let value = match self.get("/api/v1/approval-policy").await {
                        Ok(value) => value,
                        Err(_) => return Some(error(id, -32002, "Approval policy unavailable")),
                    };
                    return Some(
                        json!({"jsonrpc":"2.0","id":id,"result":{"contents":[{"uri":uri,"mimeType":"application/json","text":value.to_string()}]}}),
                    );
                }
                if uri.contains("/artifacts/") {
                    let Some((job, name, offset, length)) = artifact_chunk(uri) else {
                        return Some(error(
                            id,
                            -32602,
                            "Invalid artifact chunk URI / 成果物範囲URIが不正です / 产物分块URI无效",
                        ));
                    };
                    if !self.allowed("artifacts:read") {
                        return Some(error(id, -32001, "Insufficient permission"));
                    }
                    let request = match self
                        .builder("GET", &format!("/api/v1/jobs/{job}/artifacts/{name}"))
                        .header("range", format!("bytes={offset}-{}", offset + length - 1))
                        .body(Body::empty())
                    {
                        Ok(request) => request,
                        Err(_) => return Some(error(id, -32002, "Artifact unavailable")),
                    };
                    let response = self.router.clone().oneshot(request).await.unwrap();
                    if response.status() != axum::http::StatusCode::PARTIAL_CONTENT {
                        return Some(error(
                            id,
                            -32002,
                            "Artifact unavailable, forbidden or range unsatisfiable / 成果物・権限・範囲を確認してください / 请检查产物、权限及范围",
                        ));
                    }
                    let etag = response
                        .headers()
                        .get("etag")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_owned();
                    let range = response
                        .headers()
                        .get("content-range")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_owned();
                    let bytes = match to_bytes(response.into_body(), length as usize).await {
                        Ok(bytes) => bytes,
                        Err(_) => return Some(error(id, -32002, "Artifact response too large")),
                    };
                    return Some(
                        json!({"jsonrpc":"2.0","id":id,"result":{"contents":[{"uri":uri,"mimeType":"application/octet-stream","blob":base64::engine::general_purpose::STANDARD.encode(&bytes),"_meta":{"org.jrip.artifact-chunk-v1":{"etag":etag,"contentRange":range,"bytes":bytes.len()}}}]}}),
                    );
                }
                if timeline_resource(uri).is_some() || inspection_resource(uri).is_some() {
                    if !self.allowed(event_scope(uri)) {
                        return Some(error(id, -32001, "Insufficient permission"));
                    }
                    let path = event_resource(uri).unwrap().0;
                    let value = match self.get(&path).await {
                        Ok(value) => value,
                        Err(_) => {
                            return Some(error(id, -32002, "Resource not found or forbidden"));
                        }
                    };
                    return Some(
                        json!({"jsonrpc":"2.0","id":id,"result":{"contents":[{"uri":uri,"mimeType":"application/json","text":value.to_string()}]}}),
                    );
                }
                if let Some(printer) = uri.strip_prefix("jrip://printers/") {
                    if !self.allowed("jobs:read") {
                        return Some(error(id, -32001, "Insufficient permission"));
                    }
                    if printer.is_empty()
                        || printer.len() > 128
                        || !printer
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                    {
                        return Some(error(id, -32602, "Invalid printer resource URI"));
                    }
                    let value = match self.get(&format!("/api/v1/printers/{printer}")).await {
                        Ok(value) => value,
                        Err(_) => {
                            return Some(error(id, -32002, "Printer unavailable or forbidden"));
                        }
                    };
                    return Some(
                        json!({"jsonrpc":"2.0","id":id,"result":{"contents":[{"uri":uri,"mimeType":"application/json","text":value.to_string()}]}}),
                    );
                }
                let Some((job, suffix)) = job_resource(uri) else {
                    return Some(error(id, -32602, "Invalid resource URI"));
                };
                let scope = if suffix == "artifacts" {
                    "artifacts:read"
                } else {
                    "jobs:read"
                };
                if !self.allowed(scope) {
                    return Some(error(id, -32001, "Insufficient permission"));
                }
                let path = if suffix.is_empty() || suffix == "manifest" {
                    format!("/api/v1/jobs/{job}")
                } else {
                    format!("/api/v1/jobs/{job}/{suffix}")
                };
                let mut value = match self.get(&path).await {
                    Ok(value) => value,
                    Err(_) => return Some(error(id, -32002, "Resource not found or forbidden")),
                };
                if suffix == "manifest" {
                    value = value["manifest"].clone();
                }
                json!({"contents":[{"uri":uri,"mimeType":"application/json","text":value.to_string()}]})
            }
            "resources/subscribe" | "resources/unsubscribe" => {
                let Some(uri) = params["uri"].as_str() else {
                    return Some(error(id, -32602, "URI required"));
                };
                let Some((path, _)) = event_resource(uri) else {
                    return Some(error(id, -32602, "Invalid event resource URI"));
                };
                if !self.allowed(event_scope(uri)) {
                    return Some(error(id, -32001, "Insufficient permission"));
                }
                if method == "resources/unsubscribe" {
                    self.subscriptions.remove(uri);
                    json!({})
                } else {
                    if self.get(&path).await.is_err() {
                        return Some(error(id, -32002, "Resource not found or forbidden"));
                    }
                    if self.subscriptions.len() >= MAX_SUBSCRIPTIONS
                        && !self.subscriptions.contains_key(uri)
                    {
                        return Some(error(id, -32000, "Subscription limit reached"));
                    }
                    self.subscriptions.entry(uri.to_string()).or_insert(0);
                    json!({})
                }
            }
            _ => return Some(error(id, -32601, "Method not found")),
        };
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
    pub async fn notifications(&mut self) -> Vec<Value> {
        if !self.ready {
            return vec![];
        }
        let mut notifications = Vec::new();
        for (uri, cursor) in self.subscriptions.clone() {
            if !self.allowed(event_scope(&uri)) {
                continue;
            }
            let (path, cursor_key) = event_resource(&uri).unwrap();
            if let Ok(value) = self.get(&format!("{path}?after={cursor}&limit=100")).await
                && value["events"]
                    .as_array()
                    .is_some_and(|events| !events.is_empty())
            {
                self.subscriptions
                    .insert(uri.clone(), value[cursor_key].as_i64().unwrap());
                notifications.push(json!({"jsonrpc":"2.0","method":"notifications/resources/updated","params":{"uri":uri}}));
            }
        }
        notifications
    }
}
fn job_resource(uri: &str) -> Option<(Uuid, &str)> {
    let tail = uri.strip_prefix("jrip://jobs/")?;
    let (id, suffix) = tail.split_once('/').unwrap_or((tail, ""));
    if !matches!(
        suffix,
        "" | "events" | "manifest" | "artifacts" | "source-validation"
    ) || tail.ends_with('/')
    {
        return None;
    }
    Some((Uuid::parse_str(id).ok()?, suffix))
}
fn resource_job(uri: &str) -> Option<Uuid> {
    let value = uri.strip_prefix("jrip://jobs/")?.strip_suffix("/events")?;
    Uuid::parse_str(value).ok()
}
fn timeline_resource(uri: &str) -> Option<Uuid> {
    let value = uri
        .strip_prefix("jrip://pipelines/")?
        .strip_suffix("/timeline")?;
    Uuid::parse_str(value).ok()
}
fn event_resource(uri: &str) -> Option<(String, &'static str)> {
    if let Some(id) = inspection_resource(uri) {
        Some((
            format!("/api/v1/print/document-inspection-requests/{id}/events"),
            "nextAfter",
        ))
    } else if let Some(id) = timeline_resource(uri) {
        Some((format!("/api/v1/pipelines/{id}/timeline"), "nextAfter"))
    } else {
        resource_job(uri).map(|id| (format!("/api/v1/jobs/{id}/events"), "nextCursor"))
    }
}
fn inspection_resource(uri: &str) -> Option<Uuid> {
    Uuid::parse_str(
        uri.strip_prefix("jrip://inspections/")?
            .strip_suffix("/events")?,
    )
    .ok()
}
fn event_scope(uri: &str) -> &'static str {
    if inspection_resource(uri).is_some() {
        "jobs:submit"
    } else {
        "jobs:read"
    }
}
fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn scopes(name: &str) -> Vec<&'static str> {
    match name {
        "print_submit_document_inspection"
        | "print_get_document_inspection_request"
        | "print_get_document_inspection_events"
        | "print_get_document_inspection"
        | "rip_submit"
        | "print_create_preflight"
        | "print_resolve_intent"
        | "print_validate_ticket" => vec!["jobs:submit"],
        "print_cancel_document_inspection"
        | "rip_hold_job"
        | "rip_release_job"
        | "rip_cancel_job" => vec!["jobs:control"],
        "rip_retry_job" => vec!["jobs:control", "jobs:submit"],
        "rip_list_artifacts" => vec!["artifacts:read"],
        "rip_delete_job" | "print_revoke_receipt_key" => vec!["admin"],
        "print_deliver_job" => vec!["printers:print", "artifacts:read"],
        "print_preview_delivery" => vec!["jobs:read", "artifacts:read"],
        "print_create_pipeline_preflight"
        | "print_commit_pipeline"
        | "print_submit_pipeline"
        | "print_create_pipeline"
        | "print_resume_pipeline" => vec![
            "jobs:read",
            "jobs:submit",
            "artifacts:read",
            "printers:print",
        ],
        "print_cancel_pipeline" => vec!["jobs:control", "printers:print"],
        "print_cancel_delivery" => vec!["printers:print"],
        "color_apply" => vec!["jobs:submit", "artifacts:read"],
        "color_cancel_attempt" => vec!["jobs:control"],
        _ => vec!["jobs:read"],
    }
}
fn tools() -> Vec<Value> {
    [
        ("print_cancel_document_inspection","Cancel queued PDF inspection / 待機中のPDF検査を取消 / 取消排队PDF检查"),
        ("print_submit_document_inspection","Queue PDF inspection / PDF検査を依頼 / 排队PDF检查"),
        ("print_get_document_inspection_request","Read inspection queue state / 検査の受付状態を取得 / 读取检查队列状态"),
        ("print_get_document_inspection","Read bound PDF inspection / 原稿検査結果を取得 / 读取绑定PDF检查"),
        ("print_create_pipeline_preflight","Resolve physical PrintIntent / 実機向け印刷意図を事前確認 / 预检实体打印意图"),
        ("print_commit_pipeline","Commit bound physical workflow / 事前確認済み処理連携を受付 / 提交绑定实体流程"),
        ("print_submit_pipeline","Atomically submit source and workflow / 原稿と処理連携を一括受付 / 原子提交源文档与流程"),
        ("print_create_pipeline","Register pipeline awaiting RIP completion / RIP完了を待つ処理連携を登録 / 注册等待RIP完成的流程"),
        ("print_get_pipeline_timeline","Read unified stage timeline / 全工程履歴の取得 / 获取统一阶段历史"),
        ("print_get_pipeline_events","Resume pipeline stage history / 工程履歴の差分取得 / 恢复阶段历史"),
        ("print_get_pipeline","Read pipeline state / 連携状態を取得 / 获取流程状态"),
        ("print_get_document_inspection_events","Resume inspection history / 検査履歴を差分取得 / 恢复检查历史"),
        ("print_get_pipeline_ticket","Read unified raster-to-printer contract / RIPから実機までの共通契約を取得 / 获取从RIP到设备的统一契约"),
        ("print_resume_pipeline","Resume with current authority / 権限と承認を確認して再開 / 依据当前权限与审批恢复"),
        ("print_cancel_pipeline","Cancel before delivery / 送信準備前に取消 / 投递前取消"),
        ("print_preview_delivery","Verify physical delivery ticket / 実機送信契約を確認 / 验证实体投递契约"),
        ("print_get_delivery_result_receipt","Read device terminal receipt / 実機の終端Receiptを取得 / 获取设备终态回执"),
        ("print_get_delivery_ticket","Read committed physical ticket / 実機送信契約を取得 / 获取已提交实体契约"),
        ("print_revoke_receipt_key","Permanently revoke receipt signing key / Receipt署名鍵を恒久失効 / 永久撤销回执签名密钥"),
        ("print_get_receipt_keys","Discover receipt public keys; independently pin before trust / Receipt公開鍵を取得・信頼には別経路で固定 / 获取回执公钥，信任前须独立固定"),
        ("print_get_receipt","Read acceptance receipt / 受付Receiptを取得 / 获取受理回执"),
        ("print_get_result_receipt","Read raster result receipt / ラスター結果Receiptを取得 / 获取栅格结果回执"),
        ("print_get_capabilities","Get common stage capabilities / 共通工程能力を取得 / 获取通用阶段能力"),
        ("print_get_job_ticket","Get immutable job ticket / 保存済みJobTicketを取得 / 获取持久化JobTicket"),
        ("print_validate_ticket","Validate raster ticket settings / ラスター契約設定を検証 / 验证栅格契约设置"),
        ("color_get_capabilities","Get explicit ICC capabilities / ICC処理能力取得 / 获取ICC处理能力"),
        ("color_apply","Queue explicit ICC processing / ICC色処理を予約 / 排队显式ICC处理"),
        ("color_get_attempt","Get color attempt and reports / 色処理とレポート照会 / 查询颜色尝试及报告"),
        ("color_get_events","Resume color page progress / 色処理のページ進捗 / 恢复颜色页进度"),
        ("color_cancel_attempt","Cancel queued color attempt / 待機中の色処理取消 / 取消待处理颜色尝试"),
        ("color_get_profile","Describe registered ICC / 登録ICC照会 / 查询已注册ICC"),
        ("print_get_printer","Get live printer capabilities / 実機能力照会 / 获取设备实时能力"),
        ("print_list_printers","List configured destinations / 出力先一覧 / 已配置目标"),
        ("print_get_delivery","Get delivery state / 実機投递状態 / 获取投递状态"),
        ("print_deliver_job","Queue physical output / 実機出力を予約 / 排队实体输出"),
        ("print_cancel_delivery","Cancel delivery / 実機出力取消 / 取消投递"),
        ("rip_delete_job","Delete terminal job metadata / 終端ジョブ記録削除 / 删除结束任务记录"),
        ("print_resolve_intent","Negotiate PrintIntent / 印刷意図を照合 / 协商打印意图"),
        ("rip_get_capabilities","Get RIP capabilities / RIP能力取得 / 获取RIP能力"),
        ("rip_list_jobs","List jobs / ジョブ一覧 / 作业列表"),
        ("rip_get_job","Get job / ジョブ詳細 / 获取作业"),
        ("rip_get_events","Resume state history / 状態履歴 / 状态历史"),
        ("rip_list_artifacts","List artifact metadata / 成果物一覧 / 产物列表"),
        ("rip_hold_job","Hold job / 保留 / 暂停作业"),
        ("rip_release_job","Release job / 保留解除 / 恢复作业"),
        ("rip_cancel_job","Cancel job / 取消 / 取消作业"),
        ("print_create_preflight","Create preflight / 事前確認 / 创建预检"),
        ("rip_submit","Commit preflight with inbox source / 原稿を投入 / 提交原稿"),
        ("rip_retry_job","Retry failed or cancelled job / 失敗・取消ジョブを再試行 / 重试失败或取消的作业"),
    ].into_iter().map(|(name,description)| {
        let mut properties=json!({});let mut required=vec![];
        let control=matches!(name,"rip_hold_job"|"rip_release_job"|"rip_cancel_job"|"rip_retry_job"|"rip_delete_job"|"print_deliver_job"|"print_cancel_delivery");
        if (control && name!="print_cancel_delivery") || matches!(name,"print_get_receipt"|"print_get_result_receipt"|"rip_get_job"|"rip_get_events"|"rip_list_artifacts"|"print_get_job_ticket"|"print_preview_delivery") {properties["jobId"]=json!({"type":"string","format":"uuid"});required.push("jobId");}
        if name=="print_submit_document_inspection" {properties["sourceName"]=json!({"type":"string","minLength":1});required.push("sourceName");}
        if name=="print_cancel_document_inspection" {properties["expectedRevision"]=json!({"type":"integer","minimum":0});required.push("expectedRevision");}
        if matches!(name,"print_get_document_inspection_events"|"print_cancel_document_inspection"|"print_get_document_inspection"|"print_get_document_inspection_request"|"print_submit_document_inspection") {properties["inspectionId"]=json!({"type":"string","format":"uuid"});required.push("inspectionId");}
        if name=="print_create_pipeline_preflight" {
            properties["inspectionId"]=json!({"type":"string","format":"uuid"});
            properties["requestId"]=json!({"type":"string","format":"uuid"});required.push("requestId");
            properties["intent"]=json!({"type":"object"});required.push("intent");
            properties["color"]=json!({"type":"object"});
        }
        if name=="print_create_pipeline" {
            for key in ["jobId","requestId"] {properties[key]=json!({"type":"string","format":"uuid"});required.push(key);}
            for key in ["printerId","media"] {properties[key]=json!({"type":"string","minLength":1});required.push(key);}
            properties["color"]=json!({"type":"object","description":"Explicit inputProfileSha256, optional outputProfileSha256 and policy; validated by Print API / Print APIで検証 / 由Print API验证"});
        }
        if matches!(name,"print_get_pipeline_ticket"|"print_get_pipeline_timeline"|"print_get_pipeline_events"|"print_get_pipeline"|"print_resume_pipeline"|"print_cancel_pipeline") {properties["pipelineId"]=json!({"type":"string","format":"uuid"});required.push("pipelineId");}
        if matches!(name,"print_create_pipeline"|"print_resume_pipeline"|"print_cancel_pipeline") {properties["expectedRevision"]=json!({"type":"integer","minimum":0});required.push("expectedRevision");}
        if matches!(name,"rip_delete_job"|"print_revoke_receipt_key") {properties["approvalId"]=json!({"type":"string","format":"uuid","description":"One-use approval issued by a separate administrator / 別の管理者が発行した1回限りの承認 / 另一管理员签发的一次性审批"});}
        if matches!(name,"print_deliver_job"|"print_preview_delivery"|"print_create_pipeline") {properties["copies"]=json!({"type":"integer","minimum":1,"maximum":999,"default":1,"description":"Device-side copies / 実機出力部数 / 实体输出份数"});}
        if name=="print_resume_pipeline" {properties["approvalId"]=json!({"type":"string","format":"uuid"});}
        if name=="print_validate_ticket" {properties["ticket"]=json!({"type":"object","description":"Versioned JobTicket; deeply validated by Print API / Print APIで検証 / 由Print API验证"});required.push("ticket");}
        if control || name=="print_preview_delivery" {properties["expectedRevision"]=json!({"type":"integer","minimum":0});required.push("expectedRevision");}
        if matches!(name,"rip_retry_job"|"print_deliver_job"|"print_preview_delivery") {properties["requestId"]=json!({"type":"string","format":"uuid"});required.push("requestId");}
        if matches!(name,"print_create_preflight"|"print_create_pipeline_preflight") {properties["digestProfile"]=json!({"type":"string","enum":["org.jrip.jcs-safe-v1","org.jrip.serde-json-v1"]});}
        if matches!(name,"print_create_preflight"|"print_resolve_intent") {properties["intent"]=json!({"type":"object","description":"ACX PrintIntent; validated by Print API / Print APIで検証 / 由Print API验证"});required.push("intent");}
        if name=="print_get_printer" {properties["printerId"]=json!({"type":"string","minLength":1});required.push("printerId");}
        if name=="print_revoke_receipt_key" {properties["keyId"]=json!({"type":"string","pattern":"^sha256:[0-9a-f]{64}$"});properties["reason"]=json!({"type":"string","enum":["compromised","retired","operator-request"]});required.extend(["keyId","reason"]);}
        if name=="print_get_receipt_keys" {properties["after"]=json!({"type":"string","pattern":"^sha256:[0-9a-f]{64}$"});properties["limit"]=json!({"type":"integer","minimum":1,"maximum":100});}
        if name=="print_get_capabilities" {properties["printerId"]=json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_-]+$"});}
        if matches!(name,"print_get_delivery_result_receipt"|"print_get_delivery"|"print_cancel_delivery"|"print_get_delivery_ticket") {
            properties["deliveryId"]=json!({"type":"string","format":"uuid"});required.push("deliveryId");
        }
        if matches!(name,"print_deliver_job"|"print_preview_delivery") {
            properties["expectedTicketSha256"]=json!({"type":"string","format":"sha256","pattern":"^[0-9a-f]{64}$"});
            properties["approvalId"]=json!({"type":"string","format":"uuid"});
            properties["colorAttemptId"]=json!({"type":"string","format":"uuid"});
            for key in ["printerId","media"] {properties[key]=json!({"type":"string","minLength":1});required.push(key);}
        }
        if name=="color_apply" {
            for key in ["jobId","requestId"] {properties[key]=json!({"type":"string","format":"uuid"});required.push(key);}
            properties["expectedRevision"]=json!({"type":"integer","minimum":0});required.push("expectedRevision");
            for key in ["inputProfileSha256","outputProfileSha256"] {properties[key]=json!({"type":"string","format":"sha256","pattern":"^[0-9a-f]{64}$"});}required.push("inputProfileSha256");
            properties["policy"]=json!({"type":"object","additionalProperties":false,"required":["schemaVersion","inputColor","outputColor","intent","blackPointCompensation","blackPreservation"],"properties":{"schemaVersion":{"type":"integer","const":1},"inputColor":{"type":"string","enum":["rgb","cmyk","gray"]},"outputColor":{"type":"string","enum":["rgb","cmyk","gray"]},"intent":{"type":"string","enum":["perceptual","relative_colorimetric","absolute_colorimetric","saturation"]},"blackPointCompensation":{"type":"boolean"},"blackPreservation":{"type":"string","enum":["none","k_only","k_plane"]},"tacPercent":{"type":["integer","null"],"minimum":100,"maximum":400}},"description":"CMYK preservation excludes absolute intent; DeviceLink uses header intent and no extra BPC / DeviceLinkはヘッダーの意図を使い追加BPCを禁止 / DeviceLink使用头部意图且禁止额外BPC"});required.push("policy");
        }
        if matches!(name,"color_get_attempt"|"color_get_events"|"color_cancel_attempt") {properties["colorAttemptId"]=json!({"type":"string","format":"uuid"});required.push("colorAttemptId");}
        if name=="color_cancel_attempt" {properties["expectedRevision"]=json!({"type":"integer","minimum":0});required.push("expectedRevision");}
        if name=="color_get_profile" {properties["profileSha256"]=json!({"type":"string","format":"sha256","pattern":"^[0-9a-f]{64}$"});required.push("profileSha256");}
        if name=="print_submit_pipeline" {
            properties["sourceName"]=json!({"type":"string","minLength":1});required.push("sourceName");
            for key in ["manifest","pipeline"] {properties[key]=json!({"type":"object","description":"Validated by Print API / Print APIで検証 / 由Print API验证"});required.push(key);}
        }
        if matches!(name,"rip_submit"|"print_commit_pipeline") {
            for (key,format) in [("preflightId","uuid"),("preflightDigest",""),("sourceName","")] {
                properties[key]=json!({"type":"string","minLength":1});if format=="uuid" {properties[key]["format"]=json!(format);}required.push(key);
            }
        }
        if matches!(name,"print_get_document_inspection_events"|"print_get_pipeline_timeline"|"print_get_pipeline_events"|"rip_list_jobs"|"rip_get_events"|"color_get_events") {
            properties["limit"]=json!({"type":"integer","minimum":1,"maximum":100,"default":50});
            properties[if name=="rip_list_jobs" {"offset"}else{"after"}]=json!({"type":"integer","minimum":0,"default":0});
        }
        let modifying=control||matches!(name,"print_revoke_receipt_key"|"print_cancel_document_inspection"|"print_submit_document_inspection"|"print_create_pipeline_preflight"|"print_commit_pipeline"|"print_submit_pipeline"|"print_create_pipeline"|"print_resume_pipeline"|"print_cancel_pipeline"|"print_create_preflight"|"print_resolve_intent"|"rip_submit"|"color_apply"|"color_cancel_attempt");
        json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":!modifying,"destructiveHint":control||matches!(name,"print_revoke_receipt_key"|"print_cancel_document_inspection"|"color_cancel_attempt"|"print_cancel_pipeline"),"idempotentHint":matches!(name,"print_revoke_receipt_key"|"print_submit_document_inspection"|"print_commit_pipeline"|"print_submit_pipeline"|"print_create_pipeline"|"rip_submit"|"rip_retry_job"|"print_deliver_job"|"color_apply")||!modifying,"openWorldHint":false}})
    }).collect()
}
fn validate(name: &str, args: &Value) -> Result<(), &'static str> {
    let tool = tools()
        .into_iter()
        .find(|t| t["name"] == name)
        .ok_or("Unknown tool")?;
    let object = args.as_object().ok_or("Arguments must be an object")?;
    let props = tool["inputSchema"]["properties"].as_object().unwrap();
    for required in tool["inputSchema"]["required"].as_array().unwrap() {
        if !object.contains_key(required.as_str().unwrap()) {
            return Err("Required argument missing");
        }
    }
    for (key, value) in object {
        let property = props.get(key).ok_or("Unknown argument")?;
        if let Some(values) = property["enum"].as_array()
            && !values.contains(value)
        {
            return Err("Argument outside enum");
        }
        match property["type"].as_str().unwrap() {
            "integer" => {
                let n = value.as_i64().ok_or("Expected integer")?;
                if n < property["minimum"].as_i64().unwrap_or(0)
                    || property["maximum"].as_i64().is_some_and(|max| n > max)
                {
                    return Err("Argument out of range");
                }
            }
            "string" => {
                let s = value.as_str().ok_or("Expected string")?;
                if property["pattern"] == "^sha256:[0-9a-f]{64}$"
                    && (s.len() != 71
                        || !s.starts_with("sha256:")
                        || !s.as_bytes()[7..]
                            .iter()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b)))
                {
                    return Err("Invalid receipt key cursor");
                }
                if key == "printerId"
                    && (s.len() > 128
                        || !s
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')))
                {
                    return Err("Invalid printer identifier");
                }
                if s.is_empty() || property["format"] == "uuid" && Uuid::parse_str(s).is_err() {
                    return Err("Invalid string argument");
                }
                if property["format"] == "sha256"
                    && (s.len() != 64
                        || !s
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
                {
                    return Err("Invalid profile digest");
                }
            }
            "object" if !value.is_object() => return Err("Expected object"),
            _ => {}
        }
    }
    Ok(())
}
fn tool_path(name: &str, args: &Value) -> String {
    let id = args["jobId"].as_str().unwrap_or("");
    match name {
        "color_get_capabilities" => "/api/v1/color/capabilities".into(),
        "color_get_attempt" => format!(
            "/api/v1/color/attempts/{}",
            args["colorAttemptId"].as_str().unwrap()
        ),
        "color_get_events" => format!(
            "/api/v1/color/attempts/{}/events?after={}&limit={}",
            args["colorAttemptId"].as_str().unwrap(),
            args["after"].as_i64().unwrap_or(0),
            args["limit"].as_i64().unwrap_or(50)
        ),
        "color_get_profile" => format!(
            "/api/v1/color/profiles/{}",
            args["profileSha256"].as_str().unwrap()
        ),
        "rip_get_capabilities" => "/.well-known/acx.json".into(),
        "print_get_receipt" => format!("/api/v1/jobs/{id}/receipt"),
        "print_get_result_receipt" => format!("/api/v1/jobs/{id}/result-receipt"),
        "print_get_receipt_keys" => format!(
            "/api/v1/print/receipt-keys?after={}&limit={}",
            args["after"].as_str().unwrap_or(""),
            args["limit"].as_i64().unwrap_or(50)
        ),
        "print_get_capabilities" => args["printerId"]
            .as_str()
            .map(|id| format!("/api/v1/print/capabilities?printerId={id}"))
            .unwrap_or_else(|| "/api/v1/print/capabilities".into()),
        "print_get_job_ticket" => format!("/api/v1/jobs/{id}/ticket"),
        "print_list_printers" => "/api/v1/printers".into(),
        "print_get_printer" => format!("/api/v1/printers/{}", args["printerId"].as_str().unwrap()),
        "print_get_delivery" => format!(
            "/api/v1/deliveries/{}",
            args["deliveryId"].as_str().unwrap()
        ),
        "print_get_pipeline_timeline" => format!(
            "/api/v1/pipelines/{}/timeline?after={}&limit={}",
            args["pipelineId"].as_str().unwrap(),
            args["after"].as_i64().unwrap_or(0),
            args["limit"].as_i64().unwrap_or(50)
        ),
        "print_get_pipeline_events" => format!(
            "/api/v1/pipelines/{}/events?after={}&limit={}",
            args["pipelineId"].as_str().unwrap(),
            args["after"].as_i64().unwrap_or(0),
            args["limit"].as_i64().unwrap_or(50)
        ),
        "print_get_pipeline" => {
            format!("/api/v1/pipelines/{}", args["pipelineId"].as_str().unwrap())
        }
        "print_get_document_inspection_events" => format!(
            "/api/v1/print/document-inspection-requests/{}/events?after={}&limit={}",
            args["inspectionId"].as_str().unwrap(),
            args["after"].as_i64().unwrap_or(0),
            args["limit"].as_i64().unwrap_or(50)
        ),
        "print_get_pipeline_ticket" => {
            format!(
                "/api/v1/pipelines/{}/ticket",
                args["pipelineId"].as_str().unwrap()
            )
        }
        "print_get_delivery_result_receipt" => format!(
            "/api/v1/deliveries/{}/result-receipt",
            args["deliveryId"].as_str().unwrap()
        ),
        "print_get_delivery_ticket" => format!(
            "/api/v1/deliveries/{}/ticket",
            args["deliveryId"].as_str().unwrap()
        ),
        "rip_list_jobs" => format!(
            "/api/v1/jobs?limit={}&offset={}",
            args["limit"].as_i64().unwrap_or(50),
            args["offset"].as_i64().unwrap_or(0)
        ),
        "rip_get_events" => format!(
            "/api/v1/jobs/{id}/events?after={}&limit={}",
            args["after"].as_i64().unwrap_or(0),
            args["limit"].as_i64().unwrap_or(50)
        ),
        "rip_list_artifacts" => format!("/api/v1/jobs/{id}/artifacts"),
        "rip_get_job" => format!("/api/v1/jobs/{id}"),
        _ => format!(
            "/api/v1/jobs/{id}/{}",
            match name {
                "rip_hold_job" => "hold",
                "rip_release_job" => "release",
                "rip_cancel_job" => "cancel",
                _ => "retry",
            }
        ),
    }
}
async fn read_message<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let newline = chunk.iter().position(|b| *b == b'\n');
        let count = newline.map_or(chunk.len(), |p| p + 1);
        if line.len() + count > MAX_MESSAGE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "MCP message exceeds 64 KiB",
            ));
        }
        line.extend_from_slice(&chunk[..count]);
        reader.consume(count);
        if newline.is_some() {
            return Ok(Some(line));
        }
    }
}
async fn write_message<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    message: Value,
) -> std::io::Result<()> {
    writer.write_all(message.to_string().as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}
pub async fn serve<R: tokio::io::AsyncBufRead + Unpin, W: tokio::io::AsyncWrite + Unpin>(
    mut reader: R,
    mut writer: W,
    mut session: Session,
) -> std::io::Result<()> {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let pending = read_message(&mut reader);
        tokio::pin!(pending);
        let line = loop {
            tokio::select! {
                line=&mut pending=>break line?,
                _=interval.tick()=>{for notification in session.notifications().await {write_message(&mut writer,notification).await?;}}
            }
        };
        let Some(line) = line else {
            return Ok(());
        };
        let reply = match serde_json::from_slice(&line) {
            Ok(message) => session.handle(message).await,
            Err(_) => Some(error(Value::Null, -32700, "Parse error")),
        };
        if let Some(reply) = reply {
            write_message(&mut writer, reply).await?;
        }
    }
}

/// Canonical decimal ranges and fixed TIFF names prevent path/query injection.
fn artifact_chunk(uri: &str) -> Option<(Uuid, &str, u64, u64)> {
    let parts: Vec<_> = uri.strip_prefix("jrip://jobs/")?.split('/').collect();
    if parts.len() != 6 || parts[1] != "artifacts" || parts[3] != "chunks" {
        return None;
    }
    let job = Uuid::parse_str(parts[0]).ok()?;
    let name = parts[2];
    let page = name.strip_prefix("page-")?.strip_suffix(".tiff")?;
    if page.len() != 6
        || !page.bytes().all(|b| b.is_ascii_digit())
        || page.parse::<u32>().ok()? == 0
    {
        return None;
    }
    let offset = parts[4].parse::<u64>().ok()?;
    let length = parts[5].parse::<u64>().ok()?;
    if parts[4] != offset.to_string()
        || parts[5] != length.to_string()
        || !(1..=32768).contains(&length)
        || offset.checked_add(length).is_none()
    {
        return None;
    }
    Some((job, name, offset, length))
}
