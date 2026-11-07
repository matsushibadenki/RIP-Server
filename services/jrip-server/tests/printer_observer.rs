use axum::{
    Router,
    body::{Body, Bytes},
    extract::Path,
    http::{Request, StatusCode},
    routing::post,
};
use http_body_util::BodyExt;
use jrip_server::printer_observer::{self, Config};
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;
const TOKEN: &str = "observer-test-token-at-least-24-characters";
struct Device {
    queries: AtomicUsize,
    level: AtomicUsize,
    mode: AtomicUsize,
    release: tokio::sync::Notify,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
struct Fake {
    base: String,
    devices: Arc<Vec<Device>>,
    maximum: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fake {
    fn drop(&mut self) {
        for d in self.devices.iter() {
            d.release.notify_waiters();
        }
        self.task.abort();
    }
}
async fn make_fake(count: usize) -> Fake {
    let devices = Arc::new(
        (0..count)
            .map(|_| Device {
                queries: AtomicUsize::new(0),
                level: AtomicUsize::new(0),
                mode: AtomicUsize::new(0),
                release: tokio::sync::Notify::new(),
            })
            .collect::<Vec<_>>(),
    );
    let data = devices.clone();
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let max = maximum.clone();
    let router = Router::new().route(
        "/{index}",
        post(move |Path(index): Path<usize>, bytes: Bytes| {
            let data = data.clone();
            let active = active.clone();
            let max = max.clone();
            async move {
                let request = rip_ipp::Request::parse(&bytes).unwrap();
                assert_eq!(
                    request.operation,
                    rip_ipp::GET_PRINTER_ATTRIBUTES,
                    "observer must never print or cancel"
                );
                let device = &data[index];
                device.queries.fetch_add(1, Ordering::SeqCst);
                let running = active.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(running, Ordering::SeqCst);
                let _active = Active(active);
                match device.mode.load(Ordering::SeqCst) {
                    1 => tokio::time::sleep(Duration::from_millis(120)).await,
                    2 => device.release.notified().await,
                    _ => {}
                }
                if device.mode.load(Ordering::SeqCst) == 3 {
                    return (
                        [(axum::http::header::CONTENT_TYPE, "application/ipp")],
                        b"malformed".to_vec(),
                    );
                }
                let mut response = rip_ipp::Response::new([1, 1], rip_ipp::OK, request.request_id);
                response.text(0x47, "attributes-charset", "utf-8");
                response.text(0x48, "attributes-natural-language", "en");
                response.group(4);
                response.text(0x49, "document-format-supported", "application/pdf");
                response.attribute(0x22, "printer-is-accepting-jobs", &[1]);
                response.attribute(0x23, "printer-state", &3i32.to_be_bytes());
                response.integer("marker-levels", device.level.load(Ordering::SeqCst) as i32);
                (
                    [(axum::http::header::CONTENT_TYPE, "application/ipp")],
                    response.finish(),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Fake {
        base: format!("ipp://{address}"),
        devices,
        maximum,
        task,
    }
}
fn printers(fake: &Fake) -> Vec<jrip_server::delivery::Printer> {
    (0..fake.devices.len())
        .map(|i| jrip_server::delivery::Printer {
            id: format!("p{i}"),
            uri: format!("{}/{i}", fake.base),
            media: vec![jrip_server::delivery::Media {
                name: "iso_a4_210x297mm".into(),
                width_microns: 210000,
                height_microns: 297000,
            }],
            color_target: None,
        })
        .collect()
}
async fn make_app(fake: &Fake) -> (tempfile::TempDir, jrip_server::App) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    app.printers = printers(fake);
    (dir, app)
}
async fn history(app: &jrip_server::App, index: usize) -> Value {
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::get(format!("/api/v1/printers/p{index}/observations"))
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
#[tokio::test]
async fn once_bounds_fanout_and_restart_does_not_bypass_cooldown() {
    let fake = make_fake(4).await;
    for d in fake.devices.iter() {
        d.mode.store(1, Ordering::SeqCst);
    }
    let (_dir, app) = make_app(&fake).await;
    let config = Config::new(5, 2, 20).unwrap();
    let report = printer_observer::observe_once(&app, config).await;
    assert_eq!(report.successful, 4);
    assert_eq!(report.storage_errors, 0);
    assert_eq!(fake.maximum.load(Ordering::SeqCst), 2);
    for d in fake.devices.iter() {
        assert_eq!(d.queries.load(Ordering::SeqCst), 1);
    }
    let before = history(&app, 0).await;
    assert_eq!(
        before["latest"]["snapshot"]["markerSupplies"]["markers"][0]["levelPercent"],
        0
    );
    let mut reopened = jrip_server::open(app.root.clone(), TOKEN.into())
        .await
        .unwrap();
    reopened.printers = app.printers.clone();
    let skipped = printer_observer::observe_once(&reopened, config).await;
    assert_eq!(skipped.attempted, 0);
    assert_eq!(skipped.skipped, 4);
    assert_eq!(history(&reopened, 0).await["streamId"], before["streamId"]);
}
#[tokio::test]
async fn competing_workers_do_not_duplicate_queries_and_failures_back_off() {
    let fake = make_fake(2).await;
    for d in fake.devices.iter() {
        d.mode.store(1, Ordering::SeqCst);
    }
    let (_dir, app) = make_app(&fake).await;
    let config = Config::new(5, 2, 20).unwrap();
    let (a, b) = tokio::join!(
        printer_observer::observe_once(&app, config),
        printer_observer::observe_once(&app, config)
    );
    assert_eq!(a.attempted + b.attempted, 2);
    for d in fake.devices.iter() {
        assert_eq!(d.queries.load(Ordering::SeqCst), 1);
    }
    let bad = make_fake(1).await;
    bad.devices[0].mode.store(3, Ordering::SeqCst);
    let (_dir, app) = make_app(&bad).await;
    let failed = printer_observer::observe_once(&app, config).await;
    assert_eq!(failed.failed, 1);
    assert_eq!(
        history(&app, 0).await["latest"]["queryFailure"],
        "INVALID_RESPONSE"
    );
    assert_eq!(
        printer_observer::observe_once(&app, config).await.skipped,
        1
    );
    assert_eq!(bad.devices[0].queries.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn continuous_worker_keeps_healthy_device_fresh_while_a_peer_is_slow_and_stops_promptly() {
    let fake = make_fake(2).await;
    fake.devices[0].mode.store(2, Ordering::SeqCst);
    let (_dir, app) = make_app(&fake).await;
    let worker_app = app.clone();
    let (stop, shutdown) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        printer_observer::run_until(&worker_app, Config::new(5, 2, 20).unwrap(), async {
            let _ = shutdown.await;
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if history(&app, 1).await["latest"]["revision"] == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    fake.devices[1].level.store(25, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(8),async {loop {if history(&app,1).await["latest"]["snapshot"]["markerSupplies"]["markers"][0]["levelPercent"]==25 {break}tokio::time::sleep(Duration::from_millis(50)).await;}}).await.unwrap();
    assert!(fake.devices[1].queries.load(Ordering::SeqCst) >= 2);
    assert_eq!(fake.devices[0].queries.load(Ordering::SeqCst), 1);
    assert!(fake.maximum.load(Ordering::SeqCst) <= 2);
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        history(&app, 1).await["events"].as_array().unwrap().len(),
        2
    );
}
#[tokio::test]
async fn observer_binary_once_reports_json_and_preserves_schedule_between_processes() {
    let fake = make_fake(1).await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("printers.json");
    std::fs::write(&config, serde_json::to_vec(&printers(&fake)).unwrap()).unwrap();
    let root = dir.path().join("state");
    let command = || {
        let mut c = tokio::process::Command::new(env!("CARGO_BIN_EXE_jrip-printer-observer"));
        c.env_clear()
            .env("JRIP_DATA_ROOT", &root)
            .env("JRIP_API_TOKEN", TOKEN)
            .env("JRIP_PRINTER_CONFIG", &config)
            .env("JRIP_PRINTER_OBSERVER_INTERVAL_SECONDS", "5")
            .env("JRIP_PRINTER_OBSERVER_MAX_BACKOFF_SECONDS", "20")
            .kill_on_drop(true);
        c
    };
    let output = command().arg("--once").output().await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let first: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(first["successful"], 1);
    let output = command().arg("--once").output().await.unwrap();
    assert!(output.status.success());
    let next: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(next["skipped"], 1);
    assert_eq!(fake.devices[0].queries.load(Ordering::SeqCst), 1);
    let output = command()
        .env("JRIP_PRINTER_OBSERVER_INTERVAL_SECONDS", "4")
        .arg("--once")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("観測間隔"));
    assert_eq!(fake.devices[0].queries.load(Ordering::SeqCst), 1);
}
