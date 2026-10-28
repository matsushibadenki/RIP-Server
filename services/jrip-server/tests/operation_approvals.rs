use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
const AGENT: &str = "agent-admin-token-at-least-24-chars";
const HUMAN: &str = "human-admin-token-at-least-24-chars";
const WRITER: &str = "settings-writer-token-at-least-24-chars";
const READER: &str = "reader-token-at-least-24-characters";
async fn app() -> (tempfile::TempDir, jrip_server::App) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), String::new())
        .await
        .unwrap();
    app.require_delete_approval = true;
    app.require_settings_approval = true;
    app.credentials = vec![
        (AGENT, "agent", vec!["admin"]),
        (HUMAN, "human", vec!["admin"]),
        (WRITER, "writer", vec!["settings:write"]),
        (READER, "reader", vec!["jobs:read"]),
    ]
    .into_iter()
    .map(|(token, identity, scopes)| jrip_server::Credential {
        validity: None,
        token: token.into(),
        identity: identity.into(),
        scopes: scopes.into_iter().map(Into::into).collect(),
    })
    .collect();
    (dir, app)
}
async fn seed(app: &jrip_server::App) -> rip_core::Job {
    let job = rip_core::Job {
        id: Uuid::new_v4(),
        manifest: serde_json::from_value(json!({"name":"source.pdf","format":"application/pdf"}))
            .unwrap(),
        state: rip_core::JobState::Completed,
        source_sha256: "a".repeat(64),
        created_at: 0,
        updated_at: 0,
        revision: 0,
        error_code: None,
        selected_engine: None,
        worker_run_id: None,
    };
    app.repo.insert(&job).await.unwrap();
    job
}
async fn call(
    router: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Vec<u8>,
    approval: Option<&str>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json");
    if let Some(id) = approval {
        req = req.header("x-jrip-approval-id", id);
    }
    let response = router
        .clone()
        .oneshot(req.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn grant(router: &Router, challenge: &Value) -> String {
    let (status,value)=call(router,"POST","/api/v1/operation-approvals",HUMAN,serde_json::to_vec(&json!({"binding":challenge["error"]["approvalRequest"]["binding"],"reason":"Human reviewed exact effects"})).unwrap(),None).await;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    value["id"].as_str().unwrap().into()
}
#[tokio::test]
async fn deletion_requires_exact_separate_approval_and_keeps_files() {
    let (_dir, app) = app().await;
    let job = seed(&app).await;
    let other = seed(&app).await;
    let source = app.root.join("jobs").join(job.id.to_string());
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("document"), b"source").unwrap();
    let router = jrip_server::router(app.clone());
    let path = format!("/api/v1/jobs/{}", job.id);
    let (status, challenge) = call(&router, "DELETE", &path, AGENT, vec![], None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(challenge["error"]["code"], "approval_required");
    let binding = &challenge["error"]["approvalRequest"]["binding"];
    assert_eq!(binding["effect"]["before"]["revision"], job.revision);
    assert!(
        binding["effect"]["impact"]["ja"]
            .as_str()
            .unwrap()
            .contains("記録")
    );
    let self_grant = json!({"binding":binding,"reason":"self"});
    assert_eq!(
        call(
            &router,
            "POST",
            "/api/v1/operation-approvals",
            AGENT,
            serde_json::to_vec(&self_grant).unwrap(),
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &router,
            "POST",
            "/api/v1/operation-approvals",
            READER,
            serde_json::to_vec(&self_grant).unwrap(),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let id = grant(&router, &challenge).await;
    assert_eq!(
        call(
            &router,
            "DELETE",
            &format!("/api/v1/jobs/{}", other.id),
            AGENT,
            vec![],
            Some(&id)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&router, "DELETE", &path, HUMAN, vec![], Some(&id))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // Approval survives process recreation, but is admitted once.
    let mut reopened = jrip_server::open(app.root.clone(), String::new())
        .await
        .unwrap();
    reopened.credentials = app.credentials.clone();
    reopened.require_delete_approval = true;
    reopened.require_settings_approval = true;
    let router = jrip_server::router(reopened);
    assert_eq!(
        call(&router, "DELETE", &path, AGENT, vec![], Some(&id))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(source.join("document").exists());
    assert!(app.repo.get(job.id).await.is_err());
    let records = app.repo.audit_records(0, 100).await.unwrap();
    assert!(
        records["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["operation"] == "operation-approval-consume")
    );
}
#[tokio::test]
async fn retention_binding_covers_before_after_reason_and_revision() {
    let (_dir, app) = app().await;
    let job = seed(&app).await;
    let router = jrip_server::router(app.clone());
    let path = format!("/api/v1/jobs/{}/retention", job.id);
    let body = json!({"expectedRevision":0,"retainUntil":rip_storage::now()+86400,"reason":"retain evidence"});
    let bytes = serde_json::to_vec(&body).unwrap();
    assert_eq!(
        call(&router, "PUT", &path, READER, bytes.clone(), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, challenge) = call(&router, "PUT", &path, WRITER, bytes.clone(), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        challenge["error"]["approvalRequest"]["binding"]["effect"]["after"],
        body
    );
    let id = grant(&router, &challenge).await;
    let mut altered = body.clone();
    altered["reason"] = json!("changed");
    assert_eq!(
        call(
            &router,
            "PUT",
            &path,
            WRITER,
            serde_json::to_vec(&altered).unwrap(),
            Some(&id)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, policy) = call(&router, "PUT", &path, WRITER, bytes.clone(), Some(&id)).await;
    assert_eq!(status, StatusCode::OK, "{policy}");
    assert_eq!(policy["revision"], 1);
    assert_eq!(
        call(&router, "PUT", &path, WRITER, bytes, Some(&id))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let second =
        json!({"expectedRevision":1,"retainUntil":rip_storage::now()+172800,"reason":"extend"});
    let (_, challenge) = call(
        &router,
        "PUT",
        &path,
        WRITER,
        serde_json::to_vec(&second).unwrap(),
        None,
    )
    .await;
    let revoked = grant(&router, &challenge).await;
    assert_eq!(
        call(
            &router,
            "POST",
            &format!("/api/v1/operation-approvals/{revoked}/revoke"),
            HUMAN,
            vec![],
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &router,
            "PUT",
            &path,
            WRITER,
            serde_json::to_vec(&second).unwrap(),
            Some(&revoked)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn icc_and_color_policy_registration_require_exact_content() {
    use sha2::{Digest, Sha256};
    let (_dir, app) = app().await;
    let router = jrip_server::router(app.clone());
    let icc = lcms2::Profile::new_srgb().icc().unwrap();
    let hash = format!("{:x}", Sha256::digest(&icc));
    let (_, challenge) = call(
        &router,
        "POST",
        "/api/v1/color/profiles",
        WRITER,
        icc.clone(),
        None,
    )
    .await;
    assert_eq!(challenge["error"]["code"], "approval_required");
    assert!(
        !app.root
            .join("color-profiles")
            .join(format!("{hash}.icc"))
            .exists()
    );
    let id = grant(&router, &challenge).await;
    let mut different_icc = icc.clone();
    different_icc[24] ^= 1;
    assert_eq!(
        call(
            &router,
            "POST",
            "/api/v1/color/profiles",
            WRITER,
            different_icc,
            Some(&id)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &router,
            "POST",
            "/api/v1/color/profiles",
            WRITER,
            icc.clone(),
            Some(&id)
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(
            &router,
            "POST",
            "/api/v1/color/profiles",
            WRITER,
            icc,
            Some(&id)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let body = json!({"definition":{"schemaVersion":1,"labels":{"en":"Proof","ja":"校正","zh-CN":"校样"},"inputProfileSha256":hash,"outputProfileSha256":hash,"policy":{"schemaVersion":1,"inputColor":"rgb","outputColor":"rgb","intent":"relative_colorimetric","blackPointCompensation":false,"blackPreservation":"none","tacPercent":null}},"reason":"reviewed policy"});
    let bytes = serde_json::to_vec(&body).unwrap();
    let (_, challenge) = call(
        &router,
        "POST",
        "/api/v1/color/policies",
        WRITER,
        bytes.clone(),
        None,
    )
    .await;
    let id = grant(&router, &challenge).await;
    assert!(
        app.repo.color_policies("", 100).await.unwrap()["policies"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let mut altered = body.clone();
    altered["definition"]["labels"]["en"] = json!("Changed proof");
    assert_eq!(
        call(
            &router,
            "POST",
            "/api/v1/color/policies",
            WRITER,
            serde_json::to_vec(&altered).unwrap(),
            Some(&id)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // Canonical JSON binding accepts insignificant formatting changes.
    let pretty = serde_json::to_vec_pretty(&body).unwrap();
    assert_eq!(
        call(
            &router,
            "POST",
            "/api/v1/color/policies",
            WRITER,
            pretty,
            Some(&id)
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(
            &router,
            "POST",
            "/api/v1/color/policies",
            WRITER,
            bytes,
            Some(&id)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn receipt_key_revocation_and_policy_discovery_are_guarded() {
    let (_dir, app) = app().await;
    let doc =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let signer = rip_storage::receipts::ReceiptSigner::from_pkcs8(doc.as_ref()).unwrap();
    let repo = rip_storage::Repository::open_with_receipt_signer(
        &app.root.join("jobs.sqlite3"),
        Some(signer),
    )
    .await
    .unwrap();
    let keys = repo.receipt_keys("", 100).await.unwrap();
    let key = keys["keys"][0]["keyId"].as_str().unwrap();
    let router = jrip_server::router(app.clone());
    let (status, policy) = call(
        &router,
        "GET",
        "/api/v1/approval-policy",
        READER,
        vec![],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(policy["settingsApprovalRequired"], true);
    let path = format!("/api/v1/print/receipt-keys/{key}/revoke");
    let bytes = serde_json::to_vec(&json!({"reason":"retired"})).unwrap();
    let (_, challenge) = call(&router, "POST", &path, AGENT, bytes.clone(), None).await;
    assert_eq!(challenge["error"]["code"], "approval_required");
    let id = grant(&router, &challenge).await;
    assert_eq!(
        call(&router, "POST", &path, AGENT, bytes, Some(&id))
            .await
            .0,
        StatusCode::OK
    );
    assert!(repo.receipt_key_approval_state(key).await.unwrap()["revokedAt"].is_i64());
}

#[tokio::test]
async fn failed_business_action_does_not_restore_a_consumed_grant() {
    let (_dir, app) = app().await;
    let job = seed(&app).await;
    let router = jrip_server::router(app.clone());
    let path = format!("/api/v1/jobs/{}", job.id);
    let (_, challenge) = call(&router, "DELETE", &path, AGENT, vec![], None).await;
    let id = grant(&router, &challenge).await;
    // A new color dependency appears after review. Final deletion must still reject it.
    app.repo
        .create_color(Uuid::new_v4(), job.id, job.revision, json!({}))
        .await
        .unwrap();
    assert_eq!(
        call(&router, "DELETE", &path, AGENT, vec![], Some(&id))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert!(app.repo.get(job.id).await.is_ok());
    let (status, response) = call(&router, "DELETE", &path, AGENT, vec![], Some(&id)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(response["error"]["code"], "approval_required");
}
