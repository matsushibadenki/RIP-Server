use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    http::{Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;
const TOKEN: &str = "stored-printer-sse-reader";
const URI: &str = "jrip://printers/test/observations";
async fn post(router: &Router, session: Option<&str>, value: Value) -> Response {
    let mut request = Request::post("/mcp")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25");
    if let Some(session) = session {
        request = request.header("mcp-session-id", session);
    }
    router
        .clone()
        .oneshot(request.body(Body::from(value.to_string())).unwrap())
        .await
        .unwrap()
}
async fn get(router: &Router, session: &str) -> Response {
    router
        .clone()
        .oneshot(
            Request::get("/mcp")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("accept", "text/event-stream")
                .header("mcp-protocol-version", "2025-11-25")
                .header("mcp-session-id", session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}
async fn json(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}
async fn notification(body: &mut Body) -> Value {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(3), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let data = frame.into_data().unwrap();
        let text = std::str::from_utf8(&data).unwrap();
        if let Some(data) = text
            .lines()
            .find_map(|l| l.strip_prefix("data:").map(str::trim))
            .filter(|s| !s.is_empty())
        {
            return serde_json::from_str(data).unwrap();
        }
    }
}
async fn live(router: &Router, session: &str, id: i64) {
    let value=json(post(router,Some(session),json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"print_get_printer","arguments":{"printerId":"test"}}})).await).await;
    assert_eq!(value["result"]["isError"], false, "{value}");
}
#[tokio::test]
async fn sse_printer_notifications_are_pushed_replay_after_reconnect_and_stop_after_unsubscribe() {
    let queries = Arc::new(AtomicUsize::new(0));
    let level = Arc::new(AtomicUsize::new(0));
    let calls = queries.clone();
    let current = level.clone();
    let printer = Router::new().route(
        "/printer",
        axum::routing::post(move |bytes: Bytes| {
            let calls = calls.clone();
            let current = current.clone();
            async move {
                let request = rip_ipp::Request::parse(&bytes).unwrap();
                assert_eq!(request.operation, rip_ipp::GET_PRINTER_ATTRIBUTES);
                calls.fetch_add(1, Ordering::SeqCst);
                let mut response = rip_ipp::Response::new([1, 1], rip_ipp::OK, request.request_id);
                response.text(0x47, "attributes-charset", "utf-8");
                response.text(0x48, "attributes-natural-language", "en");
                response.group(4);
                response.text(0x49, "document-format-supported", "application/pdf");
                response.attribute(0x22, "printer-is-accepting-jobs", &[1]);
                response.attribute(0x23, "printer-state", &3i32.to_be_bytes());
                response.integer("marker-levels", current.load(Ordering::SeqCst) as i32);
                (
                    [(axum::http::header::CONTENT_TYPE, "application/ipp")],
                    response.finish(),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, printer).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    app.credentials = vec![jrip_server::Credential {
        token: TOKEN.into(),
        identity: "reader".into(),
        scopes: vec!["jobs:read".into()],
        validity: None,
    }];
    app.printers = vec![jrip_server::delivery::Printer {
        id: "test".into(),
        uri: format!("ipp://{address}/printer"),
        media: vec![],
        color_target: None,
    }];
    let router = jrip_mcp::http::router(app, vec![]);
    let response=post(&router,None,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"},"capabilities":{}}})).await;
    assert_eq!(response.status(), StatusCode::OK);
    let session = response.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    post(
        &router,
        Some(&session),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    let subscribe = json(
        post(
            &router,
            Some(&session),
            json!({"jsonrpc":"2.0","id":2,"method":"resources/subscribe","params":{"uri":URI}}),
        )
        .await,
    )
    .await;
    assert!(subscribe["error"].is_null(), "{subscribe}");
    let response = get(&router, &session).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut stream = response.into_body();
    assert_eq!(queries.load(Ordering::SeqCst), 0);
    live(&router, &session, 3).await;
    assert_eq!(notification(&mut stream).await["params"]["uri"], URI);
    assert_eq!(queries.load(Ordering::SeqCst), 1);
    live(&router, &session, 4).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(1200), notification(&mut stream))
            .await
            .is_err(),
        "identical reports do not notify"
    );
    assert_eq!(
        queries.load(Ordering::SeqCst),
        2,
        "subscription does not poll the device"
    );
    level.store(25, Ordering::SeqCst);
    live(&router, &session, 5).await;
    assert_eq!(notification(&mut stream).await["params"]["uri"], URI);
    drop(stream);
    let response = get(&router, &session).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body();
    assert_eq!(
        notification(&mut stream).await["params"]["uri"],
        URI,
        "reconnect replays resource invalidation"
    );
    let reply=json(post(&router,Some(&session),json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"print_get_printer_observations","arguments":{"printerId":"test"}}})).await).await;
    let history: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(history["events"].as_array().unwrap().len(), 2);
    let reply = json(
        post(
            &router,
            Some(&session),
            json!({"jsonrpc":"2.0","id":7,"method":"resources/unsubscribe","params":{"uri":URI}}),
        )
        .await,
    )
    .await;
    assert!(reply["error"].is_null());
    level.store(30, Ordering::SeqCst);
    live(&router, &session, 8).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(1200), notification(&mut stream))
            .await
            .is_err()
    );
    assert_eq!(queries.load(Ordering::SeqCst), 4);
    drop(stream);
    server.abort();
}
