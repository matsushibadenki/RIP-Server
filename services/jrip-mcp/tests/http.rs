use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::Response,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
const TOKEN: &str = "owner-token";
async fn post(router: &Router, token: &str, session: Option<&str>, message: Value) -> Response {
    let mut request = Request::post("/mcp")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25");
    if let Some(session) = session {
        request = request.header("mcp-session-id", session);
    }
    router
        .clone()
        .oneshot(request.body(Body::from(message.to_string())).unwrap())
        .await
        .unwrap()
}
fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"},"capabilities":{}}})
}
async fn setup() -> (tempfile::TempDir, jrip_server::App, Router, String) {
    setup_with_limits(jrip_mcp::rate_limit::RequestLimits::default()).await
}
async fn setup_with_limits(
    limits: jrip_mcp::rate_limit::RequestLimits,
) -> (tempfile::TempDir, jrip_server::App, Router, String) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    app.credentials = vec![
        jrip_server::Credential {
            token: TOKEN.into(),
            identity: "owner".into(),
            scopes: vec!["jobs:submit".into()],
        },
        jrip_server::Credential {
            token: "other-token".into(),
            identity: "other".into(),
            scopes: vec!["jobs:submit".into()],
        },
    ];
    let router = jrip_mcp::http::router_with_limits(
        app.clone(),
        vec!["https://print.example".into()],
        limits,
    );
    let response = post(&router, TOKEN, None, initialize()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let session = response.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let result: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(result["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        post(
            &router,
            TOKEN,
            Some(&session),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    (dir, app, router, session)
}
async fn get(router: &Router, session: &str) -> Response {
    router
        .clone()
        .oneshot(
            Request::get("/mcp")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("mcp-session-id", session)
                .header("mcp-protocol-version", "2025-11-25")
                .header("accept", "text/event-stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}
async fn notification(body: &mut Body) -> Value {
    use http_body_util::BodyExt;
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(3), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let bytes = frame.into_data().unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        if let Some(data) = text
            .lines()
            .find_map(|line| line.strip_prefix("data:").map(str::trim))
            .filter(|data| !data.is_empty())
        {
            return serde_json::from_str(data).unwrap();
        }
    }
}
#[tokio::test]
async fn remote_controls_notifications_reconnect_and_delete_share_one_session() {
    let (_dir, app, router, session) = setup().await;
    let id = Uuid::new_v4();
    app.repo
        .enqueue_inspection(id, "owner", &"a".repeat(64), 1, Uuid::new_v4())
        .await
        .unwrap();
    let uri = format!("jrip://inspections/{id}/events");
    let subscribe =
        json!({"jsonrpc":"2.0","id":2,"method":"resources/subscribe","params":{"uri":uri}});
    assert_eq!(
        post(&router, TOKEN, Some(&session), subscribe)
            .await
            .status(),
        StatusCode::OK
    );
    let response = get(&router, &session).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut stream = response.into_body();
    assert_eq!(notification(&mut stream).await["params"]["uri"], uri);
    assert_eq!(get(&router, &session).await.status(), StatusCode::CONFLICT);
    app.repo
        .set_inspection_state(id, 0, "RUNNING", None, None)
        .await
        .unwrap();
    assert_eq!(
        notification(&mut stream).await["method"],
        "notifications/resources/updated"
    );
    drop(stream);
    let mut stream = get(&router, &session).await.into_body();
    assert_eq!(notification(&mut stream).await["params"]["uri"], uri);
    let response=post(&router,TOKEN,Some(&session),json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"print_get_document_inspection_events","arguments":{"inspectionId":id}}})).await;
    let reply: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    let events: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(events["events"].as_array().unwrap().len(), 2);
    let response = router
        .clone()
        .oneshot(
            Request::delete("/mcp")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("mcp-session-id", &session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(get(&router, &session).await.status(), StatusCode::NOT_FOUND);
    use http_body_util::BodyExt;
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(3), stream.frame())
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn tokens_origins_protocol_headers_and_messages_are_checked() {
    let (_dir, _app, router, session) = setup().await;
    let ping = json!({"jsonrpc":"2.0","id":2,"method":"ping"});
    assert_eq!(
        post(&router, "other-token", Some(&session), ping.clone())
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(&router, "invalid", Some(&session), ping.clone())
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(&router, TOKEN, None, ping).await.status(),
        StatusCode::BAD_REQUEST
    );
    for (name, value, status) in [
        ("origin", "https://evil.example", StatusCode::FORBIDDEN),
        ("origin", "https://print.example", StatusCode::OK),
        (
            "mcp-protocol-version",
            "2024-11-05",
            StatusCode::BAD_REQUEST,
        ),
        ("accept", "application/json", StatusCode::NOT_ACCEPTABLE),
    ] {
        let mut request = Request::post("/mcp")
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(Body::from(initialize().to_string()))
            .unwrap();
        request.headers_mut().insert(
            axum::http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), status, "{name}");
    }
    assert_eq!(
        post(&router, TOKEN, None, json!([initialize()]))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(
            &router,
            TOKEN,
            None,
            json!({"jsonrpc":"2.0","id":null,"method":"initialize"})
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let oversized =
        json!({"jsonrpc":"2.0","id":2,"method":"ping","params":{"data":"x".repeat(65536)}});
    let invalid_origin = router
        .clone()
        .oneshot(
            Request::post("/mcp")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("origin", "https://evil.example")
                .body(Body::from(oversized.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_origin.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        post(&router, TOKEN, Some(&session), oversized)
            .await
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let response = post(
        &router,
        TOKEN,
        Some(&session),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
    )
    .await;
    let reply: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert!(
        !reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "print_submit_document_inspection")
    );
}
#[tokio::test]
async fn unknown_sessions_after_restart_require_reinitialization() {
    let (_dir, app, _router, session) = setup().await;
    let restarted = jrip_mcp::http::router(app, vec![]);
    assert_eq!(
        get(&restarted, &session).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(&restarted, TOKEN, None, initialize()).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn session_expiry_capacity_and_origin_rejection_apply_to_get_and_delete() {
    let (_dir, _app, router, session) =
        setup_with_limits(jrip_mcp::rate_limit::RequestLimits::new(1000, 1000).unwrap()).await;
    for method in [axum::http::Method::GET, axum::http::Method::DELETE] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/mcp")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("mcp-session-id", &session)
                    .header("accept", "text/event-stream")
                    .header("origin", "https://evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    for _ in 1..64 {
        assert_eq!(
            post(&router, TOKEN, None, initialize()).await.status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        post(&router, TOKEN, None, initialize()).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(901)).await;
    assert_eq!(get(&router, &session).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        post(&router, TOKEN, None, initialize()).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn real_http_connection_receives_unsolicited_updates() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_dir, app, router, session) = setup().await;
    let id = Uuid::new_v4();
    app.repo
        .enqueue_inspection(id, "owner", &"a".repeat(64), 1, Uuid::new_v4())
        .await
        .unwrap();
    let uri = format!("jrip://inspections/{id}/events");
    let subscribe =
        json!({"jsonrpc":"2.0","id":2,"method":"resources/subscribe","params":{"uri":uri}});
    assert_eq!(
        post(&router, TOKEN, Some(&session), subscribe)
            .await
            .status(),
        StatusCode::OK
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket.write_all(format!("GET /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\nMCP-Session-Id: {session}\r\nMCP-Protocol-Version: 2025-11-25\r\nAccept: text/event-stream\r\n\r\n").as_bytes()).await.unwrap();
    let mut bytes = vec![];
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !String::from_utf8_lossy(&bytes).contains(&uri) {
            assert!(socket.read_buf(&mut bytes).await.unwrap() > 0);
        }
    })
    .await
    .unwrap();
    let response = String::from_utf8_lossy(&bytes);
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("text/event-stream"));
    assert!(response.contains("notifications/resources/updated"));
    bytes.clear();
    app.repo
        .set_inspection_state(id, 0, "FAILED", Some("TEST_FAILURE"), None)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !String::from_utf8_lossy(&bytes).contains(&uri) {
            assert!(socket.read_buf(&mut bytes).await.unwrap() > 0);
        }
    })
    .await
    .unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("notifications/resources/updated"));
    drop(socket);
    server.abort();
}

#[tokio::test]
async fn rate_limits_share_identity_across_tokens_sessions_and_methods_before_body_reads() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (_dir, mut app, _, _) = setup().await;
    app.credentials.push(jrip_server::Credential {
        token: "owner-alias-token".into(),
        identity: "owner".into(),
        scopes: vec!["jobs:submit".into()],
    });
    let router = jrip_mcp::http::router_with_limits(
        app,
        vec![],
        jrip_mcp::rate_limit::RequestLimits::new(1, 2).unwrap(),
    );
    // Invalid credentials and Origin are rejected without consuming the valid identity budget.
    assert_eq!(
        post(&router, "unknown", None, initialize()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let invalid_origin = router
        .clone()
        .oneshot(
            Request::post("/mcp")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("origin", "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_origin.status(), StatusCode::FORBIDDEN);
    let first = post(&router, TOKEN, None, initialize()).await;
    assert_eq!(first.status(), StatusCode::OK);
    let session = first.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        post(&router, "owner-alias-token", None, initialize())
            .await
            .status(),
        StatusCode::OK
    );
    let polls = Arc::new(AtomicUsize::new(0));
    let observed = polls.clone();
    let body = Body::from_stream(futures_util::stream::poll_fn(move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        std::task::Poll::Ready(Some(Err::<axum::body::Bytes, _>(std::io::Error::other(
            "must not read exhausted request",
        ))))
    }));
    let response = router
        .clone()
        .oneshot(
            Request::post("/mcp")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    let wait = response.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!((1..=60).contains(&wait));
    let error: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(error["error"]["code"], "rate_limited");
    assert_eq!(error["error"]["retryable"], true);
    assert_eq!(error["error"]["requestsPerMinute"], 1);
    assert_eq!(error["error"]["burst"], 2);
    assert_eq!(error["error"]["retryAfterSeconds"], wait);
    for locale in ["en", "ja", "zh-CN"] {
        assert!(error["error"]["messages"][locale].is_string());
    }
    for method in ["GET", "DELETE"] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/mcp")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("mcp-session-id", &session)
                    .header("accept", "text/event-stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }
    assert_eq!(
        post(&router, "other-token", None, initialize())
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn rate_limit_recovery_keeps_the_same_session_and_does_not_dispatch_denied_calls() {
    let (_dir, app, _, _) = setup().await;
    let router = jrip_mcp::http::router_with_limits(
        app,
        vec![],
        jrip_mcp::rate_limit::RequestLimits::new(120, 2).unwrap(),
    );
    let first = post(&router, TOKEN, None, initialize()).await;
    let session = first.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        post(
            &router,
            TOKEN,
            Some(&session),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    let message = json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
    assert_eq!(
        post(&router, TOKEN, Some(&session), message.clone())
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // Real time integration complements deterministic fractional refill unit tests.
    tokio::time::sleep(std::time::Duration::from_millis(550)).await;
    let response = post(&router, TOKEN, Some(&session), message).await;
    assert_eq!(response.status(), StatusCode::OK);
    let result: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert!(result["result"]["tools"].is_array());
}
