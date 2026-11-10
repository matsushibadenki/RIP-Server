use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const TOKEN: &str = "font-test-admin-token-at-least-24-characters";
fn upload(token: &str, bytes: Vec<u8>) -> Request<Body> {
    Request::post("/api/v1/fonts")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/octet-stream")
        .body(Body::from(bytes))
        .unwrap()
}
#[tokio::test]
async fn upload_authorization_and_invalid_programs_do_not_change_library() {
    let _lock = LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(root.path().into(), TOKEN.into())
        .await
        .unwrap();
    app.credentials.push(jrip_server::Credential {
        token: TOKEN.into(),
        identity: "admin".into(),
        scopes: vec!["admin".into()],
        validity: None,
    });
    app.credentials.push(jrip_server::Credential {
        token: "font-reader-token".into(),
        identity: "reader".into(),
        scopes: vec!["jobs:read".into()],
        validity: None,
    });
    let router = jrip_server::router(app);
    for (token, status) in [
        ("invalid", StatusCode::UNAUTHORIZED),
        ("font-reader-token", StatusCode::FORBIDDEN),
        (TOKEN, StatusCode::BAD_REQUEST),
    ] {
        let response = router
            .clone()
            .oneshot(upload(token, b"invalid font".to_vec()))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    assert!(rip_font::store::current(root.path()).unwrap().is_none());
}
#[tokio::test]
#[ignore = "requires Docker RIP images"]
async fn installed_fonts_survive_restart_and_appear_for_both_engines() {
    let _lock = LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let app = jrip_server::open(root.path().into(), TOKEN.into())
        .await
        .unwrap();
    let router = jrip_server::router(app);
    let bytes = include_bytes!("../../../crates/rip-font/tests/data/shaping.ttf").to_vec();
    let response = router.oneshot(upload(TOKEN, bytes)).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let installed: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let name = installed["installed"][0]["name"].as_str().unwrap();
    let app = jrip_server::open(root.path().into(), TOKEN.into())
        .await
        .unwrap();
    let router = jrip_server::router(app);
    let response = router
        .oneshot(
            Request::get("/api/v1/fonts")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let catalog: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(
        catalog["registeredSnapshotSha256"],
        installed["snapshotSha256"]
    );
    for engine in catalog["engines"].as_array().unwrap() {
        assert!(
            engine["fonts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["name"] == name && f["source"] == "installed")
        );
    }
}
