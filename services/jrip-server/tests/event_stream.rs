use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
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
async fn event(body: &mut Body) -> (i64, Value) {
    let frame = tokio::time::timeout(std::time::Duration::from_secs(3), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let bytes = frame.into_data().unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    let id = text
        .lines()
        .find_map(|line| line.strip_prefix("id:").map(str::trim))
        .unwrap()
        .parse()
        .unwrap();
    let data = text
        .lines()
        .find_map(|line| line.strip_prefix("data:").map(str::trim))
        .unwrap();
    (id, serde_json::from_str(data).unwrap())
}
#[tokio::test]
async fn inspection_stream_replays_updates_and_reconnects_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    let id = Uuid::new_v4();
    app.repo
        .enqueue_inspection(id, "legacy-local", &"a".repeat(64), 1, Uuid::new_v4())
        .await
        .unwrap();
    let url = format!("/api/v1/print/document-inspection-requests/{id}/events/stream?after=0");
    let response = connect(&app, &url, "token", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(response.headers()["cache-control"], "no-store");
    let mut body = response.into_body();
    let (first, queued) = event(&mut body).await;
    assert_eq!(queued["state"], "QUEUED");
    app.repo
        .set_inspection_state(id, 0, "RUNNING", None, None)
        .await
        .unwrap();
    let (second, running) = event(&mut body).await;
    assert!(second > first);
    assert_eq!(running["state"], "RUNNING");
    drop(body);
    app.repo
        .set_inspection_state(id, 1, "FAILED", Some("TEST_FAILURE"), None)
        .await
        .unwrap();
    let reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    let mut body = connect(&reopened, &url, "token", Some(&second.to_string()))
        .await
        .into_body();
    let (third, failed) = event(&mut body).await;
    assert!(third > second);
    assert_eq!(failed["state"], "FAILED");
    assert_eq!(failed["reasonCode"], "TEST_FAILURE");
    assert_eq!(failed["reportAvailable"], false);
    drop(body);
    assert_eq!(reopened.event_streams.available_permits(), 64);
}
#[tokio::test]
async fn stream_enforces_owners_cursors_scopes_and_releases_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    app.credentials = vec![
        jrip_server::Credential {
            validity: None,
            token: "owner".into(),
            identity: "owner".into(),
            scopes: vec!["jobs:submit".into()],
        },
        jrip_server::Credential {
            validity: None,
            token: "other".into(),
            identity: "other".into(),
            scopes: vec!["jobs:submit".into()],
        },
        jrip_server::Credential {
            validity: None,
            token: "reader".into(),
            identity: "owner".into(),
            scopes: vec!["jobs:read".into()],
        },
    ];
    app.event_streams = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
    let id = Uuid::new_v4();
    app.repo
        .enqueue_inspection(id, "owner", &"a".repeat(64), 1, Uuid::new_v4())
        .await
        .unwrap();
    let url = format!("/api/v1/print/document-inspection-requests/{id}/events/stream");
    for (token, last, status) in [
        ("other", None, StatusCode::NOT_FOUND),
        ("reader", None, StatusCode::FORBIDDEN),
        ("wrong", None, StatusCode::UNAUTHORIZED),
        ("owner", Some("-1"), StatusCode::BAD_REQUEST),
        ("owner", Some("bad"), StatusCode::BAD_REQUEST),
    ] {
        assert_eq!(connect(&app, &url, token, last).await.status(), status);
        assert_eq!(app.event_streams.available_permits(), 1);
    }
    let response = connect(&app, &url, "owner", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        connect(&app, &url, "owner", None).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    drop(response);
    assert_eq!(app.event_streams.available_permits(), 1);
    assert_eq!(
        connect(&app, &format!("{url}?after=-1"), "owner", None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn pipeline_stream_drains_multiple_pages_without_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    let job:rip_core::Job=serde_json::from_value(json!({"id":Uuid::new_v4(),"manifest":{"name":"test.pdf","format":"application/pdf"},"state":"COMPLETED","source_sha256":"a".repeat(64),"created_at":0,"updated_at":0,"revision":0,"error_code":null,"selected_engine":null,"worker_run_id":null})).unwrap();
    app.repo.insert(&job).await.unwrap();
    let id = Uuid::new_v4();
    app.repo
        .create_pipeline(id, job.id, 0, json!({}))
        .await
        .unwrap();
    for revision in 0..105 {
        app.repo
            .update_pipeline(id, revision, "QUEUED", None, None)
            .await
            .unwrap();
    }
    let mut expected = vec![];
    let mut after = 0;
    loop {
        let page = app
            .repo
            .unified_pipeline_events(id, after, 100)
            .await
            .unwrap();
        expected.extend(page["events"].as_array().unwrap().iter().cloned());
        after = page["nextAfter"].as_i64().unwrap();
        if page["hasMore"] == false {
            break;
        }
    }
    let response = connect(
        &app,
        &format!("/api/v1/pipelines/{id}/timeline/stream"),
        "token",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let mut received = vec![];
    for _ in 0..expected.len() {
        let (sequence, value) = event(&mut body).await;
        assert_eq!(value["sequence"], sequence);
        received.push(value);
    }
    assert_eq!(received, expected);
}

#[tokio::test]
async fn expiring_credentials_reject_rest_and_stop_buffered_events() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), "legacy-token".into())
        .await
        .unwrap();
    let now = rip_storage::now();
    let expires = now + 5;
    app.credentials = vec![jrip_server::Credential {
        token: "short-lived-token-at-least-24-characters".into(),
        identity: "owner".into(),
        scopes: vec!["jobs:submit".into()],
        validity: Some(jrip_server::CredentialValidity {
            not_before: now - 1,
            expires_at: expires,
        }),
    }];
    let id = Uuid::new_v4();
    app.repo
        .enqueue_inspection(id, "owner", &"a".repeat(64), 1, Uuid::new_v4())
        .await
        .unwrap();
    let token = &app.credentials[0].token;
    let url = format!("/api/v1/print/document-inspection-requests/{id}/events/stream?after=0");
    let response = connect(&app, &url, token, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    // connect already loaded a durable event page; do not poll it before expiry.
    tokio::time::sleep(std::time::Duration::from_secs(
        (expires - rip_storage::now()).max(0) as u64 + 1,
    ))
    .await;
    let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let text = std::str::from_utf8(&frame).unwrap();
    assert!(text.contains("AUTHORIZATION_EXPIRED"), "{text}");
    assert!(!text.contains("QUEUED"));
    assert!(body.frame().await.is_none());
    drop(body);
    assert_eq!(app.event_streams.available_permits(), 64);
    assert_eq!(
        connect(&app, &url, token, None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    // Expiration does not allow fallback to the legacy admin token.
    assert_eq!(
        connect(&app, &url, "legacy-token", None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.repo.inspection_request(id, "owner").await.unwrap()["state"],
        "QUEUED"
    );
}
