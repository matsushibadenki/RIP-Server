use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;
const TOKEN: &str = "test-token-at-least-24-characters";
async fn app() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    let state = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    (dir, jrip_server::router(state))
}
fn upload(name: &str, format: &str, document: &str) -> Request<Body> {
    let manifest = serde_json::json!({"name":name,"format":format,"dpi":300});
    let body = format!(
        "--testboundary\r\nContent-Disposition: form-data; name=\"manifest\"\r\n\r\n{manifest}\r\n--testboundary\r\nContent-Disposition: form-data; name=\"document\"; filename=\"../../evil.ps\"\r\nContent-Type: application/octet-stream\r\n\r\n{document}\r\n--testboundary--\r\n"
    );
    Request::post("/api/v1/jobs")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "multipart/form-data; boundary=testboundary")
        .body(Body::from(body))
        .unwrap()
}
fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap()
}
#[tokio::test]
async fn job_lifecycle_and_storage_boundary() {
    let (dir, app) = app().await;
    let res = app
        .clone()
        .oneshot(upload(
            "縦書き-日本語.ps",
            "application/postscript",
            "%!PS-Adobe-3.0\nshowpage\n",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let job: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(job["manifest"]["engine"], "auto");
    let id = job["id"].as_str().unwrap();
    let path = format!("/api/v1/jobs/{id}");
    assert!(dir.path().join("jobs").join(id).join("document").exists());
    assert!(!dir.path().join("evil.ps").exists());
    for action in ["hold", "release", "cancel"] {
        assert_eq!(
            app.clone()
                .oneshot(request("POST", &format!("{path}/{action}")))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        app.clone()
            .oneshot(request("POST", &format!("{path}/hold")))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.clone()
            .oneshot(request("DELETE", &path))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.oneshot(request("GET", &path)).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn invalid_documents_leave_no_spool_and_auth_is_required() {
    let (dir, app) = app().await;
    assert_eq!(
        app.clone()
            .oneshot(Request::get("/api/v1/jobs").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for (format, document) in [
        ("application/pdf", "%!PS-Adobe-3.0"),
        ("application/pdf", "not a PDF"),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(upload("invalid", format, document))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        std::fs::read_dir(dir.path().join("jobs")).unwrap().count(),
        0
    );
}

#[tokio::test]
#[ignore = "requires Docker, jrip-mupdf:local and jrip-ghostscript:local images"]
async fn real_worker_uses_hybrid_engine_and_exports_tiff() {
    for (format, document, expected_engine) in [
        (
            "application/postscript",
            include_str!("../../../tests/corpus/basic.ps"),
            "ghostscript",
        ),
        (
            "application/pdf",
            include_str!("../../../tests/corpus/colors.pdf"),
            "mupdf",
        ),
    ] {
        let (dir, app) = app().await;
        let res = app
            .clone()
            .oneshot(upload("fixture", format, document))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let job: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(job["manifest"]["engine"], "auto");
        let id = job["id"].as_str().unwrap();
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_jrip-worker"))
            .arg(id)
            .env("JRIP_DATA_ROOT", dir.path())
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let path = dir
            .path()
            .join("jobs")
            .join(id)
            .join("raster/page-000001.tiff");
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.starts_with(b"II\x2a\0") || bytes.starts_with(b"MM\0\x2a"));
        let res = app
            .oneshot(request("GET", &format!("/api/v1/jobs/{id}")))
            .await
            .unwrap();
        let job: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(job["state"], "COMPLETED");
        assert_eq!(job["selected_engine"], expected_engine);
    }
}
