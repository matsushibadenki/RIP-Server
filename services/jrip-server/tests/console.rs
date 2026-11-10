use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;
#[tokio::test]
async fn console_assets_are_same_origin_static_and_do_not_grant_api_access() {
    let directory = tempfile::tempdir().unwrap();
    let app = jrip_server::router(
        jrip_server::open(directory.path().into(), "console-secret-token".into())
            .await
            .unwrap(),
    );
    for (path, mime, marker) in [
        ("/console", "text/html", "id=\"upload\""),
        ("/console/", "text/html", "id=\"login\""),
        (
            "/console/app.js",
            "text/javascript",
            "x-jrip-expected-revision",
        ),
        ("/console/style.css", "text/css", "@media"),
        (
            "/console/printer-stream.js",
            "text/javascript",
            "window.JripPrinterStream",
        ),
        (
            "/console/printer-dashboard.js",
            "text/javascript",
            "window.JripPrinterDashboard",
        ),
        (
            "/console/printer-status.js",
            "text/javascript",
            "window.JripPrinterStatus",
        ),
        (
            "/console/delivery.js",
            "text/javascript",
            "expectedTicketSha256",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with(mime)
        );
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert!(
            response.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("frame-ancestors 'none'")
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(text.contains(marker));
        assert!(!text.contains("console-secret-token"));
    }
    let response = app
        .oneshot(Request::get("/api/v1/jobs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
