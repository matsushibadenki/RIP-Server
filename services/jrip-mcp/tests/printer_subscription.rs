//! Printer notifications invalidate a cached resource; they do not query a device.
use axum::{Json, Router, extract::Query, http::StatusCode, response::IntoResponse};
use jrip_mcp::Session;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;
const URI: &str = "jrip://printers/test/observations";
#[derive(Default)]
struct Report {
    stream: Option<String>,
    events: Vec<i64>,
    pruned: i64,
    unavailable: bool,
    forbidden: bool,
    malformed: bool,
}
fn request(id: i64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}
fn initialize() -> Value {
    request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"},"capabilities":{}}),
    )
}
fn fixture() -> (Router, Arc<Mutex<Report>>, Arc<AtomicUsize>) {
    let report = Arc::new(Mutex::new(Report::default()));
    let stored = report.clone();
    let live = Arc::new(AtomicUsize::new(0));
    let queried = live.clone();
    let router=Router::new().route("/api/v1/printers/test/observations",axum::routing::get(move |Query(query):Query<HashMap<String,String>>| {let stored=stored.clone();async move {
        let report=stored.lock().unwrap();
        if report.forbidden {return (StatusCode::FORBIDDEN,Json(json!({"error":{"code":"forbidden"}}))).into_response()}
        if report.unavailable {return (StatusCode::BAD_REQUEST,Json(json!({"error":{"code":"printer_not_configured"}}))).into_response()}
        let after=query.get("after").and_then(|n|n.parse::<i64>().ok()).unwrap_or(0);
        let events:Vec<_>=report.events.iter().filter(|n|**n>after).take(100).map(|n|json!({"sequence":n})).collect();
        let mut value=json!({"profile":rip_storage::printer_observation::PROFILE,"streamId":report.stream,"events":events,"nextAfter":events.last().and_then(|e|e["sequence"].as_i64()).unwrap_or(after),"prunedThrough":report.pruned});
        if report.malformed {value.as_object_mut().unwrap().remove("nextAfter");}
        Json(value).into_response()
    }})).route("/api/v1/printers/test",axum::routing::get(move || {let queried=queried.clone();async move {queried.fetch_add(1,Ordering::SeqCst);StatusCode::INTERNAL_SERVER_ERROR}}));
    (router, report, live)
}
async fn ready(router: Router) -> Session {
    let mut session = Session::new(router, "reader".into()).with_access(
        "reader".into(),
        vec!["jobs:read".into()],
        None,
    );
    assert!(session.handle(initialize()).await.unwrap()["error"].is_null());
    session
        .handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;
    session
}
fn updated(values: Vec<Value>) {
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["method"], "notifications/resources/updated");
    assert_eq!(values[0]["params"]["uri"], URI);
}
#[tokio::test]
async fn printer_subscriptions_follow_stream_changes_pruning_and_availability_without_probing() {
    let (router, state, live) = fixture();
    let mut session = ready(router).await;
    assert!(
        session
            .handle(request(2, "resources/subscribe", json!({"uri":URI})))
            .await
            .unwrap()["error"]
            .is_null()
    );
    assert!(session.notifications().await.is_empty());
    let a = Uuid::new_v4().to_string();
    let b = Uuid::new_v4().to_string();
    {
        let mut r = state.lock().unwrap();
        r.stream = Some(a.clone());
        r.events = vec![10];
    }
    updated(session.notifications().await);
    assert!(session.notifications().await.is_empty());
    state.lock().unwrap().events.push(11);
    updated(session.notifications().await);
    // A new stream advances far beyond an older stream. Restoring the old stream must reset its cursor.
    {
        let mut r = state.lock().unwrap();
        r.stream = Some(b);
        r.events = vec![100];
    }
    updated(session.notifications().await);
    {
        let mut r = state.lock().unwrap();
        r.stream = Some(a);
        r.events = vec![10, 11, 12];
    }
    updated(session.notifications().await);
    assert!(session.notifications().await.is_empty());
    state.lock().unwrap().pruned = 11;
    updated(session.notifications().await);
    assert!(session.notifications().await.is_empty());
    state.lock().unwrap().unavailable = true;
    updated(session.notifications().await);
    assert!(session.notifications().await.is_empty());
    state.lock().unwrap().unavailable = false;
    updated(session.notifications().await);
    state.lock().unwrap().forbidden = true;
    state.lock().unwrap().events.push(13);
    assert!(session.notifications().await.is_empty());
    state.lock().unwrap().forbidden = false;
    updated(session.notifications().await);
    {
        let mut r = state.lock().unwrap();
        r.malformed = true;
        r.events.push(14);
    }
    assert!(session.notifications().await.is_empty());
    state.lock().unwrap().malformed = false;
    updated(session.notifications().await);
    let mut session = session.with_access("reader".into(), vec!["artifacts:read".into()], None);
    state.lock().unwrap().events.push(15);
    assert!(session.notifications().await.is_empty());
    assert_eq!(
        session
            .handle(request(3, "resources/subscribe", json!({"uri":URI})))
            .await
            .unwrap()["error"]["code"],
        -32001
    );
    let mut session = session.with_access("reader".into(), vec!["jobs:read".into()], None);
    updated(session.notifications().await);
    for uri in [
        "jrip://printers/test/observations/1",
        "jrip://printers/test/observations?after=1",
        "jrip://printers/../observations",
        "jrip://printers/test%2Fother/observations",
    ] {
        assert_eq!(
            session
                .handle(request(4, "resources/subscribe", json!({"uri":uri})))
                .await
                .unwrap()["error"]["code"],
            -32602
        );
    }
    session
        .handle(request(5, "resources/unsubscribe", json!({"uri":URI})))
        .await
        .unwrap();
    state.lock().unwrap().events.push(16);
    assert!(session.notifications().await.is_empty());
    assert_eq!(live.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn stdio_pushes_stored_printer_updates_without_more_client_requests_and_stops_on_unsubscribe()
{
    let (router, state, live) = fixture();
    let session = Session::new(router, "reader".into()).with_access(
        "reader".into(),
        vec!["jobs:read".into()],
        None,
    );
    let (client, server) = tokio::io::duplex(65536);
    let (server_read, server_write) = tokio::io::split(server);
    let worker = tokio::spawn(jrip_mcp::serve(
        BufReader::new(server_read),
        server_write,
        session,
    ));
    let (read, mut write) = tokio::io::split(client);
    let mut lines = BufReader::new(read).lines();
    write
        .write_all(format!("{}\n", initialize()).as_bytes())
        .await
        .unwrap();
    let first: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(first["id"], 1);
    write
        .write_all(
            format!(
                "{}\n{}\n",
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                request(2, "resources/subscribe", json!({"uri":URI}))
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let subscribed: Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(subscribed["id"], 2);
    {
        let mut r = state.lock().unwrap();
        r.stream = Some(Uuid::new_v4().to_string());
        r.events.push(1);
    }
    let notification = tokio::time::timeout(Duration::from_secs(3), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    updated(vec![serde_json::from_str(&notification).unwrap()]);
    assert!(
        tokio::time::timeout(Duration::from_millis(1100), lines.next_line())
            .await
            .is_err()
    );
    write
        .write_all(
            format!(
                "{}\n",
                request(3, "resources/unsubscribe", json!({"uri":URI}))
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let reply: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(reply["id"], 3);
    state.lock().unwrap().events.push(2);
    assert!(
        tokio::time::timeout(Duration::from_millis(1100), lines.next_line())
            .await
            .is_err()
    );
    assert_eq!(live.load(Ordering::SeqCst), 0);
    drop(write);
    worker.abort();
}
