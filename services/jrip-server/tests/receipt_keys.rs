use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
#[tokio::test]
async fn discovery_is_authenticated_scoped_and_has_strict_pagination() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), "reader".into())
        .await
        .unwrap();
    app.credentials = vec![
        jrip_server::Credential {
            token: "reader".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
        },
        jrip_server::Credential {
            token: "submitter".into(),
            identity: "submitter".into(),
            scopes: vec!["jobs:submit".into()],
        },
    ];
    for (token, path, status) in [
        ("reader", "/api/v1/print/receipt-keys", StatusCode::OK),
        (
            "submitter",
            "/api/v1/print/receipt-keys",
            StatusCode::FORBIDDEN,
        ),
        (
            "invalid",
            "/api/v1/print/receipt-keys",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "reader",
            "/api/v1/print/receipt-keys?limit=101",
            StatusCode::BAD_REQUEST,
        ),
        (
            "reader",
            "/api/v1/print/receipt-keys?after=invalid",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let response = jrip_server::router(app.clone())
            .oneshot(
                Request::get(path)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        if status == StatusCode::OK {
            let value: Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(value["keys"], json!([]));
            assert_eq!(value["signingEnabledForThisProcess"], false);
            assert_eq!(value["trustEstablished"], false);
        }
    }
}

#[tokio::test]
async fn revocation_is_admin_only_atomic_audited_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), "admin".into())
        .await
        .unwrap();
    let private =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    app.repo = rip_storage::Repository::open_with_receipt_signer(
        &app.root.join("jobs.sqlite3"),
        Some(rip_storage::receipts::ReceiptSigner::from_pkcs8(private.as_ref()).unwrap()),
    )
    .await
    .unwrap();
    app.credentials = vec![
        jrip_server::Credential {
            token: "admin".into(),
            identity: "operator".into(),
            scopes: vec!["admin".into()],
        },
        jrip_server::Credential {
            token: "reader".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
        },
    ];
    let catalog = app.repo.receipt_keys("", 50).await.unwrap();
    let id = catalog["keys"][0]["keyId"].as_str().unwrap();
    let path = format!("/api/v1/print/receipt-keys/{id}/revoke");
    for (token, reason, status, replay) in [
        ("reader", "compromised", StatusCode::FORBIDDEN, false),
        ("admin", "invalid", StatusCode::BAD_REQUEST, false),
        ("admin", "compromised", StatusCode::OK, false),
        ("admin", "compromised", StatusCode::OK, true),
        ("admin", "retired", StatusCode::CONFLICT, false),
    ] {
        let response = jrip_server::router(app.clone())
            .oneshot(
                Request::post(&path)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"reason":reason}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        if status == StatusCode::OK {
            let value: Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(value["replayed"], replay);
            assert_eq!(value["actor"], "operator");
        }
    }
    let reopened =
        rip_storage::Repository::open_with_receipt_signer(&app.root.join("jobs.sqlite3"), None)
            .await
            .unwrap();
    assert_eq!(
        reopened.receipt_keys("", 50).await.unwrap()["keys"][0]["revoked"],
        true
    );
    assert_eq!(
        app.repo.receipt_keys("", 50).await.unwrap()["signingEnabledForThisProcess"],
        false
    );
    assert_eq!(
        reopened.audit_records(0, 100).await.unwrap()["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["operation"] == "receipt-key-revoke")
            .count(),
        1
    );
}
