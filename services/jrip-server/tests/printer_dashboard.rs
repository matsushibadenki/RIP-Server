//! A no-job, read-only console fixture. No Docker or physical printer required.
use axum::{
    Router,
    body::{Body, Bytes},
    http::{Request, StatusCode},
    routing::{get, post},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tower::ServiceExt;
#[tokio::test]
#[ignore = "requires Playwright Chromium"]
async fn printer_dashboard_works_without_jobs_and_fences_session_and_device_changes() {
    let dir = tempfile::tempdir().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let failure = Arc::new(AtomicBool::new(false));
    let queries = count.clone();
    let broken = failure.clone();
    let device = Router::new().route(
        "/ipp",
        post(move |bytes: Bytes| {
            let count = queries.clone();
            let failure = broken.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                let request = rip_ipp::Request::parse(&bytes).unwrap();
                assert_eq!(
                    request.operation,
                    rip_ipp::GET_PRINTER_ATTRIBUTES,
                    "dashboard cannot print or cancel"
                );
                if failure.load(Ordering::SeqCst) {
                    return ([("content-type", "application/ipp")], b"invalid".to_vec());
                }
                let mut r = rip_ipp::Response::new([1, 1], rip_ipp::OK, request.request_id);
                r.text(0x47, "attributes-charset", "utf-8");
                r.text(0x48, "attributes-natural-language", "en");
                r.group(4);
                r.text(0x49, "document-format-supported", "application/pdf");
                r.attribute(0x23, "printer-state", &5i32.to_be_bytes());
                r.attribute(0x22, "printer-is-accepting-jobs", &[1]);
                r.text(0x44, "printer-state-reasons", "media-jam-error");
                r.integer("queued-job-count", 0);
                r.integer("marker-levels", 0);
                ([("content-type", "application/ipp")], r.finish())
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let device_task = tokio::spawn(async move { axum::serve(listener, device).await.unwrap() });
    let mut app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    let printer = jrip_server::delivery::Printer {
        id: "p1".into(),
        uri: format!("ipp://{addr}/ipp"),
        media: vec![],
        color_target: None,
    };
    app.printers = vec![
        printer.clone(),
        jrip_server::delivery::Printer {
            id: "p2".into(),
            ..printer
        },
    ];
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::get("/api/v1/printers/p1")
                .header("authorization", "Bearer token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    let printers = app.printers.clone();
    drop(app);
    let mut app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    app.printers = printers;
    app.credentials = vec![jrip_server::Credential {
        token: "reader".into(),
        identity: "reader".into(),
        scopes: vec!["jobs:read".into()],
        validity: None,
    }];
    let counter = count.clone();
    let fail = failure.clone();
    let router = jrip_server::router(app)
        .route(
            "/__test/status",
            get(move || {
                let count = counter.load(Ordering::SeqCst);
                async move { axum::Json(serde_json::json!({"queries":count})) }
            }),
        )
        .route(
            "/__test/fail",
            post(move |axum::Json(value): axum::Json<bool>| {
                fail.store(value, Ordering::SeqCst);
                async { StatusCode::NO_CONTENT }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let result = tokio::process::Command::new("node")
        .arg("tests/ui/printer-dashboard.mjs")
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .env("JRIP_UI_URL", format!("http://{addr}/console"))
        .status()
        .await
        .unwrap();
    server.abort();
    device_task.abort();
    assert!(result.success(), "dashboard browser flow failed");
}
