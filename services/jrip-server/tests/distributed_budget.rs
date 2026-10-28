use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use rip_storage::budget::{DEFAULT_CAPACITY, SLOT};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
const A: &str = "node-a-token-at-least-24-characters";
const B: &str = "node-b-token-at-least-24-characters";
const ADMIN: &str = "admin-token-at-least-24-characters";
const READER: &str = "reader-token-at-least-24-characters";
async fn call(
    app: &jrip_server::App,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(if method == "GET" {
                    Body::empty()
                } else {
                    Body::from(body.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let code = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (code, serde_json::from_slice(&bytes).unwrap())
}
async fn client(root: &std::path::Path, url: &str, token: &str) -> (jrip_server::App, String) {
    let mut app = jrip_server::open(root.into(), "local".into())
        .await
        .unwrap();
    app.snapshot_coordinator =
        Some(rip_storage::budget_client::SnapshotCoordinator::new(url, token).unwrap());
    let job:rip_core::Job=serde_json::from_value(json!({"id":Uuid::new_v4(),"manifest":{"name":"test.pdf","format":"application/pdf"},"state":"SPOOLING","source_sha256":"a".repeat(64),"created_at":rip_storage::now(),"updated_at":rip_storage::now(),"revision":0,"error_code":null,"selected_engine":null,"worker_run_id":null})).unwrap();
    app.repo.insert(&job).await.unwrap();
    let directory = app
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("raster");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("page-000001.tiff");
    std::fs::write(&path, b"test snapshot bytes").unwrap();
    let artifacts = jrip_server::artifacts::hash_pages(&[path]).await.unwrap();
    app.repo
        .complete_with_artifacts(job.clone(), &artifacts)
        .await
        .unwrap();
    (
        app,
        format!("/api/v1/jobs/{}/artifacts/page-000001.tiff", job.id),
    )
}
async fn download(app: &jrip_server::App, path: &str) -> axum::response::Response {
    jrip_server::router(app.clone())
        .oneshot(
            Request::get(path)
                .header("authorization", "Bearer local")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}
async fn wait_held(app: &jrip_server::App, bytes: u64) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if app.repo.snapshot_budget_status(None, 100).await.unwrap()["heldBytes"] == bytes {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
#[tokio::test]
async fn independent_roots_share_http_budget_and_release_after_disconnect_or_hash_failure() {
    let dir = tempfile::tempdir().unwrap();
    let mut central = jrip_server::open(dir.path().join("central"), "unused".into())
        .await
        .unwrap();
    central.credentials = vec![
        (A, "node-a", "budgets:reserve"),
        (B, "node-b", "budgets:reserve"),
        (ADMIN, "operator", "admin"),
        (READER, "reader", "jobs:read"),
    ]
    .into_iter()
    .map(|(token, identity, scope)| jrip_server::Credential {
        validity: None,
        token: token.into(),
        identity: identity.into(),
        scopes: vec![scope.into()],
    })
    .collect();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = jrip_server::router(central.clone());
    let server = Server(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    }));
    let url = format!("http://{address}/api/v1/budgets/snapshots/");
    let (a, path_a) = client(&dir.path().join("a"), &url, A).await;
    let (b, path_b) = client(&dir.path().join("b"), &url, B).await;
    let base = "/api/v1/budgets/snapshots";
    let block = Uuid::new_v4();
    assert_eq!(
        call(
            &central,
            "POST",
            &format!("{base}/reservations"),
            READER,
            json!({"requestId":block,"bytes":1})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&central, "GET", base, A, Value::Null).await.0,
        StatusCode::FORBIDDEN
    );
    let (code, held) = call(
        &central,
        "POST",
        &format!("{base}/reservations"),
        ADMIN,
        json!({"requestId":block,"bytes":DEFAULT_CAPACITY-SLOT}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        call(
            &central,
            "POST",
            &format!("{base}/reservations/{block}/release"),
            B,
            json!({"expectedBytes":DEFAULT_CAPACITY-SLOT})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let response_a = download(&a, &path_a).await;
    assert_eq!(response_a.status(), StatusCode::OK);
    wait_held(&central, DEFAULT_CAPACITY).await;
    let denied = download(&b, &path_b).await;
    assert_eq!(denied.status(), StatusCode::SERVICE_UNAVAILABLE);
    let error: Value =
        serde_json::from_slice(&denied.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(error["error"]["code"], "artifact_budget_exhausted");
    assert_eq!(
        std::fs::read_dir(b.root.join("artifact-snapshots"))
            .unwrap()
            .count(),
        0
    );
    // Disconnect without consuming the body: unlink first, then release remotely.
    drop(response_a);
    wait_held(&central, DEFAULT_CAPACITY - SLOT).await;
    assert_eq!(
        std::fs::read_dir(a.root.join("artifact-snapshots"))
            .unwrap()
            .count(),
        0
    );
    let response_b = download(&b, &path_b).await;
    assert_eq!(response_b.status(), StatusCode::OK);
    assert_eq!(
        &response_b.into_body().collect().await.unwrap().to_bytes()[..],
        b"test snapshot bytes"
    );
    wait_held(&central, DEFAULT_CAPACITY - SLOT).await;
    let invalid_range = jrip_server::router(b.clone())
        .oneshot(
            Request::get(&path_b)
                .header("authorization", "Bearer local")
                .header("range", "bytes=0-0,2-3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_range.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    drop(invalid_range);
    wait_held(&central, DEFAULT_CAPACITY - SLOT).await;
    assert_eq!(
        std::fs::read_dir(b.root.join("artifact-snapshots"))
            .unwrap()
            .count(),
        0
    );
    let stored = std::fs::read_dir(b.root.join("jobs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("raster/page-000001.tiff");
    std::fs::write(stored, b"bad! snapshot bytes").unwrap();
    assert_eq!(download(&b, &path_b).await.status(), StatusCode::CONFLICT);
    wait_held(&central, DEFAULT_CAPACITY - SLOT).await;
    assert_eq!(
        std::fs::read_dir(b.root.join("artifact-snapshots"))
            .unwrap()
            .count(),
        0
    );
    // Simulated crashed owner: its durable reservation survives coordinator reopen.
    let reopened = jrip_server::open(central.root.clone(), "unused".into())
        .await
        .unwrap();
    assert_eq!(
        reopened
            .repo
            .snapshot_budget_status(None, 100)
            .await
            .unwrap()["heldBytes"],
        DEFAULT_CAPACITY - SLOT
    );
    let reconcile = format!("{base}/reservations/{block}/reconcile");
    assert_eq!(call(&central,"POST",&reconcile,ADMIN,json!({"expectedOwner":"operator","expectedBytes":held["bytes"],"cleanupConfirmed":false})).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(call(&central,"POST",&reconcile,A,json!({"expectedOwner":"operator","expectedBytes":held["bytes"],"cleanupConfirmed":true})).await.0,StatusCode::FORBIDDEN);
    assert_eq!(call(&central,"POST",&reconcile,ADMIN,json!({"expectedOwner":"operator","expectedBytes":held["bytes"],"cleanupConfirmed":true})).await.0,StatusCode::OK);
    wait_held(&central, 0).await;
    let audit = central
        .repo
        .audit_records(0, 100)
        .await
        .unwrap()
        .to_string();
    assert!(audit.contains("reconcile"));
    assert!(!audit.contains(ADMIN));
    #[cfg(unix)]
    {
        // Make the original cleanup path disappear while the actual file survives.
        let snapshot = download(&a, &path_a).await;
        assert_eq!(snapshot.status(), StatusCode::OK);
        wait_held(&central, SLOT).await;
        let state = central
            .repo
            .snapshot_budget_status(None, 100)
            .await
            .unwrap();
        let reservation = &state["reservations"][0];
        let original = a.root.join("artifact-snapshots");
        let moved = a.root.join("moved-snapshots");
        std::fs::rename(&original, &moved).unwrap();
        drop(snapshot);
        assert_eq!(
            central
                .repo
                .snapshot_budget_status(None, 100)
                .await
                .unwrap()["heldBytes"],
            SLOT
        );
        let file = std::fs::read_dir(&moved)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(
            file.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains(reservation["id"].as_str().unwrap())
        );
        std::fs::remove_file(file).unwrap();
        std::fs::rename(moved, original).unwrap();
        assert_eq!(
            call(
                &central,
                "POST",
                &format!(
                    "{base}/reservations/{}/reconcile",
                    reservation["id"].as_str().unwrap()
                ),
                ADMIN,
                json!({"expectedOwner":"node-a","expectedBytes":SLOT,"cleanupConfirmed":true})
            )
            .await
            .0,
            StatusCode::OK
        );
        wait_held(&central, 0).await;
    }
    // Release ACK loss retains quota, even though the local file was deleted.
    let in_flight = download(&a, &path_a).await;
    assert_eq!(in_flight.status(), StatusCode::OK);
    wait_held(&central, SLOT).await;
    server.0.abort();
    while !server.0.is_finished() {
        tokio::task::yield_now().await;
    }
    drop(server);
    drop(in_flight);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        std::fs::read_dir(a.root.join("artifact-snapshots"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        central
            .repo
            .snapshot_budget_status(None, 100)
            .await
            .unwrap()["heldBytes"],
        SLOT
    );
    // An unreachable configured authority must never fall back to local capacity.
    let denied = download(&a, &path_a).await;
    assert_eq!(denied.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        String::from_utf8(
            denied
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec()
        )
        .unwrap()
        .contains("snapshot_coordinator_unavailable")
    );
}
