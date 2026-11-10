use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use rip_storage::printer_observation::QueryFailure;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;
use uuid::Uuid;
const URL: &str = "/api/v1/printers/test/observations/stream";
fn binding(app: &jrip_server::App) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(
                &json!({"backend":"native-ipp","printer":app.printers[0],"security":null})
            )
            .unwrap()
        )
    )
}
async fn fixture() -> (
    tempfile::TempDir,
    jrip_server::App,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let counter = count.clone();
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            drop(socket);
        }
    });
    app.printers = vec![jrip_server::delivery::Printer {
        id: "test".into(),
        uri: format!("ipp://{addr}/ipp"),
        media: vec![],
        color_target: None,
    }];
    (dir, app, count, task)
}
fn snapshot(count: u32) -> Value {
    json!({"schemaVersion":1,"profile":"org.jrip.printer-state-snapshot-v1","state":3,"acceptingJobs":true,"queuedJobCount":count,"reasons":[],"conditions":rip_core::printer_conditions::describe(3,&[]),"markerSupplies":{"profile":"org.jrip.cups-marker-supplies-v1","status":"UNREPORTED"},"printerSupplies":{"profile":"org.jrip.pwg-printer-supplies-v1","status":"UNREPORTED"}})
}
async fn record(app: &jrip_server::App, result: Result<Value, QueryFailure>) {
    let binding = binding(app);
    let serial = app
        .repo
        .begin_printer_query("test", &binding)
        .await
        .unwrap();
    app.repo
        .finish_printer_query("test", &binding, serial, result)
        .await
        .unwrap();
}
async fn connect(app: &jrip_server::App, url: &str, token: &str, last: Option<&str>) -> Response {
    let mut request = Request::get(url).header("authorization", format!("Bearer {token}"));
    if let Some(last) = last {
        request = request.header("last-event-id", last);
    }
    jrip_server::router(app.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}
async fn event(body: &mut Body) -> (String, String, Value) {
    let data = tokio::time::timeout(Duration::from_secs(3), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap();
    let text = std::str::from_utf8(&data).unwrap();
    let value = text
        .lines()
        .find_map(|s| s.strip_prefix("data:"))
        .unwrap()
        .trim();
    let id = text
        .lines()
        .find_map(|s| s.strip_prefix("id:"))
        .unwrap_or("")
        .trim()
        .to_owned();
    let kind = text
        .lines()
        .find_map(|s| s.strip_prefix("event:"))
        .unwrap()
        .trim()
        .to_owned();
    (id, kind, serde_json::from_str(value).unwrap())
}
#[tokio::test]
async fn latest_snapshots_coalesce_queries_and_resume_after_restart_without_device_io() {
    let (_dir, app, count, task) = fixture().await;
    let response = connect(&app, URL, "token", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(response.headers()["x-accel-buffering"], "no");
    let mut body = response.into_body();
    let (id, kind, empty) = event(&mut body).await;
    assert_eq!(id, "");
    assert_eq!(kind, "printer-observation");
    assert!(empty["latest"].is_null());
    record(&app, Ok(snapshot(0))).await;
    let (first, _, zero) = event(&mut body).await;
    assert!(first.ends_with(":1"));
    assert_eq!(zero["latest"]["snapshot"]["queuedJobCount"], 0);
    assert_eq!(zero["isLive"], false);
    assert_eq!(zero["authorizationGranted"], false);
    // Identical content still reports a new successful query/TTL, without an event revision.
    record(&app, Ok(snapshot(0))).await;
    let (second, _, same) = event(&mut body).await;
    assert!(second.ends_with(":2"));
    assert_eq!(same["latest"]["revision"], zero["latest"]["revision"]);
    record(&app, Ok(snapshot(1))).await;
    record(&app, Ok(snapshot(2))).await;
    let (coalesced, _, latest) = event(&mut body).await;
    assert!(coalesced.ends_with(":4"));
    assert_eq!(latest["latest"]["snapshot"]["queuedJobCount"], 2);
    record(&app, Err(QueryFailure::Unavailable)).await;
    let (failed, _, failure) = event(&mut body).await;
    assert!(failed.ends_with(":5"));
    assert_eq!(failure["latest"]["queryFailure"], "UNAVAILABLE");
    assert_eq!(failure["latest"]["snapshot"], latest["latest"]["snapshot"]);
    assert_eq!(
        failure["latest"]["expiresAt"],
        latest["latest"]["expiresAt"]
    );
    drop(body);
    let mut reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    reopened.printers = app.printers.clone();
    let response = connect(
        &reopened,
        &format!("{URL}?after={first}"),
        "token",
        Some(&failed),
    )
    .await;
    let mut body = response.into_body();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), body.frame())
            .await
            .is_err(),
        "Last-Event-ID takes precedence and equal head is not repeated"
    );
    record(&reopened, Ok(snapshot(2))).await;
    let (recovered, _, value) = event(&mut body).await;
    assert!(recovered.ends_with(":6"));
    assert!(value["latest"]["queryFailure"].is_null());
    drop(body);
    // A stale cursor or an ahead cursor receives the current complete head, never a fake replay.
    let mut body = connect(&reopened, URL, "token", Some(&first))
        .await
        .into_body();
    let (id, _, value) = event(&mut body).await;
    assert_eq!(id, recovered);
    assert_eq!(value["deliveryMeaning"], "coalesced-latest-snapshot");
    drop(body);
    let ahead = format!("{}:999", recovered.split(':').next().unwrap());
    let mut body = connect(&reopened, URL, "token", Some(&ahead))
        .await
        .into_body();
    assert_eq!(event(&mut body).await.2["reasonCode"], "CURSOR_AHEAD");
    drop(body);
    let mut changed = reopened.clone();
    changed.printers[0].uri.push_str("/new");
    let mut body = connect(&changed, URL, "token", Some(&recovered))
        .await
        .into_body();
    assert!(event(&mut body).await.2["latest"].is_null());
    drop(body);
    record(&changed, Ok(snapshot(9))).await;
    let mut body = connect(&changed, URL, "token", Some(&recovered))
        .await
        .into_body();
    let (_, _, value) = event(&mut body).await;
    assert_eq!(value["reasonCode"], "STREAM_CHANGED");
    assert_eq!(value["latest"]["snapshot"]["queuedJobCount"], 9);
    drop(body);
    assert_eq!(count.load(Ordering::SeqCst), 0);
    task.abort();
}
#[tokio::test]
async fn authorization_cursor_capacity_and_disconnect_are_bounded() {
    let (_dir, mut app, count, task) = fixture().await;
    app.event_streams = Arc::new(tokio::sync::Semaphore::new(1));
    app.credentials = vec![
        jrip_server::Credential {
            token: "reader".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
            validity: None,
        },
        jrip_server::Credential {
            token: "writer".into(),
            identity: "writer".into(),
            scopes: vec!["jobs:submit".into()],
            validity: None,
        },
    ];
    assert_eq!(
        connect(&app, URL, "bad", None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        connect(&app, URL, "writer", None).await.status(),
        StatusCode::FORBIDDEN
    );
    for bad in [
        "1",
        "invalid:2",
        "00000000-0000-0000-0000-000000000000:-1",
        "00000000-0000-0000-0000-000000000000:01",
    ] {
        assert_eq!(
            connect(&app, URL, "reader", Some(bad)).await.status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        connect(
            &app,
            &format!("{URL}?after=bad"),
            "reader",
            Some(&format!("{}:1", Uuid::new_v4()))
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let body = connect(&app, URL, "reader", None).await.into_body();
    assert_eq!(app.event_streams.available_permits(), 0);
    assert_eq!(
        connect(&app, URL, "reader", None).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    drop(body);
    assert_eq!(app.event_streams.available_permits(), 1);
    let mut body = connect(&app, URL, "reader", None).await.into_body();
    assert!(event(&mut body).await.2["latest"].is_null());
    drop(body);
    let mut unknown = app.clone();
    unknown.printers.clear();
    assert_eq!(
        connect(&unknown, URL, "reader", None).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    task.abort();
}
#[tokio::test]
async fn expired_credentials_stop_even_buffered_snapshots() {
    let (_dir, mut app, _count, task) = fixture().await;
    record(&app, Ok(snapshot(0))).await;
    let now = rip_storage::now();
    app.credentials = vec![jrip_server::Credential {
        token: "short".into(),
        identity: "reader".into(),
        scopes: vec!["jobs:read".into()],
        validity: Some(jrip_server::CredentialValidity {
            not_before: now - 1,
            expires_at: now + 2,
        }),
    }];
    let response = connect(&app, URL, "short", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (_, kind, value) = event(&mut body).await;
    assert_eq!(kind, "stream-error");
    assert_eq!(value["reasonCode"], "AUTHORIZATION_EXPIRED");
    assert!(body.frame().await.is_none());
    task.abort();
}

#[tokio::test]
async fn real_http_pushes_updates_and_disconnect_releases_shared_capacity() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_dir, mut app, count, task) = fixture().await;
    app.event_streams = Arc::new(tokio::sync::Semaphore::new(1));
    record(&app, Ok(snapshot(8))).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = jrip_server::router(app.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket
        .write_all(
            format!("GET {URL} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer token\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let text = tokio::time::timeout(Duration::from_secs(3), async {
        let mut text = String::new();
        let mut bytes = [0; 8192];
        while !text.contains("\"queuedJobCount\":8") {
            let n = socket.read(&mut bytes).await.unwrap();
            assert!(n > 0);
            text.push_str(&String::from_utf8_lossy(&bytes[..n]));
            assert!(text.len() < 16384);
        }
        text
    })
    .await
    .unwrap();
    assert!(text.starts_with("HTTP/1.1 200"));
    assert!(
        text.contains("event: printer-observation") || text.contains("event:printer-observation")
    );
    assert_eq!(app.event_streams.available_permits(), 0);
    drop(socket);
    tokio::time::timeout(Duration::from_secs(3), async {
        while app.event_streams.available_permits() == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 0);
    server.abort();
    task.abort();
}
