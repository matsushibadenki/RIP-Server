use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;
use uuid::Uuid;
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
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
            && (name != "rip_submit" || self.input_root.is_some())
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
        if matches!(name, "print_deliver_job" | "print_cancel_delivery") {
            let path = if name == "print_deliver_job" {
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
            let body = if name == "print_deliver_job" {
                json!({"requestId":args["requestId"],"printerId":args["printerId"],"media":args["media"],"expectedRevision":args["expectedRevision"],"colorAttemptId":args["colorAttemptId"]})
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
            let request = self
                .builder(
                    "DELETE",
                    &format!("/api/v1/jobs/{}", args["jobId"].as_str().unwrap()),
                )
                .header(
                    "x-jrip-expected-revision",
                    args["expectedRevision"].as_i64().unwrap().to_string(),
                )
                .body(Body::empty())
                .map_err(|_| json!({"code":"invalid_request"}))?;
            return self.api(request).await;
        }
        if name == "rip_submit" {
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
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            let request = self
                .builder(
                    "POST",
                    &format!(
                        "/api/v1/print/preflights/{}/commit",
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
            "resources/list" => json!({"resources":[]}),
            "resources/templates/list" => {
                let mut templates = vec![];
                for (suffix, scope, label) in [
                    ("", "jobs:read", "Job / ジョブ / 任务"),
                    ("/events", "jobs:read", "Events / 履歴 / 历史"),
                    ("/manifest", "jobs:read", "Manifest / 設定 / 设置"),
                    ("/artifacts", "artifacts:read", "Artifacts / 成果物 / 产物"),
                ] {
                    if self.allowed(scope) {
                        templates.push(json!({"uriTemplate":format!("jrip://jobs/{{jobId}}{suffix}"),"name":label,"mimeType":"application/json"}));
                    }
                }
                if self.allowed("jobs:read") {
                    templates.push(json!({"uriTemplate":"jrip://printers/{printerId}","name":"Printer / プリンター / 打印机","mimeType":"application/json"}));
                }
                json!({"resourceTemplates":templates})
            }
            "resources/read" => {
                let Some(uri) = params["uri"].as_str() else {
                    return Some(error(id, -32602, "URI required"));
                };
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
                if !self.allowed("jobs:read") {
                    return Some(error(id, -32001, "Insufficient permission"));
                }
                let Some(uri) = params["uri"].as_str() else {
                    return Some(error(id, -32602, "URI required"));
                };
                let Some(job) = resource_job(uri) else {
                    return Some(error(id, -32602, "Invalid event resource URI"));
                };
                if method == "resources/unsubscribe" {
                    self.subscriptions.remove(uri);
                    json!({})
                } else {
                    let value = match self.get(&format!("/api/v1/jobs/{job}/events")).await {
                        Ok(v) => v,
                        Err(_) => {
                            return Some(error(id, -32002, "Resource not found or forbidden"));
                        }
                    };
                    if method == "resources/read" {
                        json!({"contents":[{"uri":uri,"mimeType":"application/json","text":value.to_string()}]})
                    } else {
                        if self.subscriptions.len() >= MAX_SUBSCRIPTIONS
                            && !self.subscriptions.contains_key(uri)
                        {
                            return Some(error(id, -32000, "Subscription limit reached"));
                        }
                        self.subscriptions.entry(uri.to_string()).or_insert(0);
                        json!({})
                    }
                }
            }
            _ => return Some(error(id, -32601, "Method not found")),
        };
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
    pub async fn notifications(&mut self) -> Vec<Value> {
        if !self.ready || !self.allowed("jobs:read") {
            return vec![];
        }
        let mut notifications = Vec::new();
        for (uri, cursor) in self.subscriptions.clone() {
            let job = resource_job(&uri).unwrap();
            if let Ok(value) = self
                .get(&format!(
                    "/api/v1/jobs/{job}/events?after={cursor}&limit=100"
                ))
                .await
                && value["events"]
                    .as_array()
                    .is_some_and(|events| !events.is_empty())
            {
                self.subscriptions
                    .insert(uri.clone(), value["nextCursor"].as_i64().unwrap());
                notifications.push(json!({"jsonrpc":"2.0","method":"notifications/resources/updated","params":{"uri":uri}}));
            }
        }
        notifications
    }
}
fn job_resource(uri: &str) -> Option<(Uuid, &str)> {
    let tail = uri.strip_prefix("jrip://jobs/")?;
    let (id, suffix) = tail.split_once('/').unwrap_or((tail, ""));
    if !matches!(suffix, "" | "events" | "manifest" | "artifacts") || tail.ends_with('/') {
        return None;
    }
    Some((Uuid::parse_str(id).ok()?, suffix))
}
fn resource_job(uri: &str) -> Option<Uuid> {
    let value = uri.strip_prefix("jrip://jobs/")?.strip_suffix("/events")?;
    Uuid::parse_str(value).ok()
}
fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn scopes(name: &str) -> Vec<&'static str> {
    match name {
        "rip_submit" | "print_create_preflight" | "print_resolve_intent" => vec!["jobs:submit"],
        "rip_hold_job" | "rip_release_job" | "rip_cancel_job" => vec!["jobs:control"],
        "rip_retry_job" => vec!["jobs:control", "jobs:submit"],
        "rip_list_artifacts" => vec!["artifacts:read"],
        "rip_delete_job" => vec!["admin"],
        "print_deliver_job" => vec!["printers:print", "artifacts:read"],
        "print_cancel_delivery" => vec!["printers:print"],
        "color_apply" => vec!["jobs:submit", "artifacts:read"],
        "color_cancel_attempt" => vec!["jobs:control"],
        _ => vec!["jobs:read"],
    }
}
fn tools() -> Vec<Value> {
    [
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
        if (control && name!="print_cancel_delivery") || matches!(name,"rip_get_job"|"rip_get_events"|"rip_list_artifacts") {properties["jobId"]=json!({"type":"string","format":"uuid"});required.push("jobId");}
        if control {properties["expectedRevision"]=json!({"type":"integer","minimum":0});required.push("expectedRevision");}
        if matches!(name,"rip_retry_job"|"print_deliver_job") {properties["requestId"]=json!({"type":"string","format":"uuid"});required.push("requestId");}
        if matches!(name,"print_create_preflight"|"print_resolve_intent") {properties["intent"]=json!({"type":"object","description":"ACX PrintIntent; validated by Print API / Print APIで検証 / 由Print API验证"});required.push("intent");}
        if name=="print_get_printer" {properties["printerId"]=json!({"type":"string","minLength":1});required.push("printerId");}
        if matches!(name,"print_get_delivery"|"print_cancel_delivery") {
            properties["deliveryId"]=json!({"type":"string","format":"uuid"});required.push("deliveryId");
        }
        if name=="print_deliver_job" {
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
        if name=="rip_submit" {
            for (key,format) in [("preflightId","uuid"),("preflightDigest",""),("sourceName","")] {
                properties[key]=json!({"type":"string","minLength":1});if format=="uuid" {properties[key]["format"]=json!(format);}required.push(key);
            }
        }
        if matches!(name,"rip_list_jobs"|"rip_get_events"|"color_get_events") {
            properties["limit"]=json!({"type":"integer","minimum":1,"maximum":100,"default":50});
            properties[if name=="rip_list_jobs" {"offset"}else{"after"}]=json!({"type":"integer","minimum":0,"default":0});
        }
        let modifying=control||matches!(name,"print_create_preflight"|"print_resolve_intent"|"rip_submit"|"color_apply"|"color_cancel_attempt");
        json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":!modifying,"destructiveHint":control||name=="color_cancel_attempt","idempotentHint":matches!(name,"rip_submit"|"rip_retry_job"|"print_deliver_job"|"color_apply")||!modifying,"openWorldHint":false}})
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
        "print_list_printers" => "/api/v1/printers".into(),
        "print_get_printer" => format!("/api/v1/printers/{}", args["printerId"].as_str().unwrap()),
        "print_get_delivery" => format!(
            "/api/v1/deliveries/{}",
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
