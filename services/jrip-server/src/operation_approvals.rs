//! Bind approval to a server-generated, reviewable effect, never a client description.
use super::*;
use axum::{
    body::{Body, to_bytes},
    extract::{MatchedPath, Request},
    middleware::Next,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Grant {
    binding: Value,
    reason: String,
}
pub(crate) async fn issue(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<Grant>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&app, &headers, "admin")?;
    let issuer = pipeline::actor(&app, &headers)?;
    let identity = req.binding["identity"]
        .as_str()
        .ok_or(bad("invalid_approval"))?;
    if identity == issuer
        || !app.credentials.iter().any(|c| c.identity == identity)
        || req.binding["profile"] != "org.jrip.operation-binding-v1"
        || !matches!(
            req.binding["operation"].as_str(),
            Some(
                "job-delete"
                    | "icc-register"
                    | "color-policy-create"
                    | "retention-extend"
                    | "receipt-key-revoke"
            )
        )
        || req.reason.trim().is_empty()
        || req.reason.len() > 1024
        || req.reason.chars().any(char::is_control)
        || serde_json::to_vec(&req.binding).map_err(internal)?.len() > 24576
    {
        return Err(bad("invalid_approval"));
    }
    Ok((
        StatusCode::CREATED,
        Json(
            app.repo
                .issue_operation_approval(&req.binding, &issuer, &req.reason)
                .await?,
        ),
    ))
}
pub(crate) async fn revoke(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    authorize(&app, &headers, "admin")?;
    app.repo
        .revoke_operation_approval(id, &pipeline::actor(&app, &headers)?)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(crate) async fn policy(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    pipeline::actor(&app, &headers)?;
    Ok(Json(
        json!({"profile":"org.jrip.operation-approval-policy-v1","deleteApprovalRequired":app.require_delete_approval,"settingsApprovalRequired":app.require_settings_approval,"lifetimeSeconds":300,"singleUse":true,"separateIssuerIdentity":true,"operations":["job-delete","icc-register","color-policy-create","retention-extend","receipt-key-revoke"],"grantEndpoint":"/api/v1/operation-approvals","approvalHeader":"x-jrip-approval-id"}),
    ))
}
pub(crate) async fn enforce(State(app): State<App>, request: Request, next: Next) -> Response {
    match guard(&app, request).await {
        Ok(request) => next.run(request).await,
        Err(response) => *response,
    }
}
async fn guard(app: &App, request: Request) -> Result<Request, Box<Response>> {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str())
        .unwrap_or("");
    let method = request.method().as_str();
    let operation = match (method, route) {
        ("DELETE", "/api/v1/jobs/{id}") if app.require_delete_approval => "job-delete",
        ("POST", "/api/v1/color/profiles") if app.require_settings_approval => "icc-register",
        ("POST", "/api/v1/color/policies") if app.require_settings_approval => {
            "color-policy-create"
        }
        ("PUT", "/api/v1/jobs/{id}/retention") if app.require_settings_approval => {
            "retention-extend"
        }
        ("POST", "/api/v1/print/receipt-keys/{key_id}/revoke") if app.require_settings_approval => {
            "receipt-key-revoke"
        }
        _ => return Ok(request),
    };
    authorize(
        app,
        request.headers(),
        if matches!(operation, "job-delete" | "receipt-key-revoke") {
            "admin"
        } else {
            "settings:write"
        },
    )
    .map_err(IntoResponse::into_response)?;
    let identity = pipeline::actor(app, request.headers()).map_err(IntoResponse::into_response)?;
    let (mut parts, body) = request.into_parts();
    let limit = match operation {
        "icc-register" => 4 * 1024 * 1024,
        "color-policy-create" => 16384,
        "job-delete" => 0,
        _ => 4096,
    };
    let bytes = to_bytes(body, limit)
        .await
        .map_err(|_| ApiError(StatusCode::PAYLOAD_TOO_LARGE, "invalid_approval").into_response())?;
    let path = parts.uri.path().to_owned();
    let value = if matches!(operation, "icc-register" | "job-delete") {
        Value::Null
    } else {
        rip_digest::parse(&bytes).map_err(|_| bad("invalid_approval").into_response())?
    };
    let effect = effects(app, operation, &path, &parts.headers, &value, &bytes)
        .await
        .map_err(IntoResponse::into_response)?;
    let request_digest = if operation == "icc-register" {
        format!("sha256:{:x}", Sha256::digest(&bytes))
    } else {
        rip_digest::digest(&value).map_err(|_| bad("invalid_approval").into_response())?
    };
    let binding = json!({"profile":"org.jrip.operation-binding-v1","identity":identity,"operation":operation,"method":parts.method.as_str(),"path":path,"requestDigest":request_digest,"effect":effect});
    let approved = if let Some(id) = parts
        .headers
        .get("x-jrip-approval-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
    {
        app.repo
            .consume_operation_approval(id, &binding, &identity)
            .await
            .map_err(|e| ApiError::from(e).into_response())?
    } else {
        false
    };
    if !approved {
        return Err(Box::new((StatusCode::FORBIDDEN, Json(json!({"error":{"code":"approval_required","messages":{"en":"A separate administrator must approve this exact operation","ja":"この操作内容に対する別の管理者の承認が必要です","zh-CN":"此操作需要另一管理员审批"},"retryable":false,"approvalRequest":{"binding":binding,"grantEndpoint":"/api/v1/operation-approvals","requiredScope":"admin","separateIssuerIdentity":true,"expiresInSeconds":300,"singleUse":true},"recovery":{"en":"Review the effect, obtain approval, then retry the identical request with x-jrip-approval-id","ja":"影響を確認して承認を取得し、同じ要求にx-jrip-approval-idを付けて再実行してください","zh-CN":"核对影响并取得审批后，携带x-jrip-approval-id重试相同请求"}}}))).into_response()));
    }
    // Bind optional legacy delete revision to the exact revision reviewed at admission.
    if operation == "job-delete" {
        parts.headers.insert(
            "x-jrip-expected-revision",
            binding["effect"]["before"]["revision"]
                .as_i64()
                .unwrap()
                .to_string()
                .parse()
                .unwrap(),
        );
    }
    Ok(Request::from_parts(parts, Body::from(bytes)))
}
async fn effects(
    app: &App,
    operation: &str,
    path: &str,
    headers: &HeaderMap,
    value: &Value,
    bytes: &[u8],
) -> Result<Value, ApiError> {
    match operation {
        "job-delete" => {
            let id = path
                .rsplit('/')
                .next()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or(bad("invalid_approval"))?;
            let job = app.repo.get(id).await?;
            if !job.state.terminal() {
                return Err(ApiError(StatusCode::CONFLICT, "state_conflict"));
            }
            if let Some(rev) = headers.get("x-jrip-expected-revision") {
                let rev = rev
                    .to_str()
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .filter(|n| *n >= 0)
                    .ok_or(bad("invalid_revision"))?;
                if rev != job.revision {
                    return Err(ApiError(StatusCode::CONFLICT, "state_conflict"));
                }
            }
            Ok(
                json!({"before":{"jobId":id,"revision":job.revision,"state":job.state,"sourceSha256":job.source_sha256},"after":null,"impact":{"en":"Delete job metadata; files and historical evidence remain until retention maintenance","ja":"ジョブ記録を削除します。ファイルと履歴証跡は保持期限管理に従い保持します","zh-CN":"删除任务元数据；文件及历史证据按保留规则保存"}}),
            )
        }
        "retention-extend" => {
            let parsed: crate::retention::Extension = serde_json::from_value(value.clone())
                .map_err(|_| bad("invalid_artifact_retention"))?;
            crate::retention::validate_extension(&parsed)?;
            let id = path
                .split('/')
                .nth(4)
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or(bad("invalid_approval"))?;
            let before = app.repo.artifact_retention_policy(id).await?;
            let until = value["retainUntil"]
                .as_i64()
                .ok_or(bad("invalid_artifact_retention"))?;
            if value["expectedRevision"] != before["revision"] {
                return Err(ApiError(StatusCode::CONFLICT, "state_conflict"));
            }
            if until <= rip_storage::now()
                || until > rip_storage::now() + 3650 * 86400
                || before["retainUntil"]
                    .as_i64()
                    .is_some_and(|old| until < old)
            {
                return Err(bad("invalid_artifact_retention"));
            }
            Ok(
                json!({"before":before,"after":value,"impact":{"en":"Extend minimum retention of the local job artifact family","ja":"ローカルのジョブ関連成果物の最低保持期限を延長します","zh-CN":"延长本地任务相关产物的最低保留期限"}}),
            )
        }
        "icc-register" => {
            // Validate using the same parser without publishing registry content.
            rip_color::validate_profile(bytes).map_err(|_| bad("invalid_icc_profile"))?;
            let hash = format!("{:x}", Sha256::digest(bytes));
            let registry = rip_color::registry::Registry::open(app.root.join("color-profiles"))
                .map_err(internal)?;
            let before = registry
                .describe(&hash)
                .ok()
                .map(serde_json::to_value)
                .transpose()
                .map_err(internal)?;
            Ok(
                json!({"before":before,"after":{"sha256":hash,"bytes":bytes.len()},"impact":{"en":"Register an immutable ICC profile; defaults are unchanged","ja":"不変ICC profileを登録します。既定値は変更しません","zh-CN":"注册不可变ICC profile，默认值保持不变"}}),
            )
        }
        "color-policy-create" => {
            let parsed: crate::color::policies::Save = serde_json::from_value(value.clone())
                .map_err(|_| bad("invalid_color_policy_definition"))?;
            crate::color::policies::validate_save(app, &parsed)?;
            let definition = &value["definition"];
            let digest = rip_digest::digest(definition)
                .map_err(|_| bad("invalid_color_policy_definition"))?;
            let hash = digest.strip_prefix("sha256:").unwrap();
            let before = match app.repo.color_policy(hash).await {
                Ok(v) => Some(v),
                Err(StorageError::NotFound) => None,
                Err(e) => return Err(e.into()),
            };
            Ok(
                json!({"before":before,"after":value,"digest":digest,"impact":{"en":"Register an immutable color policy; defaults are unchanged","ja":"不変Color Policyを登録します。既定値は変更しません","zh-CN":"注册不可变颜色策略，默认值保持不变"}}),
            )
        }
        "receipt-key-revoke" => {
            if value.as_object().is_none_or(|v| v.len() != 1)
                || !matches!(
                    value["reason"].as_str(),
                    Some("compromised" | "retired" | "operator-request")
                )
            {
                return Err(bad("invalid_receipt_key_revocation"));
            }
            let key = path
                .split('/')
                .nth(5)
                .ok_or(bad("invalid_receipt_key_revocation"))?;
            let before = app.repo.receipt_key_approval_state(key).await?;
            Ok(
                json!({"before":before,"after":{"keyId":key,"revoked":true,"reason":value["reason"]},"impact":{"en":"Revoke trust in this Receipt signing key; new signatures using it will fail","ja":"Receipt署名鍵の信頼を失効させます。この鍵での新規署名は失敗します","zh-CN":"撤销此Receipt签名密钥的信任；使用该密钥的新签名将失败"}}),
            )
        }
        _ => Err(bad("invalid_approval")),
    }
}
