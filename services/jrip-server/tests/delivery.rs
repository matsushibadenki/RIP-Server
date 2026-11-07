use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    http::{Request, StatusCode},
    routing::post,
};
use http_body_util::BodyExt;
use rip_core::{Artifact, Job, JobState, Manifest};
use rip_storage::delivery::DeliveryState;
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires locally built jrip-color:local Docker image"]
async fn color_managed_delivery_requires_matching_target_and_binds_attempt() {
    let ranges = serde_json::from_value(serde_json::json!([{"first":2,"last":2}])).unwrap();
    let (_dir, mut app, job, fake, task) = fixture_with_source_hash(
        false,
        1,
        Some((ranges, 2)),
        &format!("{:x}", Sha256::digest(b"source")),
    )
    .await;
    let icc = lcms2::Profile::new_srgb().icc().unwrap();
    let record = rip_color::registry::Registry::open(app.root.join("color-profiles"))
        .unwrap()
        .register(&icc)
        .unwrap();
    let color_id = Uuid::new_v4();
    let request = serde_json::json!({"requestId":color_id,"expectedRevision":job.revision,"inputProfileSha256":record.sha256,"outputProfileSha256":record.sha256,"policy":{"schemaVersion":1,"inputColor":"rgb","outputColor":"rgb","intent":"relative_colorimetric","blackPointCompensation":false,"blackPreservation":"none","tacPercent":null}});
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::post(format!("/api/v1/jobs/{}/color-attempts", job.id))
                .header("authorization", "Bearer token")
                .header("content-type", "application/json")
                .body(Body::from(request.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    jrip_server::color::tick(&app).await.unwrap();
    assert_eq!(
        app.repo.color_attempt(color_id).await.unwrap()["state"],
        "COMPLETED"
    );
    assert_eq!(
        app.repo.color_attempt(color_id).await.unwrap()["result"]["reports"][0]["warnings"][0]["code"],
        "SOURCE_PROFILE_UNEMBEDDED"
    );
    let result = app.repo.color_attempt(color_id).await.unwrap();
    assert_eq!(result["result"]["artifacts"][0]["sourcePage"], 2);
    assert_eq!(result["result"]["artifacts"][0]["sourcePageCount"], 2);
    let delivery_id = Uuid::new_v4();
    let body = serde_json::json!({"requestId":delivery_id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm","colorAttemptId":color_id});
    async fn send(app: &jrip_server::App, job: &Job, body: &serde_json::Value) -> StatusCode {
        let response = jrip_server::router(app.clone())
            .oneshot(
                Request::post(format!("/api/v1/jobs/{}/deliveries", job.id))
                    .header("authorization", "Bearer token")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        if status != StatusCode::ACCEPTED {
            eprintln!(
                "{}",
                String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
            );
        }
        status
    }
    assert_eq!(send(&app, &job, &body).await, StatusCode::BAD_REQUEST);
    app.printers[0].color_target = Some(jrip_server::delivery::ColorTarget {
        output_profile_sha256: Some("b".repeat(64)),
        device_link_sha256: None,
    });
    assert_eq!(send(&app, &job, &body).await, StatusCode::BAD_REQUEST);
    app.printers[0]
        .color_target
        .as_mut()
        .unwrap()
        .output_profile_sha256 = Some(record.sha256);
    let mut raw = body.clone();
    raw.as_object_mut().unwrap().remove("colorAttemptId");
    assert_eq!(send(&app, &job, &raw).await, StatusCode::BAD_REQUEST);
    app.credentials = vec![jrip_server::Credential {
        validity: None,
        token: "token".into(),
        identity: "operator".into(),
        scopes: vec!["admin".into()],
    }];
    app.require_print_approval = true;
    assert_eq!(send(&app, &job, &body).await, StatusCode::FORBIDDEN);
    async fn grant(app: &jrip_server::App, job: &Job, request: Uuid, color: Option<Uuid>) -> Uuid {
        let payload = serde_json::json!({"identity":"operator","jobId":job.id,"requestId":request,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm","reason":"test color output","colorAttemptId":color});
        let response = jrip_server::router(app.clone())
            .oneshot(
                Request::post("/api/v1/approvals")
                    .header("authorization", "Bearer token")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let value: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        Uuid::parse_str(value["id"].as_str().unwrap()).unwrap()
    }
    async fn approved(
        app: &jrip_server::App,
        job: &Job,
        body: &serde_json::Value,
        grant: Uuid,
    ) -> StatusCode {
        jrip_server::router(app.clone())
            .oneshot(
                Request::post(format!("/api/v1/jobs/{}/deliveries", job.id))
                    .header("authorization", "Bearer token")
                    .header("x-jrip-approval-id", grant.to_string())
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }
    let raw_grant = grant(&app, &job, delivery_id, None).await;
    assert_eq!(
        approved(&app, &job, &body, raw_grant).await,
        StatusCode::FORBIDDEN
    );
    let color_grant = grant(&app, &job, delivery_id, Some(color_id)).await;
    assert_eq!(
        approved(&app, &job, &body, color_grant).await,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        app.repo
            .delivery(delivery_id)
            .await
            .unwrap()
            .ticket
            .color_attempt_id,
        Some(color_id)
    );
    assert_eq!(send(&app, &job, &body).await, StatusCode::ACCEPTED);
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::get(format!(
                "/api/v1/deliveries/{delivery_id}/artifact/descriptor"
            ))
            .header("authorization", "Bearer token")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let descriptor: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(
        descriptor["provenance"]["colorAttemptId"],
        color_id.to_string()
    );
    assert_eq!(descriptor["selectedSourcePages"], serde_json::json!([2]));
    assert_eq!(descriptor["pageCount"], 1);
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    let download = jrip_server::router(app.clone())
        .oneshot(
            Request::get(format!(
                "/api/v1/color/attempts/{color_id}/artifacts/page-000001.tiff"
            ))
            .header("authorization", "Bearer token")
            .header("range", "bytes=0-3")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        download
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .len(),
        4
    );
    std::fs::write(
        app.root
            .join("jobs")
            .join(job.id.to_string())
            .join("color")
            .join(color_id.to_string())
            .join("page-000001.tiff"),
        b"tampered",
    )
    .unwrap();
    let mut changed = body;
    changed["requestId"] = serde_json::json!(Uuid::new_v4());
    app.require_print_approval = false;
    assert_eq!(send(&app, &job, &changed).await, StatusCode::CONFLICT);
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    task.abort();
}
#[derive(Clone)]
struct Fake {
    document: Arc<std::sync::Mutex<Vec<u8>>>,
    copies: Arc<AtomicUsize>,
    copies_max: Arc<AtomicUsize>,
    duplex: Arc<AtomicUsize>,
    prints: Arc<AtomicUsize>,
    state: Arc<AtomicUsize>,
    progress_mode: Arc<AtomicUsize>,
    queue_count: Arc<AtomicUsize>,
    accepting_jobs: Arc<AtomicUsize>,
    printer_state: Arc<AtomicUsize>,
    marker_mode: Arc<AtomicUsize>,
    pwg_supply_mode: Arc<AtomicUsize>,
    capability_queries: Arc<AtomicUsize>,
    printer_reasons: Arc<std::sync::Mutex<Vec<String>>>,
    reasons: Arc<std::sync::Mutex<Vec<String>>>,
    malformed: bool,
    job_name: Arc<std::sync::Mutex<String>>,
}
fn fake_back(mode: usize) -> &'static str {
    match mode {
        1 => "normal",
        2 => "rotated",
        4 => "flipped",
        5 => "manual-tumble",
        _ => "unknown",
    }
}
async fn handle(
    State(f): State<Fake>,
    bytes: Bytes,
) -> ([(axum::http::header::HeaderName, &'static str); 1], Vec<u8>) {
    let q = rip_ipp::Request::parse(&bytes).unwrap();
    let mut r = rip_ipp::Response::new([1, 1], 0, q.request_id);
    r.text(0x47, "attributes-charset", "utf-8");
    r.text(0x48, "attributes-natural-language", "en");
    match q.operation {
        rip_ipp::GET_PRINTER_ATTRIBUTES => {
            f.capability_queries.fetch_add(1, Ordering::SeqCst);
            r.group(4);
            let mut range = 1i32.to_be_bytes().to_vec();
            range.extend_from_slice(&(f.copies_max.load(Ordering::SeqCst) as i32).to_be_bytes());
            r.attribute(0x33, "copies-supported", &range);
            if f.duplex.load(Ordering::SeqCst) > 0 {
                r.text(0x44, "sides-supported", "one-sided");
                r.additional(0x44, b"two-sided-long-edge");
                r.additional(0x44, b"two-sided-short-edge");
                r.text(
                    0x44,
                    "pwg-raster-document-sheet-back",
                    fake_back(f.duplex.load(Ordering::SeqCst)),
                );
            }
            r.text(0x49, "document-format-supported", "image/pwg-raster");
            r.attribute(
                0x22,
                "printer-is-accepting-jobs",
                &[u8::from(f.accepting_jobs.load(Ordering::SeqCst) != 0)],
            );
            r.attribute(
                0x23,
                "printer-state",
                &(f.printer_state.load(Ordering::SeqCst) as i32).to_be_bytes(),
            );
            for (index, reason) in f.printer_reasons.lock().unwrap().iter().enumerate() {
                if index == 0 {
                    r.text(0x44, "printer-state-reasons", reason)
                } else {
                    r.additional(0x44, reason.as_bytes())
                }
            }
            match f.queue_count.load(Ordering::SeqCst) {
                usize::MAX => {}
                n if n == usize::MAX - 1 => r.integer("queued-job-count", -1),
                n => r.integer("queued-job-count", n as i32),
            }
            match f.marker_mode.load(Ordering::SeqCst) {
                1 => {
                    r.integer("marker-levels", 0);
                    r.additional(0x21, &90i32.to_be_bytes());
                    r.text(0x42, "marker-names", "Black toner");
                    r.additional(0x42, b"Waste tank");
                    r.text(0x44, "marker-types", "toner");
                    r.additional(0x44, b"waste-ink");
                    r.integer("marker-high-levels", 100);
                    r.additional(0x21, &0i32.to_be_bytes());
                    r.integer("marker-low-levels", 5);
                    r.additional(0x21, &95i32.to_be_bytes());
                }
                2 => {
                    r.integer("marker-levels", -1);
                    r.additional(0x21, &(-2i32).to_be_bytes());
                    r.additional(0x21, &(-3i32).to_be_bytes());
                }
                3 => {
                    r.integer("marker-levels", 1);
                    r.additional(0x21, &2i32.to_be_bytes());
                    r.text(0x42, "marker-names", "Only one");
                }
                4 => r.attribute(0x12, "marker-levels", &[]),
                _ => {}
            }
            match f.pwg_supply_mode.load(Ordering::SeqCst) {
                1=>{
                    r.attribute(0x30,"printer-supply",b"type=toner;maxcapacity=100;level=0;colorantname=black;unit=percent;class=supplyThatIsConsumed;");
                    let text="黒トナー".as_bytes();let mut d=2u16.to_be_bytes().to_vec();d.extend_from_slice(b"ja");d.extend_from_slice(&(text.len() as u16).to_be_bytes());d.extend_from_slice(text);r.attribute(0x35,"printer-supply-description",&d);
                },
                2=>r.attribute(0x30,"printer-supply",b"type=staples;maxcapacity=5000;level=2000;colorantname=no-color;unit=items;"),
                3=>r.attribute(0x30,"printer-supply",b"type=ink;maxcapacity=-2;level=-3;colorantname=cyan;"),
                4=>r.attribute(0x30,"printer-supply",b"type=toner;maxcapacity=100;level=10;Level=20;colorantname=black;"),
                5=>{r.attribute(0x30,"printer-supply",b"type=toner;maxcapacity=100;level=10;colorantname=black;");r.text(0x41,"printer-supply-description","One");r.additional(0x41,b"Extra");},
                6=>r.attribute(0x12,"printer-supply",&[]),
                7=>r.attribute(0x30,"printer-supply",b"type=wasteToner;maxcapacity=100;level=25;colorantname=no-color;class=receptacleThatIsFilled;unit=percent;"),
                _=>{},
            }
            r.resolution_dpi("pwg-raster-document-resolution-supported", 72);
            r.text(0x44, "pwg-raster-document-type-supported", "rgb_8");
            r.text(0x44, "media-supported", "custom_square_25.4x25.4mm");
        }
        rip_ipp::PRINT_JOB => {
            let copies = q.integer(2, "copies").unwrap();
            assert!((1..=f.copies_max.load(Ordering::SeqCst) as i32).contains(&copies));
            f.copies.store(copies as usize, Ordering::SeqCst);
            assert_eq!(q.one(1, "ipp-attribute-fidelity").unwrap().value, [1]);
            assert_eq!(q.text(1, "document-format", 0x49), Some("image/pwg-raster"));
            assert_eq!(q.text(2, "media", 0x44), Some("custom_square_25.4x25.4mm"));
            let sides = q.text(2, "sides", 0x44).unwrap_or("one-sided");
            let pages = match sides {
                "one-sided" => rip_raster::inspect(q.document),
                "two-sided-long-edge" => rip_raster::inspect_duplex(
                    q.document,
                    rip_raster::DuplexLayout {
                        binding: rip_raster::DuplexBinding::LongEdge,
                        sheet_back: fake_back(f.duplex.load(Ordering::SeqCst)).parse().unwrap(),
                    },
                ),
                "two-sided-short-edge" => rip_raster::inspect_duplex(
                    q.document,
                    rip_raster::DuplexLayout {
                        binding: rip_raster::DuplexBinding::ShortEdge,
                        sheet_back: fake_back(f.duplex.load(Ordering::SeqCst)).parse().unwrap(),
                    },
                ),
                _ => panic!("unexpected sides"),
            }
            .unwrap();
            if sides != "one-sided" {
                assert!(matches!(f.duplex.load(Ordering::SeqCst), 1 | 2 | 4 | 5));
            }
            assert!(!pages.is_empty());
            *f.document.lock().unwrap() = q.document.to_vec();
            *f.job_name.lock().unwrap() = q.text(1, "job-name", 0x42).unwrap().to_string();
            f.prints.fetch_add(1, Ordering::SeqCst);
            if f.malformed {
                return (
                    [(axum::http::header::CONTENT_TYPE, "application/ipp")],
                    vec![0],
                );
            }
            r.group(2);
            r.integer("job-id", 42);
            r.attribute(0x23, "job-state", &3i32.to_be_bytes());
        }
        rip_ipp::GET_JOB_ATTRIBUTES => {
            if f.progress_mode.load(Ordering::SeqCst) == 9 {
                return (
                    [(axum::http::header::CONTENT_TYPE, "application/ipp")],
                    vec![0],
                );
            }
            if f.progress_mode.load(Ordering::SeqCst) == 10 {
                let mut response = rip_ipp::Response::new([1, 1], rip_ipp::NOT_FOUND, q.request_id);
                response.text(0x47, "attributes-charset", "utf-8");
                response.text(0x48, "attributes-natural-language", "en");
                return (
                    [(axum::http::header::CONTENT_TYPE, "application/ipp")],
                    response.finish(),
                );
            }
            r.group(2);
            r.integer("job-id", 42);
            r.text(0x42, "job-name", &f.job_name.lock().unwrap());
            r.attribute(
                0x23,
                "job-state",
                &(f.state.load(Ordering::SeqCst) as i32).to_be_bytes(),
            );
            let (impressions, sheets) = match f.state.load(Ordering::SeqCst) {
                5 => (3, 2),
                6 => (4, 2),
                9 => (8, 4),
                _ => (0, 0),
            };
            let progress_mode = f.progress_mode.load(Ordering::SeqCst);
            if progress_mode != 1 {
                r.integer(
                    "job-impressions-completed",
                    if progress_mode == 2 { 0 } else { impressions },
                );
                if progress_mode != 3 {
                    r.integer(
                        "job-media-sheets-completed",
                        if progress_mode == 2 { 0 } else { sheets },
                    );
                }
            }
            for (index, reason) in f.reasons.lock().unwrap().iter().enumerate() {
                if index == 0 {
                    r.text(0x44, "job-state-reasons", reason);
                } else {
                    r.additional(0x44, reason.as_bytes());
                }
            }
        }
        rip_ipp::CANCEL_JOB => {
            assert_eq!(q.integer(1, "job-id"), Some(42));
            f.state.store(7, Ordering::SeqCst);
        }
        _ => panic!("unexpected request"),
    }
    (
        [(axum::http::header::CONTENT_TYPE, "application/ipp")],
        r.finish(),
    )
}
async fn fixture(
    malformed: bool,
) -> (
    tempfile::TempDir,
    jrip_server::App,
    Job,
    Fake,
    tokio::task::JoinHandle<()>,
) {
    fixture_pages(malformed, 1).await
}
async fn fixture_pages(
    malformed: bool,
    count: u32,
) -> (
    tempfile::TempDir,
    jrip_server::App,
    Job,
    Fake,
    tokio::task::JoinHandle<()>,
) {
    fixture_mapped_pages(malformed, count, None).await
}
async fn fixture_mapped_pages(
    malformed: bool,
    count: u32,
    mapping: Option<(rip_core::pages::PageRanges, u32)>,
) -> (
    tempfile::TempDir,
    jrip_server::App,
    Job,
    Fake,
    tokio::task::JoinHandle<()>,
) {
    fixture_with_source_hash(malformed, count, mapping, "source").await
}
async fn fixture_with_source_hash(
    malformed: bool,
    count: u32,
    mapping: Option<(rip_core::pages::PageRanges, u32)>,
    source_hash: &str,
) -> (
    tempfile::TempDir,
    jrip_server::App,
    Job,
    Fake,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    let f = Fake {
        document: Arc::new(std::sync::Mutex::new(Vec::new())),
        copies: Arc::new(AtomicUsize::new(0)),
        copies_max: Arc::new(AtomicUsize::new(5)),
        duplex: Arc::new(AtomicUsize::new(0)),
        prints: Arc::new(AtomicUsize::new(0)),
        state: Arc::new(AtomicUsize::new(3)),
        progress_mode: Arc::new(AtomicUsize::new(0)),
        queue_count: Arc::new(AtomicUsize::new(7)),
        accepting_jobs: Arc::new(AtomicUsize::new(1)),
        printer_state: Arc::new(AtomicUsize::new(3)),
        marker_mode: Arc::new(AtomicUsize::new(0)),
        pwg_supply_mode: Arc::new(AtomicUsize::new(0)),
        capability_queries: Arc::new(AtomicUsize::new(0)),
        printer_reasons: Arc::new(std::sync::Mutex::new(vec![])),
        reasons: Arc::new(std::sync::Mutex::new(vec![])),
        malformed,
        job_name: Arc::new(std::sync::Mutex::new(String::new())),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/printer", post(handle))
        .with_state(f.clone());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    app.printers = vec![jrip_server::delivery::Printer {
        id: "test".into(),
        color_target: None,
        uri: format!("ipp://{addr}/printer"),
        media: vec![jrip_server::delivery::Media {
            name: "custom_square_25.4x25.4mm".into(),
            width_microns: 25400,
            height_microns: 25400,
        }],
    }];
    let mut job = Job {
        id: Uuid::new_v4(),
        manifest: serde_json::from_value::<Manifest>(
            serde_json::json!({"name":"test.pdf","format":"application/pdf","dpi":72,"color_mode":"rgb"}),
        )
        .unwrap(),
        state: JobState::Spooling,
        source_sha256: source_hash.into(),
        created_at: 0,
        updated_at: 0,
        revision: 0,
        error_code: None,
        selected_engine: None,
        worker_run_id: None,
    };
    if let Some((ranges, _)) = &mapping {
        job.manifest.page_ranges = ranges.clone().into();
    }
    app.repo.insert(&job).await.unwrap();
    let path = app
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("raster");
    std::fs::create_dir_all(&path).unwrap();
    let mut artifacts = Vec::new();
    for page in 1..=count {
        let name = format!("page-{page:06}.tiff");
        let file = path.join(&name);
        let sample = match page {
            1 => 128,
            2 => 64,
            _ => 192,
        };
        tiff::encoder::TiffEncoder::new(std::fs::File::create(&file).unwrap())
            .unwrap()
            .write_image::<tiff::encoder::colortype::RGB8>(72, 72, &vec![sample; 72 * 72 * 3])
            .unwrap();
        let bytes = std::fs::read(file).unwrap();
        artifacts.push(Artifact {
            source_page: None,
            source_page_count: None,
            page,
            name,
            media_type: "image/tiff".into(),
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        });
    }
    if let Some((ranges, total)) = &mapping {
        for (artifact, index) in artifacts.iter_mut().zip(ranges.indices(*total).unwrap()) {
            artifact.source_page = Some(index as u32 + 1);
            artifact.source_page_count = Some(*total);
        }
    }
    app.repo
        .complete_with_artifacts(job.clone(), &artifacts)
        .await
        .unwrap();
    let job = app.repo.get(job.id).await.unwrap();
    (dir, app, job, f, task)
}
async fn prepare(app: &jrip_server::App, job: &Job, id: Uuid) -> StatusCode {
    let body = serde_json::json!({"requestId":id,"printerId":"test","media":"custom_square_25.4x25.4mm","expectedRevision":job.revision});
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::post(format!("/api/v1/jobs/{}/deliveries", job.id))
                .header("authorization", "Bearer token")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    status
}
#[tokio::test]
async fn api_worker_replay_and_device_completion() {
    let (_dir, app, job, f, task) = fixture(false).await;
    let id = Uuid::new_v4();
    prepare(&app, &job, id).await;
    prepare(&app, &job, id).await;
    assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(
        app.repo.delivery(id).await.unwrap().state,
        DeliveryState::Accepted
    );
    let mut restarted = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    restarted.printers = app.printers.clone();
    f.state.store(9, Ordering::SeqCst);
    jrip_server::delivery::tick(&app).await.unwrap();
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(
        app.repo.delivery(id).await.unwrap().state,
        DeliveryState::Completed
    );
    jrip_server::delivery::tick(&restarted).await.unwrap();
    assert_eq!(f.prints.load(Ordering::SeqCst), 1);
    assert_eq!(
        app.repo.get(job.id).await.unwrap().state,
        JobState::Completed
    );
    task.abort();
}
#[tokio::test]
async fn delivery_preview_pins_ticket_and_requires_matching_commit() {
    let (_dir, app, job, f, task) = fixture(false).await;
    let id = Uuid::new_v4();
    let mut body = serde_json::json!({"requestId":id,"printerId":"test","media":"custom_square_25.4x25.4mm","expectedRevision":job.revision});
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::post(format!("/api/v1/jobs/{}/delivery-preview", job.id))
                .header("authorization", "Bearer token")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let preview: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(preview["queued"], false);
    assert!(app.repo.delivery(id).await.is_err());
    assert!(!app.root.join("deliveries").exists());
    body["expectedTicketSha256"] = serde_json::json!("a".repeat(64));
    let path = format!("/api/v1/jobs/{}/deliveries", job.id);
    let request = |body: &serde_json::Value| {
        Request::post(&path)
            .header("authorization", "Bearer token")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    assert_eq!(
        jrip_server::router(app.clone())
            .oneshot(request(&body))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    body["expectedTicketSha256"] = preview["contract"]["ticketSha256"].clone();
    for _ in 0..2 {
        assert_eq!(
            jrip_server::router(app.clone())
                .oneshot(request(&body))
                .await
                .unwrap()
                .status(),
            StatusCode::ACCEPTED
        );
    }
    assert_eq!(
        app.repo.delivery_contract(id).await.unwrap(),
        preview["contract"]
    );
    let reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    assert_eq!(
        reopened.repo.delivery_contract(id).await.unwrap(),
        preview["contract"]
    );
    assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    task.abort();
}
#[tokio::test]
async fn tampering_is_rejected_and_uncertain_sends_are_not_retried() {
    for malformed in [false, true] {
        let (_dir, app, job, f, task) = fixture(malformed).await;
        let id = Uuid::new_v4();
        prepare(&app, &job, id).await;
        if !malformed {
            std::fs::write(
                app.root.join("deliveries").join(format!("{id}.pwg")),
                b"tampered",
            )
            .unwrap();
        }
        jrip_server::delivery::tick(&app).await.unwrap();
        jrip_server::delivery::tick(&app).await.unwrap();
        assert_eq!(
            app.repo.delivery(id).await.unwrap().state,
            if malformed {
                DeliveryState::Unknown
            } else {
                DeliveryState::Rejected
            }
        );
        assert_eq!(f.prints.load(Ordering::SeqCst), usize::from(malformed));
        task.abort();
    }
}
#[tokio::test]
async fn permissions_geometry_and_destination_changes_prevent_printing() {
    let (_dir, mut app, job, f, task) = fixture(false).await;
    let request = || {
        Request::post(format!("/api/v1/jobs/{}/deliveries",job.id)).header("authorization","Bearer token").header("content-type","application/json").body(Body::from(serde_json::json!({"requestId":Uuid::new_v4(),"printerId":"test","media":"custom_square_25.4x25.4mm","expectedRevision":job.revision}).to_string())).unwrap()
    };
    app.credentials = vec![jrip_server::Credential {
        validity: None,
        token: "token".into(),
        identity: "reader".into(),
        scopes: vec!["jobs:read".into()],
    }];
    assert_eq!(
        jrip_server::router(app.clone())
            .oneshot(request())
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    app.credentials.clear();
    app.printers[0].media[0].width_microns = 210000;
    assert_eq!(
        jrip_server::router(app.clone())
            .oneshot(request())
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(app.repo.next_delivery().await.unwrap().is_none());
    app.printers[0].media[0].width_microns = 25400;
    let id = Uuid::new_v4();
    prepare(&app, &job, id).await;
    app.printers[0].uri = "ipp://127.0.0.1:1/different".into();
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(
        app.repo.delivery(id).await.unwrap().state,
        DeliveryState::Rejected
    );
    assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

async fn cancel_request(app: &jrip_server::App, id: Uuid, revision: i64) -> StatusCode {
    jrip_server::router(app.clone())
        .oneshot(
            Request::post(format!("/api/v1/deliveries/{id}/cancel"))
                .header("authorization", "Bearer token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"expectedRevision":revision}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}
#[tokio::test]
async fn cancellation_before_send_and_after_acceptance_is_durable() {
    let (_dir, app, job, f, task) = fixture(false).await;
    let queued = Uuid::new_v4();
    prepare(&app, &job, queued).await;
    assert_eq!(cancel_request(&app, queued, 99).await, StatusCode::CONFLICT);
    assert_eq!(cancel_request(&app, queued, 0).await, StatusCode::OK);
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    let accepted = Uuid::new_v4();
    prepare(&app, &job, accepted).await;
    jrip_server::delivery::tick(&app).await.unwrap();
    let d = app.repo.delivery(accepted).await.unwrap();
    assert_eq!(
        cancel_request(&app, accepted, d.revision).await,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        app.repo.delivery(accepted).await.unwrap().state,
        DeliveryState::Accepted
    );
    let mut restarted = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    restarted.printers = app.printers.clone();
    jrip_server::delivery::tick(&restarted).await.unwrap();
    assert_eq!(
        app.repo.delivery(accepted).await.unwrap().state,
        DeliveryState::Cancelled
    );
    jrip_server::delivery::tick(&restarted).await.unwrap();
    assert_eq!(f.prints.load(Ordering::SeqCst), 1);
    assert_eq!(
        app.repo.get(job.id).await.unwrap().state,
        JobState::Completed
    );
    task.abort();
}

#[tokio::test]
async fn unknown_reconciliation_checks_remote_name_and_never_resends() {
    let (_dir, app, job, f, task) = fixture(true).await;
    let id = Uuid::new_v4();
    prepare(&app, &job, id).await;
    jrip_server::delivery::tick(&app).await.unwrap();
    let unknown = app.repo.delivery(id).await.unwrap();
    assert_eq!(unknown.state, DeliveryState::Unknown);
    async fn resolve(app: &jrip_server::App, id: Uuid, revision: i64) -> StatusCode {
        jrip_server::router(app.clone())
            .oneshot(
                Request::post(format!("/api/v1/deliveries/{id}/reconcile"))
                    .header("authorization", "Bearer token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"expectedRevision":revision,"deviceJobId":42})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }
    let name = f.job_name.lock().unwrap().clone();
    *f.job_name.lock().unwrap() = "another-job".into();
    assert_eq!(
        resolve(&app, id, unknown.revision).await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.repo.delivery(id).await.unwrap().state,
        DeliveryState::Unknown
    );
    *f.job_name.lock().unwrap() = name;
    f.state.store(9, Ordering::SeqCst);
    assert_eq!(resolve(&app, id, unknown.revision).await, StatusCode::OK);
    assert_eq!(
        app.repo.delivery(id).await.unwrap().state,
        DeliveryState::Completed
    );
    assert_eq!(
        resolve(&app, id, unknown.revision).await,
        StatusCode::CONFLICT
    );
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(f.prints.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn approval_binds_actor_attempt_profile_and_can_be_revoked() {
    let (_dir, mut app, job, f, task) = fixture(false).await;
    app.require_print_approval = true;
    app.credentials = vec![
        jrip_server::Credential {
            validity: None,
            token: "admin-token-at-least-24-characters".into(),
            identity: "operator".into(),
            scopes: vec!["admin".into()],
        },
        jrip_server::Credential {
            validity: None,
            token: "agent-token-at-least-24-characters".into(),
            identity: "agent".into(),
            scopes: vec!["printers:print".into(), "artifacts:read".into()],
        },
    ];
    let request = Uuid::new_v4();
    async fn submit(
        app: &jrip_server::App,
        job: &Job,
        request: Uuid,
        approval: Option<Uuid>,
    ) -> StatusCode {
        let mut builder = Request::post(format!("/api/v1/jobs/{}/deliveries", job.id))
            .header("authorization", "Bearer agent-token-at-least-24-characters")
            .header("content-type", "application/json");
        if let Some(approval) = approval {
            builder = builder.header("x-jrip-approval-id", approval.to_string());
        }
        jrip_server::router(app.clone()).oneshot(builder.body(Body::from(serde_json::json!({"requestId":request,"printerId":"test","media":"custom_square_25.4x25.4mm","expectedRevision":job.revision}).to_string())).unwrap()).await.unwrap().status()
    }
    assert_eq!(
        submit(&app, &job, request, None).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    let payload = serde_json::json!({"identity":"agent","jobId":job.id,"expectedRevision":job.revision,"requestId":request,"printerId":"test","media":"custom_square_25.4x25.4mm","reason":"Approved test output"});
    async fn approve(app: &jrip_server::App, payload: &serde_json::Value) -> Uuid {
        let response = jrip_server::router(app.clone())
            .oneshot(
                Request::post("/api/v1/approvals")
                    .header("authorization", "Bearer admin-token-at-least-24-characters")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        Uuid::parse_str(body["id"].as_str().unwrap()).unwrap()
    }
    let approval = approve(&app, &payload).await;
    assert_eq!(
        submit(&app, &job, Uuid::new_v4(), Some(approval)).await,
        StatusCode::FORBIDDEN
    );
    let mut changed = app.clone();
    changed.printers[0].media[0].width_microns += 100;
    assert_eq!(
        submit(&changed, &job, request, Some(approval)).await,
        StatusCode::FORBIDDEN
    );
    app.repo.revoke_approval(approval).await.unwrap();
    assert_eq!(
        submit(&app, &job, request, Some(approval)).await,
        StatusCode::FORBIDDEN
    );
    let approval = approve(&app, &payload).await;
    assert_eq!(
        submit(&app, &job, request, Some(approval)).await,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        submit(&app, &job, request, None).await,
        StatusCode::ACCEPTED
    );
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(f.prints.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn live_capabilities_have_revision_expiry_and_no_device_address() {
    let (_dir, app, _job, f, task) = fixture(false).await;
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::get("/api/v1/printers/test")
                .header("authorization", "Bearer token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["model"]["capabilities"]["acceptingJobs"], true);
    assert_eq!(body["model"]["capabilities"]["queuedJobCount"], 7);
    assert!(
        body["capabilityRevision"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(body["expiresAt"].as_i64().unwrap() > body["observedAt"].as_i64().unwrap());
    assert!(!body.to_string().contains(&app.printers[0].uri));
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::get("/api/v1/print/capabilities?printerId=test")
                .header("authorization", "Bearer token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let common: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(common["stages"].as_array().unwrap().len(), 4);
    assert_eq!(common["deliveryIntersection"]["settingsCompatible"], true);
    assert_eq!(
        common["deliveryIntersection"]["authorizationGranted"],
        false
    );
    assert_eq!(
        common["deliveryIntersection"]["dpi"],
        serde_json::json!([[72, 72]])
    );
    assert!(!common.to_string().contains(&app.printers[0].uri));
    let mut revisions = Vec::new();
    for count in [7, 0, usize::MAX, 7] {
        f.queue_count.store(count, Ordering::SeqCst);
        let response = jrip_server::router(app.clone())
            .oneshot(
                Request::get("/api/v1/print/capabilities?printerId=test")
                    .header("authorization", "Bearer token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let snapshot: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let stage = &snapshot["stages"][3];
        let live = &stage["capability"]["extensions"]["org.jrip.printer"];
        assert_eq!(live["acceptingJobs"], true);
        assert_eq!(
            live["queuedJobCount"],
            if count == usize::MAX {
                serde_json::Value::Null
            } else {
                serde_json::json!(count)
            }
        );
        assert_eq!(
            live["queuedJobCountMeaning"],
            "device-reported-nonterminal-jobs"
        );
        assert_eq!(
            stage["expiresAt"].as_i64().unwrap() - stage["observedAt"].as_i64().unwrap(),
            30
        );
        assert_eq!(stage["capability"]["availability"], "available");
        revisions.push((
            snapshot["capabilityRevision"].clone(),
            stage["capabilityRevision"].clone(),
        ));
        assert!(!snapshot.to_string().contains(&app.printers[0].uri));
    }
    assert_ne!(revisions[0], revisions[1]);
    assert_ne!(revisions[1], revisions[2]);
    assert_eq!(revisions[0], revisions[3]);
    f.queue_count.store(0, Ordering::SeqCst);
    f.accepting_jobs.store(0, Ordering::SeqCst);
    let stopped = jrip_server::print::models(&app, Some("test"))
        .await
        .unwrap();
    assert_eq!(
        stopped["stages"][3]["capability"]["availability"],
        "unavailable"
    );
    assert_eq!(
        stopped["stages"][3]["capability"]["extensions"]["org.jrip.printer"]["queuedJobCount"],
        0
    );
    assert_eq!(
        stopped["stages"][3]["capability"]["extensions"]["org.jrip.printer"]["acceptingJobs"],
        false
    );
    f.queue_count.store(usize::MAX - 1, Ordering::SeqCst);
    assert!(
        jrip_server::print::models(&app, Some("test"))
            .await
            .is_err()
    );

    assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn printer_conditions_match_live_raw_and_common_models_without_sending() {
    let (_dir, app, job, f, task) = fixture(false).await;
    for (state, reasons, evidence) in [
        (
            3,
            vec!["toner-low-warning", "vendor-alert-report"],
            "REPORTED",
        ),
        (
            5,
            vec!["media-empty", "media-jam-error", "cover-open"],
            "REPORTED",
        ),
        (5, vec!["toner-empty-warning"], "REPORTED"),
        (3, vec!["none"], "NO_REASONS_REPORTED"),
        (3, vec![], "UNREPORTED"),
        (3, vec!["none", "media-jam-error"], "REPORTED"),
    ] {
        f.printer_state.store(state, Ordering::SeqCst);
        *f.printer_reasons.lock().unwrap() = reasons.iter().map(|s| s.to_string()).collect();
        let response = jrip_server::router(app.clone())
            .oneshot(
                Request::get("/api/v1/printers/test")
                    .header("authorization", "Bearer token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let raw: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let common = jrip_server::print::models(&app, Some("test"))
            .await
            .unwrap();
        let conditions = &raw["model"]["conditions"];
        assert_eq!(
            conditions,
            &common["stages"][3]["capability"]["extensions"]["org.jrip.printer"]["conditions"]
        );
        assert_eq!(conditions["reasonEvidence"], evidence);
        assert_eq!(conditions["supplyLevelsKnown"], false);
        assert_eq!(conditions["automaticResendPermitted"], false);
        for condition in conditions["conditions"].as_array().unwrap() {
            let raw = condition["rawReason"].as_str().unwrap();
            assert!(reasons.contains(&raw));
            assert_eq!(condition["known"], !raw.starts_with("vendor"));
            if raw == "media-empty" {
                assert_eq!(condition["severity"], "error");
                assert_eq!(condition["recovery"]["action"], "CHECK_AND_LOAD_MEDIA");
            }
            if raw == "toner-empty-warning" {
                assert_eq!(condition["severity"], "warning");
                assert_eq!(
                    conditions["inconsistencies"],
                    serde_json::json!(["STOPPED_WITHOUT_REPORTED_ERROR"])
                );
            }
            for lang in ["en", "ja", "zh-CN"] {
                assert!(condition["recovery"]["messages"][lang].as_str().is_some());
            }
        }
        if state == 3 && reasons.contains(&"media-jam-error") {
            assert_eq!(conditions["inconsistencies"].as_array().unwrap().len(), 2);
        }
        assert_eq!(f.prints.load(Ordering::SeqCst), 0);
        assert_eq!(app.repo.get(job.id).await.unwrap().revision, job.revision);
    }
    task.abort();
}

#[tokio::test]
async fn marker_supplies_have_consistent_read_only_raw_and_common_snapshots() {
    let (_dir, mut app, job, f, task) = fixture(false).await;
    app.credentials = vec![jrip_server::Credential {
        token: "reader".into(),
        identity: "reader".into(),
        scopes: vec!["jobs:read".into()],
        validity: None,
    }];
    for mode in 0..=4 {
        f.marker_mode.store(mode, Ordering::SeqCst);
        let mut inventories = Vec::new();
        for path in [
            "/api/v1/printers/test",
            "/api/v1/print/capabilities?printerId=test",
        ] {
            let response = jrip_server::router(app.clone())
                .oneshot(
                    Request::get(path)
                        .header("authorization", "Bearer reader")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            let inventory = if path.contains("/printers/") {
                body["model"]["capabilities"]["markerSupplies"].clone()
            } else {
                assert_eq!(body["stages"][3]["capability"]["availability"], "available");
                body["stages"][3]["capability"]["extensions"]["org.jrip.printer"]["markerSupplies"]
                    .clone()
            };
            assert_eq!(inventory["profile"], "org.jrip.cups-marker-supplies-v1");
            assert_eq!(inventory["advisoryOnly"], true);
            assert!(!body.to_string().contains(&app.printers[0].uri));
            inventories.push(inventory);
        }
        assert_eq!(inventories[0], inventories[1]);
        let inventory = &inventories[0];
        match mode {
            0 => assert_eq!(inventory["status"], "UNREPORTED"),
            1 => {
                assert_eq!(inventory["numericLevelsReported"], 2);
                assert_eq!(inventory["markers"][0]["levelPercent"], 0);
                assert_eq!(inventory["markers"][1]["kind"], "waste-ink");
                assert_eq!(inventory["markers"][1]["levelPercent"], 90);
                assert_eq!(inventory["markers"][1]["highLevelPercent"], 0);
            }
            2 => {
                assert_eq!(inventory["numericLevelsReported"], 0);
                for (i, state) in ["UNAVAILABLE", "UNKNOWN", "NOT_AT_CAPACITY"]
                    .iter()
                    .enumerate()
                {
                    assert_eq!(inventory["markers"][i]["levelState"], *state);
                    assert!(inventory["markers"][i]["levelPercent"].is_null());
                }
            }
            3 => {
                assert_eq!(inventory["status"], "INVALID_RESPONSE");
                assert_eq!(inventory["reasonCode"], "MARKER_ARRAY_LENGTH_MISMATCH");
                assert!(inventory["markers"].as_array().unwrap().is_empty());
            }
            4 => assert_eq!(inventory["levelsAttributeState"], "UNKNOWN"),
            _ => unreachable!(),
        }
        assert_eq!(app.repo.get(job.id).await.unwrap().revision, job.revision);
        assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    }
    task.abort();
}

#[tokio::test]
async fn pwg_supply_snapshots_keep_standard_units_and_separate_cups_evidence_without_print_authority()
 {
    let (_dir, mut app, job, f, task) = fixture(false).await;
    app.credentials = vec![jrip_server::Credential {
        token: "reader".into(),
        identity: "reader".into(),
        scopes: vec!["jobs:read".into()],
        validity: None,
    }];
    f.marker_mode.store(1, Ordering::SeqCst);
    for mode in 0..=7 {
        f.pwg_supply_mode.store(mode, Ordering::SeqCst);
        let mut reports = Vec::new();
        for path in [
            "/api/v1/printers/test",
            "/api/v1/print/capabilities?printerId=test",
        ] {
            let response = jrip_server::router(app.clone())
                .oneshot(
                    Request::get(path)
                        .header("authorization", "Bearer reader")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            let (standard, markers) = if path.contains("/printers/") {
                (
                    body["model"]["capabilities"]["printerSupplies"].clone(),
                    body["model"]["capabilities"]["markerSupplies"].clone(),
                )
            } else {
                assert_eq!(body["stages"][3]["capability"]["availability"], "available");
                let live = &body["stages"][3]["capability"]["extensions"]["org.jrip.printer"];
                (
                    live["printerSupplies"].clone(),
                    live["markerSupplies"].clone(),
                )
            };
            assert_eq!(markers["status"], "REPORTED");
            assert_eq!(markers["markers"].as_array().unwrap().len(), 2);
            assert_eq!(standard["profile"], "org.jrip.pwg-printer-supplies-v1");
            assert_eq!(standard["advisoryOnly"], true);
            assert!(!body.to_string().contains(&app.printers[0].uri));
            reports.push(standard);
        }
        assert_eq!(reports[0], reports[1]);
        let r = &reports[0];
        match mode {
            0 => assert_eq!(r["status"], "UNREPORTED"),
            1 => {
                assert_eq!(r["supplies"][0]["levelPercent"], 0);
                assert_eq!(r["supplies"][0]["description"]["language"], "ja");
                assert_eq!(r["supplies"][0]["description"]["text"], "黒トナー");
            }
            2 => {
                assert_eq!(r["supplies"][0]["level"], 2000);
                assert_eq!(r["supplies"][0]["unit"], "items");
                assert!(r["supplies"][0]["levelPercent"].is_null());
            }
            3 => {
                assert_eq!(r["supplies"][0]["levelState"], "SOME_AVAILABLE");
                assert!(r["supplies"][0]["level"].is_null());
            }
            4 | 5 => {
                assert_eq!(r["status"], "INVALID_RESPONSE");
                assert!(r["supplies"].as_array().unwrap().is_empty());
            }
            6 => assert_eq!(r["attributeState"], "UNKNOWN"),
            7 => {
                assert_eq!(r["supplies"][0]["levelPercent"], 25);
                assert_eq!(r["supplies"][0]["levelMeaning"], "REMAINING_SPACE");
            }
            _ => unreachable!(),
        }
        assert_eq!(app.repo.get(job.id).await.unwrap().revision, job.revision);
        assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    }
    task.abort();
}

#[tokio::test]
async fn durable_device_observations_are_scoped_replayable_and_never_probe_or_send_on_history_reads()
 {
    async fn get(
        app: &jrip_server::App,
        path: &str,
        token: Option<&str>,
    ) -> axum::response::Response {
        let mut req = Request::get(path);
        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        jrip_server::router(app.clone())
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }
    async fn body(response: axum::response::Response) -> serde_json::Value {
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }
    let (_dir, mut app, job, f, task) = fixture(false).await;
    app.credentials = vec![
        jrip_server::Credential {
            token: "reader".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
            validity: None,
        },
        jrip_server::Credential {
            token: "artifact".into(),
            identity: "artifact".into(),
            scopes: vec!["artifacts:read".into()],
            validity: None,
        },
    ];
    let history = "/api/v1/printers/test/observations";
    for (token, status) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some("artifact"), StatusCode::FORBIDDEN),
    ] {
        assert_eq!(get(&app, history, token).await.status(), status);
    }
    let empty = body(get(&app, history, Some("reader")).await).await;
    assert!(empty["latest"].is_null());
    assert_eq!(f.capability_queries.load(Ordering::SeqCst), 0);
    assert_eq!(
        get(&app, "/api/v1/printers/test", Some("reader"))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        get(
            &app,
            "/api/v1/print/capabilities?printerId=test",
            Some("reader")
        )
        .await
        .status(),
        StatusCode::OK
    );
    let response = get(&app, history, Some("reader")).await;
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    let first = body(response).await;
    assert_eq!(first["events"].as_array().unwrap().len(), 1);
    assert_eq!(first["latest"]["revision"], 1);
    let sequence = first["events"][0]["sequence"].as_i64().unwrap();
    let reference = first["events"][0]["reference"].as_str().unwrap();
    assert!(reference.ends_with(&sequence.to_string()));
    f.queue_count.store(0, Ordering::SeqCst);
    f.marker_mode.store(1, Ordering::SeqCst);
    f.pwg_supply_mode.store(7, Ordering::SeqCst);
    *f.printer_reasons.lock().unwrap() = vec!["toner-low-warning".into()];
    assert_eq!(
        get(&app, "/api/v1/printers/test", Some("reader"))
            .await
            .status(),
        StatusCode::OK
    );
    let changed = body(get(&app, history, Some("reader")).await).await;
    assert_eq!(changed["latest"]["revision"], 2);
    assert_eq!(changed["latest"]["snapshot"]["queuedJobCount"], 0);
    assert_eq!(
        changed["latest"]["snapshot"]["markerSupplies"]["markers"][0]["levelPercent"],
        0
    );
    assert_eq!(
        changed["latest"]["snapshot"]["printerSupplies"]["supplies"][0]["levelMeaning"],
        "REMAINING_SPACE"
    );
    let page = body(get(&app, &format!("{history}?limit=1"), Some("reader")).await).await;
    assert_eq!(page["hasMore"], true);
    let page2 = body(
        get(
            &app,
            &format!("{history}?after={}&limit=1", page["nextAfter"]),
            Some("reader"),
        )
        .await,
    )
    .await;
    assert_eq!(page2["events"].as_array().unwrap().len(), 1);
    let queries = f.capability_queries.load(Ordering::SeqCst);
    let event = body(get(&app, reference, Some("reader")).await).await;
    assert_eq!(event["snapshot"], first["latest"]["snapshot"]);
    assert_eq!(
        event["snapshotSha256"],
        rip_digest::digest(&event["snapshot"]).unwrap()
    );
    assert_eq!(f.capability_queries.load(Ordering::SeqCst), queries);
    for token in [None, Some("artifact")] {
        assert!(matches!(
            get(&app, reference, token).await.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    }
    f.queue_count.store(usize::MAX - 1, Ordering::SeqCst);
    for _ in 0..2 {
        assert_eq!(
            get(&app, "/api/v1/printers/test", Some("reader"))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let failed = body(get(&app, history, Some("reader")).await).await;
    assert_eq!(failed["latest"]["queryFailure"], "INVALID_RESPONSE");
    assert_eq!(failed["latest"]["snapshot"], changed["latest"]["snapshot"]);
    assert_eq!(
        failed["latest"]["lastObservedAt"],
        changed["latest"]["lastObservedAt"]
    );
    assert_eq!(failed["events"].as_array().unwrap().len(), 3);
    f.queue_count.store(0, Ordering::SeqCst);
    assert_eq!(
        get(
            &app,
            "/api/v1/print/capabilities?printerId=test",
            Some("reader")
        )
        .await
        .status(),
        StatusCode::OK
    );
    let recovered = body(get(&app, history, Some("reader")).await).await;
    assert!(recovered["latest"]["queryFailure"].is_null());
    assert_eq!(recovered["events"][3]["kind"], "QUERY_RECOVERED");
    let mut reopened = jrip_server::open(app.root.clone(), "reader".into())
        .await
        .unwrap();
    reopened.printers = app.printers.clone();
    reopened.credentials = app.credentials.clone();
    let restored = body(get(&reopened, history, Some("reader")).await).await;
    assert_eq!(restored["streamId"], first["streamId"]);
    assert_eq!(restored["latest"], recovered["latest"]);
    let queries = f.capability_queries.load(Ordering::SeqCst);
    reopened.printers[0].uri = "ipp://different-device.invalid/printer".into();
    assert!(body(get(&reopened, history, Some("reader")).await).await["latest"].is_null());
    assert_eq!(
        get(&reopened, reference, Some("reader")).await.status(),
        StatusCode::NOT_FOUND
    );
    reopened.printers = app.printers.clone();
    reopened.printer_security = vec![jrip_server::delivery::PrinterSecurity {
        printer_id: "test".into(),
        bearer_token: Some("private-device-credential".into()),
        ca_pem: None,
    }];
    assert!(body(get(&reopened, history, Some("reader")).await).await["latest"].is_null());
    assert_eq!(f.capability_queries.load(Ordering::SeqCst), queries);
    assert_eq!(
        get(&reopened, "/api/v1/printers/test", Some("reader"))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let configuration_failure = body(get(&reopened, history, Some("reader")).await).await;
    assert_eq!(
        configuration_failure["latest"]["queryFailure"],
        "CONFIGURATION_UNAVAILABLE"
    );
    assert!(configuration_failure["latest"]["snapshot"].is_null());
    assert_ne!(configuration_failure["streamId"], first["streamId"]);
    assert_eq!(f.capability_queries.load(Ordering::SeqCst), queries);
    task.abort();
    assert_eq!(
        get(&app, "/api/v1/printers/test", Some("reader"))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let unavailable = body(get(&app, history, Some("reader")).await).await;
    assert_eq!(unavailable["latest"]["queryFailure"], "UNAVAILABLE");
    assert_eq!(
        unavailable["latest"]["snapshot"],
        recovered["latest"]["snapshot"]
    );
    for suffix in ["?after=-1", "?limit=0", "?limit=101", "/0"] {
        assert_eq!(
            get(&app, &format!("{history}{suffix}"), Some("reader"))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert!(!restored.to_string().contains(&app.printers[0].uri));
    assert!(!restored.to_string().contains("private-device-credential"));
    assert_eq!(app.repo.get(job.id).await.unwrap().revision, job.revision);
    assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

async fn pipeline_request(
    app: &jrip_server::App,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::post(path)
                .header("authorization", "Bearer token")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (
        status,
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap(),
    )
}
#[tokio::test]
async fn delivery_artifact_contract_reads_never_send_and_survive_cancel_restart_and_config_removal()
{
    let (dir, mut app, job, fake, task) =
        fixture_with_source_hash(false, 3, None, &format!("{:x}", Sha256::digest(b"source"))).await;
    fake.duplex.store(1, Ordering::SeqCst);
    let id = Uuid::new_v4();
    let (status,_)=pipeline_request(&app,&format!("/api/v1/jobs/{}/deliveries",job.id),
        serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm",
            "copies":3,"sides":"two-sided-long-edge","pageRanges":[{"first":2,"last":3}]})).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let original = app.repo.delivery(id).await.unwrap();
    app.credentials = [
        ("token", vec!["admin"]),
        ("job", vec!["jobs:read"]),
        ("artifact", vec!["artifacts:read"]),
    ]
    .into_iter()
    .map(|(token, scopes)| jrip_server::Credential {
        token: token.into(),
        identity: token.into(),
        scopes: scopes.into_iter().map(String::from).collect(),
        validity: None,
    })
    .collect();
    async fn get(
        app: &jrip_server::App,
        id: Uuid,
        descriptor: bool,
        token: Option<&str>,
        range: Option<&str>,
    ) -> axum::response::Response {
        let mut req = Request::get(format!(
            "/api/v1/deliveries/{id}/artifact{}",
            if descriptor { "/descriptor" } else { "" }
        ));
        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        if let Some(range) = range {
            req = req.header("range", range);
        }
        jrip_server::router(app.clone())
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }
    async fn value(response: axum::response::Response) -> serde_json::Value {
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }
    for descriptor in [false, true] {
        for (token, status) in [
            (None, StatusCode::UNAUTHORIZED),
            (Some("job"), StatusCode::FORBIDDEN),
            (Some("artifact"), StatusCode::FORBIDDEN),
        ] {
            assert_eq!(
                get(&app, id, descriptor, token, None).await.status(),
                status
            );
        }
    }
    let response = get(&app, id, true, Some("token"), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    let descriptor = value(response).await;
    assert_eq!(descriptor["kind"], "DeliveryArtifact");
    assert_eq!(descriptor["pageCount"], 2);
    assert_eq!(descriptor["copies"], 3);
    assert_eq!(descriptor["selectedSourcePages"], serde_json::json!([2, 3]));
    assert_eq!(descriptor["pages"][0]["width"], 72);
    assert_eq!(descriptor["pages"][0]["height"], 72);
    assert_eq!(descriptor["pages"][0]["dpi"], serde_json::json!([72, 72]));
    assert_eq!(descriptor["physicalContractVerified"], true);
    assert_eq!(descriptor["printedAppearanceVerified"], false);
    assert_eq!(descriptor["provenance"]["sourceSha256"], job.source_sha256);
    assert_eq!(descriptor["sha256"], original.ticket.artifact_sha256);
    assert!(!descriptor.to_string().contains(&app.printers[0].uri));
    assert!(
        !descriptor
            .to_string()
            .contains(dir.path().to_str().unwrap())
    );
    assert_eq!(
        app.repo.delivery(id).await.unwrap().revision,
        original.revision
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    let response = get(&app, id, false, Some("token"), Some("bytes=0-3")).await;
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()["content-type"], "image/pwg-raster");
    assert_eq!(
        &response.into_body().collect().await.unwrap().to_bytes()[..],
        b"RaS2"
    );
    let mut reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    reopened.credentials = app.credentials.clone();
    assert!(reopened.printers.is_empty());
    assert_eq!(
        value(get(&reopened, id, true, Some("token"), None).await).await["contentId"],
        descriptor["contentId"]
    );
    reopened
        .repo
        .request_delivery_cancel(id, original.revision)
        .await
        .unwrap();
    let cancelled = value(get(&reopened, id, true, Some("token"), None).await).await;
    assert_eq!(cancelled["execution"]["deliveryState"], "CANCELLED");
    assert_eq!(cancelled["contentId"], descriptor["contentId"]);
    let path = app.root.join("deliveries").join(format!("{id}.pwg"));
    let bytes = std::fs::read(&path).unwrap();
    let legacy_id = Uuid::new_v4();
    app.repo
        .enqueue_delivery(legacy_id, original.ticket.clone())
        .await
        .unwrap();
    std::fs::write(
        app.root.join("deliveries").join(format!("{legacy_id}.pwg")),
        &bytes,
    )
    .unwrap();
    let legacy = value(get(&app, legacy_id, true, Some("token"), None).await).await;
    assert_eq!(legacy["physicalContractVerified"], false);
    assert!(legacy["provenance"]["sourceSha256"].is_null());
    let response = get(&app, id, false, Some("token"), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut changed = bytes.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    std::fs::write(&path, changed).unwrap();
    assert_eq!(
        &response.into_body().collect().await.unwrap().to_bytes()[..],
        bytes.as_slice()
    );
    assert_eq!(
        get(&app, id, true, Some("token"), None).await.status(),
        StatusCode::CONFLICT
    );
    let invalid_id = Uuid::new_v4();
    let invalid = b"RaS2bad";
    let mut ticket = original.ticket;
    ticket.artifact_bytes = invalid.len() as u64;
    ticket.artifact_sha256 = format!("{:x}", Sha256::digest(invalid));
    app.repo.enqueue_delivery(invalid_id, ticket).await.unwrap();
    std::fs::write(
        app.root
            .join("deliveries")
            .join(format!("{invalid_id}.pwg")),
        invalid,
    )
    .unwrap();
    assert_eq!(
        get(&app, invalid_id, true, Some("token"), None)
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn failed_device_queries_are_distinct_from_print_failure_and_recover_without_resend() {
    let (_dir, mut app, job, fake, task) = fixture(false).await;
    let id = Uuid::new_v4();
    let (status, _) = pipeline_request(&app,&format!("/api/v1/jobs/{}/pipelines",job.id),
        serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm"})).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    jrip_server::pipeline::tick(&app).await.unwrap();
    jrip_server::delivery::tick(&app).await.unwrap();
    let observed = app.repo.delivery(id).await.unwrap().device_observation;
    for (mode, reason) in [(9, "INVALID_RESPONSE"), (10, "JOB_NOT_FOUND")] {
        fake.progress_mode.store(mode, Ordering::SeqCst);
        jrip_server::delivery::tick(&app).await.unwrap();
        let failed = app.repo.delivery(id).await.unwrap();
        assert_eq!(failed.state, rip_storage::delivery::DeliveryState::Accepted);
        assert_eq!(failed.device_observation, observed);
        let snapshot = app.repo.pipeline_ticket(id).await.unwrap();
        assert_eq!(snapshot["execution"]["deviceQuery"]["failure"], reason);
        assert_eq!(snapshot["execution"]["completed"], false);
        assert!(app.repo.delivery_result_receipt(id).await.is_err());
        jrip_server::delivery::tick(&app).await.unwrap();
        assert_eq!(
            app.repo.delivery(id).await.unwrap().revision,
            failed.revision
        );
    }
    fake.progress_mode.store(0, Ordering::SeqCst);
    jrip_server::delivery::tick(&app).await.unwrap();
    assert!(
        app.repo
            .delivery(id)
            .await
            .unwrap()
            .device_query
            .unwrap()
            .failure
            .is_none()
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    let original_uri = app.printers[0].uri.clone();
    app.printers[0].uri = "ipp://127.0.0.1:1/changed".into();
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(
        serde_json::to_value(app.repo.delivery(id).await.unwrap().device_query).unwrap()["failure"],
        "REGISTRATION_UNAVAILABLE"
    );
    app.printers[0].uri = original_uri;
    jrip_server::delivery::tick(&app).await.unwrap();
    assert!(
        app.repo
            .delivery(id)
            .await
            .unwrap()
            .device_query
            .unwrap()
            .failure
            .is_none()
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    let address = app.printers[0]
        .uri
        .strip_prefix("ipp://")
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    task.abort();
    let _ = task.await;
    let last_sample = app.repo.delivery(id).await.unwrap().device_observation;
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(
        serde_json::to_value(app.repo.delivery(id).await.unwrap().device_query).unwrap()["failure"],
        "UNAVAILABLE"
    );
    assert_eq!(
        app.repo.delivery(id).await.unwrap().device_observation,
        last_sample
    );
    let listener = tokio::net::TcpListener::bind(&address).await.unwrap();
    let router = Router::new()
        .route("/printer", post(handle))
        .with_state(fake.clone());
    let restarted = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    jrip_server::delivery::tick(&app).await.unwrap();
    assert!(
        app.repo
            .delivery(id)
            .await
            .unwrap()
            .device_query
            .unwrap()
            .failure
            .is_none()
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    let timeline = app.repo.unified_pipeline_events(id, 0, 100).await.unwrap();
    assert!(
        timeline["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["details"]["deviceQuery"]["failure"] == "JOB_NOT_FOUND")
    );
    restarted.abort();
}

#[tokio::test]
async fn device_terminal_outcomes_preserve_pipeline_receipt_and_event_meanings() {
    for (state, outcome, receipt_status) in [
        (7, "CANCELLED", "cancelled"),
        (8, "FAILED", "failed"),
        (9, "COMPLETED", "succeeded"),
    ] {
        let (_dir, app, job, fake, task) = fixture(false).await;
        let id = Uuid::new_v4();
        let (status, _) = pipeline_request(
            &app,
            &format!("/api/v1/jobs/{}/pipelines", job.id),
            serde_json::json!({"requestId":id,"expectedRevision":job.revision,
                "printerId":"test","media":"custom_square_25.4x25.4mm"}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        jrip_server::pipeline::tick(&app).await.unwrap();
        jrip_server::delivery::tick(&app).await.unwrap();
        fake.state.store(state, Ordering::SeqCst);
        *fake.reasons.lock().unwrap() = vec!["vendor-terminal-reason".into()];
        jrip_server::delivery::tick(&app).await.unwrap();
        jrip_server::pipeline::tick(&app).await.unwrap();
        let pipeline = app.repo.pipeline(id).await.unwrap();
        assert_eq!(pipeline["state"], outcome);
        let receipt = app.repo.delivery_result_receipt(id).await.unwrap();
        assert_eq!(receipt["status"], receipt_status);
        assert_eq!(
            receipt["proof"]["result"]["deviceObservation"]["state"],
            state
        );
        let timeline = app.repo.unified_pipeline_events(id, 0, 100).await.unwrap();
        assert!(
            timeline["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["origin"]["kind"] == "printer"
                    && e["details"]["deviceObservation"]["state"] == state
                    && e["details"]["deviceObservation"]["reasons"]
                        == serde_json::json!(["vendor-terminal-reason"]))
        );
        assert_eq!(
            pipeline_request(
                &app,
                &format!("/api/v1/pipelines/{id}/cancel"),
                serde_json::json!({"expectedRevision":pipeline["revision"]})
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        for _ in 0..2 {
            jrip_server::delivery::tick(&app).await.unwrap();
            jrip_server::pipeline::tick(&app).await.unwrap();
        }
        assert_eq!(
            app.repo.pipeline(id).await.unwrap()["revision"],
            pipeline["revision"]
        );
        assert_eq!(app.repo.delivery_result_receipt(id).await.unwrap(), receipt);
        assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
        task.abort();
    }
}

#[tokio::test]
async fn pipeline_recovers_and_tracks_device_completion_without_duplicate_delivery() {
    let (_dir, app, job, f, task) = fixture(false).await;
    let id = Uuid::new_v4();
    let body = serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm"});
    for _ in 0..2 {
        assert_eq!(
            pipeline_request(
                &app,
                &format!("/api/v1/jobs/{}/pipelines", job.id),
                body.clone()
            )
            .await
            .0,
            StatusCode::ACCEPTED
        );
    }
    let pending = app.repo.pipeline_ticket(id).await.unwrap();
    assert_eq!(pending["contractFinalized"], false);
    assert_eq!(pending["execution"]["completed"], false);
    assert!(pending["execution"]["deviceObservation"].is_null());
    assert_eq!(pending["rasterEvidenceAvailable"], false);
    assert!(app.repo.delete(job.clone()).await.is_err());
    assert_eq!(
        app.repo.pipeline_events(id, 0, 100).await.unwrap()["events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    jrip_server::pipeline::tick(&app).await.unwrap();
    assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "DELIVERY");
    let prepared = app.repo.pipeline_ticket(id).await.unwrap();
    assert_eq!(prepared["contractFinalized"], true);
    assert_eq!(prepared["execution"]["completed"], false);
    let mut reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    reopened.printers = app.printers.clone();
    jrip_server::pipeline::tick(&reopened).await.unwrap();
    assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    jrip_server::delivery::tick(&reopened).await.unwrap();
    for (state, reasons) in [
        (5, vec!["job-printing"]),
        (
            6,
            vec!["media-needed", "printer-stopped", "vendor-needs-attention"],
        ),
        (5, vec!["job-printing"]),
    ] {
        *f.reasons.lock().unwrap() = reasons.iter().map(|s| s.to_string()).collect();
        f.state.store(state, Ordering::SeqCst);
        jrip_server::delivery::tick(&reopened).await.unwrap();
        let sample = reopened.repo.delivery(id).await.unwrap();
        assert_eq!(sample.state, rip_storage::delivery::DeliveryState::Accepted);
        assert_eq!(
            sample.device_observation.as_ref().unwrap().state,
            state as i32
        );
        assert_eq!(f.prints.load(Ordering::SeqCst), 1);
        let unified = reopened.repo.pipeline_ticket(id).await.unwrap();
        assert_eq!(
            unified["execution"]["deviceObservation"],
            serde_json::to_value(sample.device_observation.as_ref().unwrap()).unwrap()
        );
        assert_eq!(unified["ticketSha256"], prepared["ticketSha256"]);
        assert_eq!(unified["portableDigest"], prepared["portableDigest"]);
        assert_eq!(unified["execution"]["completed"], false);
        jrip_server::delivery::tick(&reopened).await.unwrap();
        assert_eq!(
            reopened.repo.delivery(id).await.unwrap().revision,
            sample.revision
        );
        assert_eq!(
            app.repo.delivery(id).await.unwrap().device_observation,
            sample.device_observation
        );
    }
    *f.reasons.lock().unwrap() = vec!["job-completed-successfully".into()];
    f.state.store(9, Ordering::SeqCst);
    jrip_server::delivery::tick(&reopened).await.unwrap();
    jrip_server::pipeline::tick(&reopened).await.unwrap();
    assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "COMPLETED");
    assert_eq!(f.prints.load(Ordering::SeqCst), 1);
    let complete = reopened.repo.pipeline_ticket(id).await.unwrap();
    assert_eq!(complete["execution"]["completed"], true);
    assert_eq!(complete["execution"]["deviceObservation"]["state"], 9);
    assert_eq!(
        complete["execution"]["deviceObservation"]["progress"],
        serde_json::json!({"impressionsCompleted":8,"mediaSheetsCompleted":4})
    );
    assert_eq!(
        complete["execution"]["deviceObservation"]["reasons"],
        serde_json::json!(["job-completed-successfully"])
    );
    let receipt = app.repo.delivery_result_receipt(id).await.unwrap();
    assert_eq!(receipt["status"], "succeeded");
    assert_eq!(receipt["proof"]["result"]["deviceObservation"]["state"], 9);
    assert_eq!(
        receipt["proof"]["result"]["deviceObservation"]["reasons"],
        serde_json::json!(["job-completed-successfully"])
    );

    assert_eq!(
        receipt["proof"]["result"]["deviceJobId"],
        complete["execution"]["deviceJobId"]
    );
    assert_eq!(
        receipt["proof"]["result"]["completionMeaning"],
        "device-confirmed"
    );
    assert_eq!(
        receipt["proof"]["result"]["printedAppearanceVerified"],
        false
    );

    assert_eq!(complete["ticketSha256"], prepared["ticketSha256"]);
    assert_eq!(complete["portableDigest"]["available"], true);
    assert_eq!(
        complete["portableDigest"]["profile"],
        "org.jrip.jcs-safe-v1"
    );
    assert_eq!(complete["portableDigest"], prepared["portableDigest"]);

    assert_eq!(
        complete["ticket"]["physical"]["ticket"]["jobId"],
        job.id.to_string()
    );
    let ticket_url = format!("/api/v1/pipelines/{id}/ticket");
    let response = jrip_server::router(reopened.clone())
        .oneshot(
            Request::get(&ticket_url)
                .header("authorization", "Bearer token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let read: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(read, complete);
    assert_eq!(
        jrip_server::router(reopened.clone())
            .oneshot(Request::get(&ticket_url).body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let history = reopened.repo.pipeline_events(id, 0, 100).await.unwrap();
    let timeline = reopened
        .repo
        .unified_pipeline_events(id, 0, 100)
        .await
        .unwrap();
    let events = timeline["events"].as_array().unwrap();
    for kind in ["rip", "printer", "pipeline"] {
        assert!(events.iter().any(|event| event["origin"]["kind"] == kind));
    }
    assert!(
        events
            .iter()
            .any(|event| event["origin"]["kind"] == "printer"
                && event["state"] == "COMPLETED"
                && event["revision"].is_number())
    );
    let mut after = 0;
    let mut resumed = vec![];
    loop {
        let page = reopened
            .repo
            .unified_pipeline_events(id, after, 2)
            .await
            .unwrap();
        resumed.extend(page["events"].as_array().unwrap().iter().cloned());
        after = page["nextAfter"].as_i64().unwrap();
        if page["hasMore"] == false {
            break;
        }
    }
    assert_eq!(resumed, *events);
    let restarted_again = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    assert_eq!(
        restarted_again
            .repo
            .unified_pipeline_events(id, 0, 100)
            .await
            .unwrap(),
        timeline
    );
    let states: Vec<_> = history["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, vec!["QUEUED", "DELIVERY", "COMPLETED"]);
    assert_eq!(
        history["events"][2]["completionMeaning"],
        "device-confirmed"
    );
    assert!(
        reopened
            .repo
            .update_pipeline(id, 0, "BLOCKED", None, None)
            .await
            .is_err()
    );
    assert_eq!(
        reopened.repo.pipeline_events(id, 0, 100).await.unwrap(),
        history
    );
    let first = reopened.repo.pipeline_events(id, 0, 1).await.unwrap();
    let after = first["nextAfter"].as_i64().unwrap();
    assert_eq!(
        reopened.repo.pipeline_events(id, after, 100).await.unwrap()["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let url = format!("/api/v1/pipelines/{id}/events?after={after}&limit=100");
    let response = jrip_server::router(reopened.clone())
        .oneshot(
            Request::get(&url)
                .header("authorization", "Bearer token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(page["events"].as_array().unwrap().len(), 2);
    assert_eq!(
        jrip_server::router(reopened.clone())
            .oneshot(Request::get(&url).body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    reopened.repo.delete(job).await.unwrap();
    assert_eq!(reopened.repo.pipeline_ticket(id).await.unwrap(), complete);
    task.abort();
}
#[tokio::test]
async fn unified_ticket_rejects_unrelated_attempt_with_same_id() {
    let (_dir, app, job, _, task) = fixture(false).await;
    let mut other = job.clone();
    other.id = Uuid::new_v4();
    app.repo.insert(&other).await.unwrap();
    let id = Uuid::new_v4();
    assert_eq!(pipeline_request(&app, &format!("/api/v1/jobs/{}/pipelines", job.id),
        serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm"})).await.0, StatusCode::ACCEPTED);
    app.repo
        .enqueue_approved_delivery(
            id,
            rip_storage::delivery::DeliveryTicket {
                page_ranges: Default::default(),
                sides: Default::default(),
                sheet_back: Default::default(),
                copies: 1,
                source_job: other.id,
                color_attempt_id: None,
                printer_id: "test".into(),
                artifact_sha256: "a".repeat(64),
                artifact_bytes: 1,
                dpi: 72,
                document_type: "rgb_8".into(),
                media: "custom_square_25.4x25.4mm".into(),
            },
            None,
        )
        .await
        .unwrap();
    assert!(matches!(
        app.repo.pipeline_ticket(id).await,
        Err(rip_storage::StorageError::Conflict)
    ));
    assert_eq!(
        jrip_server::router(app)
            .oneshot(
                Request::get(format!("/api/v1/pipelines/{id}/ticket"))
                    .header("authorization", "Bearer token")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    task.abort();
}
#[tokio::test]
async fn pipeline_waits_for_approval_and_rechecks_authorization() {
    let (_dir, mut app, job, f, task) = fixture(false).await;
    app.require_print_approval = true;
    let id = Uuid::new_v4();
    let body = serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm"});
    assert_eq!(
        pipeline_request(&app, &format!("/api/v1/jobs/{}/pipelines", job.id), body)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    jrip_server::pipeline::tick(&app).await.unwrap();
    assert_eq!(
        app.repo.pipeline(id).await.unwrap()["state"],
        "WAITING_APPROVAL"
    );
    assert!(app.repo.next_delivery().await.unwrap().is_none());
    assert_eq!(
        pipeline_request(
            &app,
            &format!("/api/v1/pipelines/{id}/resume"),
            serde_json::json!({"expectedRevision":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    app.credentials = vec![jrip_server::Credential {
        validity: None,
        token: "token".into(),
        identity: "legacy-local".into(),
        scopes: vec!["jobs:read".into()],
    }];
    jrip_server::pipeline::tick(&app).await.unwrap();
    assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "BLOCKED");
    app.credentials.clear();
    let revision = app.repo.pipeline(id).await.unwrap()["revision"].clone();
    assert_eq!(
        pipeline_request(
            &app,
            &format!("/api/v1/pipelines/{id}/cancel"),
            serde_json::json!({"expectedRevision":revision})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "CANCELLED");
    let cancelled = app.repo.pipeline(id).await.unwrap();
    assert_eq!(
        pipeline_request(
            &app,
            &format!("/api/v1/pipelines/{id}/cancel"),
            serde_json::json!({"expectedRevision":cancelled["revision"]})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(app.repo.pipeline(id).await.unwrap(), cancelled);
    jrip_server::pipeline::tick(&app).await.unwrap();
    assert!(app.repo.next_delivery().await.unwrap().is_none());
    assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
#[ignore = "requires locally built jrip-color:local Docker image"]
async fn pipeline_color_approval_and_delivery_survive_restart() {
    let (_dir, mut app, job, f, task) = fixture(false).await;
    let record = rip_color::registry::Registry::open(app.root.join("color-profiles"))
        .unwrap()
        .register(&lcms2::Profile::new_srgb().icc().unwrap())
        .unwrap();
    app.printers[0].color_target = Some(jrip_server::delivery::ColorTarget {
        output_profile_sha256: Some(record.sha256.clone()),
        device_link_sha256: None,
    });
    app.require_print_approval = true;
    app.credentials = vec![jrip_server::Credential {
        validity: None,
        token: "token".into(),
        identity: "operator".into(),
        scopes: vec!["admin".into()],
    }];
    let id = Uuid::new_v4();
    let body = serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm",
        "color":{"inputProfileSha256":record.sha256,"outputProfileSha256":record.sha256,"policy":{"schemaVersion":1,"inputColor":"rgb","outputColor":"rgb","intent":"relative_colorimetric","blackPointCompensation":false,"blackPreservation":"none","tacPercent":null}}});
    assert_eq!(
        pipeline_request(&app, &format!("/api/v1/jobs/{}/pipelines", job.id), body)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    jrip_server::pipeline::tick(&app).await.unwrap();
    jrip_server::pipeline::tick(&app).await.unwrap();
    assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "COLOR");
    let mut reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    reopened.printers = app.printers.clone();
    reopened.credentials = app.credentials.clone();
    reopened.require_print_approval = true;
    jrip_server::color::tick(&reopened).await.unwrap();
    assert_eq!(
        reopened.repo.color_attempt(id).await.unwrap()["state"],
        "COMPLETED",
        "{}",
        reopened.repo.color_attempt(id).await.unwrap()
    );
    jrip_server::pipeline::tick(&reopened).await.unwrap();
    let workflow = reopened.repo.pipeline(id).await.unwrap();
    assert_eq!(workflow["state"], "WAITING_APPROVAL");
    assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    let (status,approval)=pipeline_request(&reopened,"/api/v1/approvals",serde_json::json!({"identity":"operator","jobId":job.id,"expectedRevision":job.revision,"requestId":id,"printerId":"test","media":"custom_square_25.4x25.4mm","colorAttemptId":id,"reason":"pipeline acceptance"})).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        pipeline_request(
            &reopened,
            &format!("/api/v1/pipelines/{id}/resume"),
            serde_json::json!({"expectedRevision":workflow["revision"],"approvalId":approval["id"]})
        )
        .await
        .0,
        StatusCode::OK
    );
    jrip_server::pipeline::tick(&reopened).await.unwrap();
    assert_eq!(
        reopened.repo.pipeline(id).await.unwrap()["state"],
        "DELIVERY"
    );
    jrip_server::delivery::tick(&reopened).await.unwrap();
    f.state.store(9, Ordering::SeqCst);
    jrip_server::delivery::tick(&reopened).await.unwrap();
    jrip_server::pipeline::tick(&reopened).await.unwrap();
    assert_eq!(
        reopened.repo.pipeline(id).await.unwrap()["state"],
        "COMPLETED"
    );
    assert_eq!(f.prints.load(Ordering::SeqCst), 1);
    let timeline = reopened
        .repo
        .unified_pipeline_events(id, 0, 100)
        .await
        .unwrap();
    assert!(
        timeline["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["origin"]["kind"] == "color"
                && event["state"] == "COMPLETED"
                && event["progressPercent"] == 100
                && event["completionMeaning"] == "color_managed_export")
    );
    task.abort();
}

#[tokio::test]
async fn pipeline_unknown_is_not_automatically_retried() {
    let (_dir, app, job, f, task) = fixture(true).await;
    let id = Uuid::new_v4();
    assert_eq!(pipeline_request(&app,&format!("/api/v1/jobs/{}/pipelines",job.id),serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm"})).await.0,StatusCode::ACCEPTED);
    jrip_server::pipeline::tick(&app).await.unwrap();
    jrip_server::delivery::tick(&app).await.unwrap();
    for _ in 0..3 {
        jrip_server::pipeline::tick(&app).await.unwrap();
        jrip_server::delivery::tick(&app).await.unwrap();
    }
    assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "UNKNOWN");
    assert_eq!(
        app.repo.pipeline_ticket(id).await.unwrap()["execution"]["completed"],
        false
    );
    assert_eq!(f.prints.load(Ordering::SeqCst), 1);
    assert!(app.repo.delete(job).await.is_err());
    task.abort();
}

#[tokio::test]
async fn pipeline_waits_for_rip_and_stops_on_terminal_source_failure() {
    let (_dir, app, completed, fake, task) = fixture(false).await;
    for terminal in [None, Some(JobState::Failed), Some(JobState::Cancelled)] {
        let mut source = completed.clone();
        source.id = Uuid::new_v4();
        source.revision = 0;
        source.state = JobState::Queued;
        source.worker_run_id = None;
        app.repo.insert(&source).await.unwrap();
        let id = Uuid::new_v4();
        let body = serde_json::json!({"requestId":id,"expectedRevision":0,"printerId":"test","media":"custom_square_25.4x25.4mm"});
        let path = format!("/api/v1/jobs/{}/pipelines", source.id);
        assert_eq!(
            pipeline_request(&app, &path, body.clone()).await.0,
            StatusCode::ACCEPTED
        );
        Box::pin(jrip_server::pipeline::tick(&app)).await.unwrap();
        assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "RIP");
        assert!(app.repo.delivery(id).await.is_err());
        let mut reopened = jrip_server::open(app.root.clone(), "token".into())
            .await
            .unwrap();
        reopened.printers = app.printers.clone();
        Box::pin(jrip_server::pipeline::tick(&reopened))
            .await
            .unwrap();
        assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "RIP");
        if terminal.is_none() {
            source = app
                .repo
                .transition(source, JobState::Held, None)
                .await
                .unwrap();
            Box::pin(jrip_server::pipeline::tick(&reopened))
                .await
                .unwrap();
            assert_eq!(app.repo.get(source.id).await.unwrap().state, JobState::Held);
            assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "RIP");
            assert!(app.repo.delivery(id).await.is_err());
            source = app
                .repo
                .transition(source, JobState::Queued, None)
                .await
                .unwrap();
        }
        if let Some(state) = terminal {
            app.repo.transition(source, state, None).await.unwrap();
            Box::pin(jrip_server::pipeline::tick(&reopened))
                .await
                .unwrap();
            let pipeline = app.repo.pipeline(id).await.unwrap();
            assert_eq!(pipeline["state"], "FAILED");
            assert_eq!(
                pipeline["reason"],
                if state == JobState::Failed {
                    "pipeline_rip_failed"
                } else {
                    "pipeline_rip_cancelled"
                }
            );
            assert!(app.repo.delivery(id).await.is_err());
        } else {
            let source = app
                .repo
                .claim_queued(source, Uuid::new_v4(), i64::MAX)
                .await
                .unwrap();
            Box::pin(jrip_server::pipeline::tick(&reopened))
                .await
                .unwrap();
            assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "RIP");
            let source = app
                .repo
                .transition(source, JobState::Spooling, None)
                .await
                .unwrap();
            let raster = app
                .root
                .join("jobs")
                .join(source.id.to_string())
                .join("raster");
            std::fs::create_dir_all(&raster).unwrap();
            let bytes = std::fs::read(
                app.root
                    .join("jobs")
                    .join(completed.id.to_string())
                    .join("raster/page-000001.tiff"),
            )
            .unwrap();
            std::fs::write(raster.join("page-000001.tiff"), &bytes).unwrap();
            let source = app
                .repo
                .complete_with_artifacts(
                    source,
                    &[Artifact {
                        source_page: None,
                        source_page_count: None,
                        page: 1,
                        name: "page-000001.tiff".into(),
                        media_type: "image/tiff".into(),
                        bytes: bytes.len() as u64,
                        sha256: format!("{:x}", Sha256::digest(&bytes)),
                    }],
                )
                .await
                .unwrap();
            assert_eq!(
                pipeline_request(&reopened, &path, body).await.0,
                StatusCode::ACCEPTED
            );
            Box::pin(jrip_server::pipeline::tick(&reopened))
                .await
                .unwrap();
            assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "DELIVERY");
            assert_eq!(
                app.repo.delivery_contract(id).await.unwrap()["ticket"]["sourceRevision"],
                source.revision
            );
            Box::pin(jrip_server::delivery::tick(&reopened))
                .await
                .unwrap();
            fake.state.store(9, Ordering::SeqCst);
            Box::pin(jrip_server::delivery::tick(&reopened))
                .await
                .unwrap();
            Box::pin(jrip_server::pipeline::tick(&reopened))
                .await
                .unwrap();
            assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "COMPLETED");
        }
    }
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn pipeline_rejects_source_changes_before_rip_completion() {
    let (_dir, app, mut source, fake, task) = fixture(false).await;
    source.id = Uuid::new_v4();
    source.state = JobState::Queued;
    source.revision = 0;
    app.repo.insert(&source).await.unwrap();
    let id = Uuid::new_v4();
    let body = serde_json::json!({"requestId":id,"expectedRevision":0,"printerId":"test","media":"custom_square_25.4x25.4mm"});
    assert_eq!(
        pipeline_request(&app, &format!("/api/v1/jobs/{}/pipelines", source.id), body)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    source.source_sha256 = "changed".into();
    app.repo
        .transition(source, JobState::Held, None)
        .await
        .unwrap();
    Box::pin(jrip_server::pipeline::tick(&app)).await.unwrap();
    let workflow = app.repo.pipeline(id).await.unwrap();
    assert_eq!(workflow["state"], "FAILED");
    assert_eq!(workflow["reason"], "pipeline_source_changed");
    assert!(app.repo.delivery(id).await.is_err());
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

async fn submit_workflow(
    app: &jrip_server::App,
    id: Uuid,
    document: &str,
    media: &str,
) -> (StatusCode, serde_json::Value) {
    let manifest = serde_json::json!({"name":"atomic.pdf","format":"application/pdf","dpi":72,"color_mode":"rgb"});
    let pipeline =
        serde_json::json!({"requestId":id,"expectedRevision":0,"printerId":"test","media":media});
    let body = format!(
        "--atomic\r\nContent-Disposition: form-data; name=\"manifest\"\r\n\r\n{manifest}\r\n--atomic\r\nContent-Disposition: form-data; name=\"pipeline\"\r\n\r\n{pipeline}\r\n--atomic\r\nContent-Disposition: form-data; name=\"document\"\r\n\r\n{document}\r\n--atomic--\r\n"
    );
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::post("/api/v1/print/submissions")
                .header("authorization", "Bearer token")
                .header("content-type", "multipart/form-data; boundary=atomic")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
async fn atomic_submission_replays_and_rolls_back_conflicts() {
    let (_dir, app, _, fake, task) = fixture(false).await;
    let baseline = app.repo.list(100, 0).await.unwrap().len();
    let id = Uuid::new_v4();
    let media = "custom_square_25.4x25.4mm";
    let (a, b) = tokio::join!(
        Box::pin(submit_workflow(&app, id, "%PDF-1.7\nsource", media)),
        Box::pin(submit_workflow(&app, id, "%PDF-1.7\nsource", media))
    );
    assert_eq!(a.0, StatusCode::ACCEPTED, "{a:?}");
    assert_eq!(b.0, StatusCode::ACCEPTED, "{b:?}");
    assert_eq!(a.1, b.1);
    assert_eq!(app.repo.list(100, 0).await.unwrap().len(), baseline + 1);
    let job_id = Uuid::parse_str(a.1["jobId"].as_str().unwrap()).unwrap();
    let unified = app.repo.pipeline_ticket(id).await.unwrap();
    assert_eq!(unified["rasterEvidenceAvailable"], true);
    assert_eq!(unified["ticket"]["raster"]["jobId"], job_id.to_string());
    assert_eq!(
        unified["ticket"]["raster"]["document"]["sha256"],
        unified["ticket"]["registration"]["sourceSha256"]
    );
    assert_eq!(
        app.repo.job_ticket(job_id).await.unwrap().job_id,
        Some(job_id)
    );
    assert_eq!(
        app.repo.pipeline_events(id, 0, 100).await.unwrap()["events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let mut reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    reopened.printers = app.printers.clone();
    assert_eq!(reopened.repo.pipeline_ticket(id).await.unwrap(), unified);
    Box::pin(jrip_server::pipeline::tick(&reopened))
        .await
        .unwrap();
    assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "RIP");
    let replay = submit_workflow(&reopened, id, "%PDF-1.7\nsource", media).await;
    assert_eq!(replay.0, StatusCode::ACCEPTED);
    assert_eq!(replay.1["jobId"], a.1["jobId"]);
    assert_eq!(
        submit_workflow(&app, id, "%PDF-1.7\nchanged", media)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        submit_workflow(&app, Uuid::new_v4(), "invalid", media)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        submit_workflow(&app, Uuid::new_v4(), "%PDF-1.7\nsource", "bad-media")
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(app.repo.list(100, 0).await.unwrap().len(), baseline + 1);
    assert_eq!(
        std::fs::read_dir(app.root.join("jobs")).unwrap().count(),
        baseline + 1
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

fn physical_intent(document: &str) -> serde_json::Value {
    serde_json::json!({"document":{"uri":"https://example.org/proof.pdf","mediaType":"application/pdf","sha256":format!("{:x}",Sha256::digest(document))},"copies":{"mode":"required","value":1},"output":{"destination":"printer","printerId":"test"},"media":{"mode":"required","value":"custom_square_25.4x25.4mm"}})
}
async fn commit_physical(
    app: &jrip_server::App,
    preflight: &serde_json::Value,
    document: &str,
    raster_endpoint: bool,
) -> (StatusCode, serde_json::Value) {
    let path = format!(
        "/api/v1/print/{}/{}/commit",
        if raster_endpoint {
            "preflights"
        } else {
            "pipeline-preflights"
        },
        preflight["preflightId"].as_str().unwrap()
    );
    let body = format!(
        "--commit\r\nContent-Disposition: form-data; name=\"document\"\r\n\r\n{document}\r\n--commit--\r\n"
    );
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::post(path)
                .header("authorization", "Bearer token")
                .header(
                    "x-jrip-preflight-digest",
                    preflight["preflightDigest"].as_str().unwrap(),
                )
                .header("content-type", "multipart/form-data; boundary=commit")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
async fn physical_intent_selects_live_settings_and_commits_bound_workflow() {
    let (_dir, app, _, fake, task) = fixture(false).await;
    let document = "%PDF-1.7\nsource";
    let id = Uuid::new_v4();
    let mut intent = physical_intent(document);
    intent["dpi"] = serde_json::json!({"mode":"required","value":300});
    let rejected = Box::pin(pipeline_request(
        &app,
        "/api/v1/print/pipeline-preflights",
        serde_json::json!({"requestId":id,"intent":intent}),
    ))
    .await;
    assert_eq!(rejected.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(rejected.1["executable"], false);
    intent["dpi"] = serde_json::json!({"mode":"preferred","value":300});
    let accepted = Box::pin(pipeline_request(
        &app,
        "/api/v1/print/pipeline-preflights",
        serde_json::json!({"requestId":id,"intent":intent}),
    ))
    .await;
    assert_eq!(accepted.0, StatusCode::CREATED, "{accepted:?}");
    let preflight = accepted.1;
    assert_eq!(preflight["ticket"]["manifest"]["dpi"], 72);
    assert_eq!(preflight["ticket"]["manifest"]["color_mode"], "rgb");
    assert_eq!(preflight["adjustments"][0]["field"], "dpi");
    assert_eq!(preflight["authorizationGranted"], false);
    assert_eq!(
        commit_physical(&app, &preflight, document, true).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        commit_physical(&app, &preflight, "%PDF-1.7\nchanged", false)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert!(app.repo.pipeline(id).await.is_err());
    let (a, b) = tokio::join!(
        Box::pin(commit_physical(&app, &preflight, document, false)),
        Box::pin(commit_physical(&app, &preflight, document, false))
    );
    assert!(a.0.is_success(), "{a:?}");
    assert!(b.0.is_success(), "{b:?}");
    assert_eq!(a.1["jobId"], b.1["jobId"]);
    assert_eq!(app.repo.list(100, 0).await.unwrap().len(), 2);
    let job_id = Uuid::parse_str(a.1["jobId"].as_str().unwrap()).unwrap();
    let receipt = app.repo.get_print_receipt(job_id).await.unwrap();
    assert_eq!(receipt["proof"]["workflowId"], id.to_string());
    assert_eq!(
        receipt["proof"]["workflowCompletionMeaning"],
        "device-confirmed"
    );
    assert_eq!(receipt["status"], "accepted");
    Box::pin(jrip_server::pipeline::tick(&app)).await.unwrap();
    assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "RIP");
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}
#[tokio::test]
async fn physical_preflight_rechecks_actor_digest_and_configuration() {
    let (_dir, mut app, _, fake, task) = fixture(false).await;
    app.credentials = vec![jrip_server::Credential {
        validity: None,
        token: "token".into(),
        identity: "owner".into(),
        scopes: vec!["admin".into()],
    }];
    let document = "%PDF-1.7\nsource";
    let accepted = Box::pin(pipeline_request(
        &app,
        "/api/v1/print/pipeline-preflights",
        serde_json::json!({"requestId":Uuid::new_v4(),"intent":physical_intent(document)}),
    ))
    .await;
    assert_eq!(accepted.0, StatusCode::CREATED, "{accepted:?}");
    let mut invalid = accepted.1.clone();
    invalid["preflightDigest"] = serde_json::json!("changed");
    assert_eq!(
        commit_physical(&app, &invalid, document, false).await.0,
        StatusCode::CONFLICT
    );
    app.credentials[0].identity = "another".into();
    assert_eq!(
        commit_physical(&app, &accepted.1, document, false).await.0,
        StatusCode::FORBIDDEN
    );
    app.credentials[0].identity = "owner".into();
    app.printers[0].media[0].width_microns += 1;
    assert_eq!(
        commit_physical(&app, &accepted.1, document, false).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(app.repo.list(100, 0).await.unwrap().len(), 1);
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
#[ignore = "requires Docker and locally built jrip-mupdf:local image"]
async fn physical_intent_real_rip_reaches_device_confirmed_completion() {
    let (_dir, app, _, fake, task) = fixture(false).await;
    let stream = "1 0 0 rg 0 0 72 72 re f\n";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources << >> /Contents 4 0 R >>"
            .into(),
        format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
    ];
    let mut document = "%PDF-1.4\n".to_string();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(document.len());
        document.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
    }
    let xref = document.len();
    document.push_str("xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        document.push_str(&format!("{offset:010} 00000 n \n"));
    }
    document.push_str(&format!(
        "trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    let input = app.root.join("inspection-fixture.pdf");
    std::fs::write(&input, &document).unwrap();
    let inspection = tokio::process::Command::new(env!("CARGO_BIN_EXE_jrip-inspect-document"))
        .arg(&input)
        .env("JRIP_DATA_ROOT", &app.root)
        .env("JRIP_API_TOKEN", "token")
        .output()
        .await
        .unwrap();
    assert!(
        inspection.status.success(),
        "{}",
        String::from_utf8_lossy(&inspection.stderr)
    );
    let inspection: serde_json::Value = serde_json::from_slice(&inspection.stdout).unwrap();
    assert_eq!(inspection["pages"][0]["widthPoints"], 72.0);
    assert_eq!(inspection["contentParsed"], true);
    let workflow = Uuid::new_v4();
    let preflight = Box::pin(pipeline_request(
        &app,
        "/api/v1/print/pipeline-preflights",
        serde_json::json!({"requestId":workflow,"intent":physical_intent(&document),"inspectionId":inspection["inspectionId"]}),
    ))
    .await;
    assert_eq!(preflight.0, StatusCode::CREATED, "{preflight:?}");
    let accepted = commit_physical(&app, &preflight.1, &document, false).await;
    assert_eq!(accepted.0, StatusCode::CREATED, "{accepted:?}");
    Box::pin(jrip_server::pipeline::tick(&app)).await.unwrap();
    assert_eq!(app.repo.pipeline(workflow).await.unwrap()["state"], "RIP");
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_jrip-worker"))
        .arg(accepted.1["jobId"].as_str().unwrap())
        .env("JRIP_DATA_ROOT", &app.root)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Box::pin(jrip_server::pipeline::tick(&app)).await.unwrap();
    assert_eq!(
        app.repo.pipeline(workflow).await.unwrap()["state"],
        "DELIVERY"
    );
    Box::pin(jrip_server::delivery::tick(&app)).await.unwrap();
    fake.state.store(9, Ordering::SeqCst);
    Box::pin(jrip_server::delivery::tick(&app)).await.unwrap();
    Box::pin(jrip_server::pipeline::tick(&app)).await.unwrap();
    assert_eq!(
        app.repo.pipeline(workflow).await.unwrap()["state"],
        "COMPLETED"
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    let receipt = app.repo.delivery_result_receipt(workflow).await.unwrap();
    assert_eq!(receipt["status"], "succeeded");
    assert!(receipt["proof"]["result"]["request"]["physicalContract"].is_object());
    let raster = app
        .repo
        .get_result_receipt(Uuid::parse_str(accepted.1["jobId"].as_str().unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        receipt["proof"]["result"]["rasterResultReceiptSha256"],
        format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&raster).unwrap())
        )
    );

    task.abort();
}

#[tokio::test]
async fn physical_preflight_expiry_and_icc_integrity_block_acceptance() {
    let (_dir, mut app, _, fake, task) = fixture(false).await;
    let registry = rip_color::registry::Registry::open(app.root.join("color-profiles")).unwrap();
    let record = registry
        .register(&lcms2::Profile::new_srgb().icc().unwrap())
        .unwrap();
    app.printers[0].color_target = Some(jrip_server::delivery::ColorTarget {
        output_profile_sha256: Some(record.sha256.clone()),
        device_link_sha256: None,
    });
    let document = "%PDF-1.7\nsource";
    let request = serde_json::json!({"requestId":Uuid::new_v4(),"intent":physical_intent(document),"color":{"inputProfileSha256":record.sha256,"outputProfileSha256":record.sha256,"policy":{"schemaVersion":1,"inputColor":"rgb","outputColor":"rgb","intent":"relative_colorimetric","blackPointCompensation":false,"blackPreservation":"none","tacPercent":null}}});
    let preflight = Box::pin(pipeline_request(
        &app,
        "/api/v1/print/pipeline-preflights",
        request,
    ))
    .await;
    assert_eq!(preflight.0, StatusCode::CREATED, "{preflight:?}");
    let mut expired = preflight.1.clone();
    let id = Uuid::new_v4();
    expired["preflightId"] = serde_json::json!(id);
    expired["expiresAt"] = serde_json::json!(rip_storage::now() - 1);
    expired.as_object_mut().unwrap().remove("preflightDigest");
    expired["preflightDigest"] = serde_json::json!(rip_digest::digest(&expired).unwrap());
    app.repo
        .save_preflight(id, &expired, rip_storage::now() - 1)
        .await
        .unwrap();
    assert_eq!(
        commit_physical(&app, &expired, document, false).await.0,
        StatusCode::GONE
    );
    std::fs::write(
        app.root
            .join("color-profiles")
            .join(format!("{}.icc", record.sha256)),
        b"corrupt",
    )
    .unwrap();
    assert_eq!(
        commit_physical(&app, &preflight.1, document, false).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(app.repo.list(100, 0).await.unwrap().len(), 1);
    assert_eq!(std::fs::read_dir(app.root.join("jobs")).unwrap().count(), 1);
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn source_inspection_is_owner_hash_and_media_bound() {
    let (_dir, mut app, _, fake, task) = fixture(false).await;
    app.credentials = vec![jrip_server::Credential {
        validity: None,
        token: "token".into(),
        identity: "owner".into(),
        scopes: vec!["admin".into()],
    }];
    let document = "%PDF-1.7\nsource";
    let id = Uuid::new_v4();
    let report = serde_json::json!({"schemaVersion":1,"inspectionId":id,"inspectorRevision":"jrip-mupdf-inspection-v1","sourceSha256":format!("{:x}",Sha256::digest(document)),"sourceBytes":document.len(),"pages":[{"page":1,"widthPoints":72.0,"heightPoints":72.0,"rotation":0}],"contentParsed":true,"repaired":false,"encrypted":false});
    app.repo
        .save_document_inspection(id, "owner", &report)
        .await
        .unwrap();
    let body = serde_json::json!({"requestId":Uuid::new_v4(),"intent":physical_intent(document),"inspectionId":id});
    let result = Box::pin(pipeline_request(
        &app,
        "/api/v1/print/pipeline-preflights",
        body.clone(),
    ))
    .await;
    assert_eq!(result.0, StatusCode::CREATED, "{result:?}");
    assert_eq!(result.1["sourceInspection"], report);
    let mut changed = body.clone();
    changed["intent"]["document"]["sha256"] = serde_json::json!("a".repeat(64));
    assert_eq!(
        Box::pin(pipeline_request(
            &app,
            "/api/v1/print/pipeline-preflights",
            changed
        ))
        .await
        .0,
        StatusCode::CONFLICT
    );
    app.credentials[0].identity = "another".into();
    assert_eq!(
        Box::pin(pipeline_request(
            &app,
            "/api/v1/print/pipeline-preflights",
            body.clone()
        ))
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    app.credentials[0].identity = "owner".into();
    let wrong_id = Uuid::new_v4();
    let mut wrong = report.clone();
    wrong["inspectionId"] = serde_json::json!(wrong_id);
    wrong["pages"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"page":2,"widthPoints":144,"heightPoints":72}));
    app.repo
        .save_document_inspection(wrong_id, "owner", &wrong)
        .await
        .unwrap();
    let mut body = body;
    body["inspectionId"] = serde_json::json!(wrong_id);
    assert_eq!(
        Box::pin(pipeline_request(
            &app,
            "/api/v1/print/pipeline-preflights",
            body
        ))
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn legacy_physical_preflight_keeps_its_digest_and_workflow_effects() {
    let (_dir, app, _, fake, task) = fixture(false).await;
    let document = "%PDF-1.7\nsource";
    let req = serde_json::json!({"requestId":Uuid::new_v4(),"intent":physical_intent(document)});
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::post("/api/v1/print/pipeline-preflights")
                .header("authorization", "Bearer token")
                .header("x-jrip-digest-profile", "org.jrip.serde-json-v1")
                .header("content-type", "application/json")
                .body(Body::from(req.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let pf: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(pf["profile"], "org.jrip.pipeline-preflight-v1");
    let committed = commit_physical(&app, &pf, document, false).await;
    assert_eq!(committed.0, StatusCode::CREATED);
    let receipt = app
        .repo
        .get_print_receipt(Uuid::parse_str(committed.1["jobId"].as_str().unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        receipt["proof"]["preflightDigestProfile"],
        "org.jrip.serde-json-v1"
    );
    assert_eq!(
        receipt["proof"]["workflowCompletionMeaning"],
        "device-confirmed"
    );
    assert!(
        receipt["effects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "workflow-queued")
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

async fn copies_request(
    app: &jrip_server::App,
    job: &Job,
    id: Uuid,
    copies: u32,
    preview: bool,
    approval: Option<Uuid>,
) -> (StatusCode, serde_json::Value) {
    let path = format!(
        "/api/v1/jobs/{}/{}",
        job.id,
        if preview {
            "delivery-preview"
        } else {
            "deliveries"
        }
    );
    let mut request = Request::post(path)
        .header("authorization", "Bearer token")
        .header("content-type", "application/json");
    if let Some(approval) = approval {
        request = request.header("x-jrip-approval-id", approval.to_string());
    }
    let response=jrip_server::router(app.clone()).oneshot(request.body(Body::from(serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm","copies":copies}).to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}
#[tokio::test]
async fn device_copies_are_pinned_previewed_persisted_and_sent_once() {
    let (_dir, app, job, fake, task) = fixture(false).await;
    let id = Uuid::new_v4();
    let (status, preview) = copies_request(&app, &job, id, 3, true, None).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    let (status, queued) = copies_request(&app, &job, id, 3, false, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    assert_eq!(queued["ticket"]["copies"], 3);
    let contract = app.repo.delivery_contract(id).await.unwrap();
    assert_eq!(contract["ticket"]["copies"], 3);
    assert_eq!(contract["ticket"]["delivery"]["copies"], 3);
    assert_eq!(
        copies_request(&app, &job, id, 2, false, None).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        copies_request(&app, &job, id, 3, false, None).await.0,
        StatusCode::ACCEPTED
    );
    let reopened = rip_storage::Repository::open(&app.root.join("jobs.sqlite3"))
        .await
        .unwrap();
    assert_eq!(reopened.delivery(id).await.unwrap().ticket.copies, 3);
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(fake.copies.load(Ordering::SeqCst), 3);
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    fake.state.store(9, Ordering::SeqCst);
    jrip_server::delivery::tick(&app).await.unwrap();
    let receipt = app.repo.delivery_result_receipt(id).await.unwrap();
    assert_eq!(receipt["proof"]["result"]["request"]["ticket"]["copies"], 3);
    assert_eq!(
        app.repo.delivery(id).await.unwrap().state,
        DeliveryState::Completed
    );
    task.abort();
}
#[tokio::test]
async fn unsupported_and_changed_copy_capabilities_never_send() {
    let (_dir, app, job, fake, task) = fixture(false).await;
    for count in [0, 6, 1000] {
        let id = Uuid::new_v4();
        assert_eq!(
            copies_request(&app, &job, id, count, false, None).await.0,
            StatusCode::BAD_REQUEST
        );
        assert!(app.repo.delivery(id).await.is_err());
    }
    let id = Uuid::new_v4();
    assert_eq!(
        copies_request(&app, &job, id, 3, false, None).await.0,
        StatusCode::ACCEPTED
    );
    fake.copies_max.store(1, Ordering::SeqCst);
    jrip_server::delivery::tick(&app).await.unwrap();
    let result = app.repo.delivery(id).await.unwrap();
    assert_eq!(result.state, DeliveryState::Rejected);
    assert_eq!(result.reason.as_deref(), Some("unsupported_copies"));
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}
#[tokio::test]
async fn physical_approval_is_bound_to_the_exact_copy_count() {
    let (_dir, mut app, job, fake, task) = fixture(false).await;
    app.require_print_approval = true;
    app.credentials = vec![jrip_server::Credential {
        token: "token".into(),
        identity: "operator".into(),
        scopes: vec!["admin".into()],
        validity: None,
    }];
    let id = Uuid::new_v4();
    let response=jrip_server::router(app.clone()).oneshot(Request::post("/api/v1/approvals").header("authorization","Bearer token").header("content-type","application/json").body(Body::from(serde_json::json!({"identity":"operator","jobId":job.id,"expectedRevision":job.revision,"requestId":id,"printerId":"test","media":"custom_square_25.4x25.4mm","copies":3,"reason":"three copies approved"}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let approval = Uuid::parse_str(value["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        copies_request(&app, &job, id, 2, false, Some(approval))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        copies_request(&app, &job, id, 3, false, Some(approval))
            .await
            .0,
        StatusCode::ACCEPTED
    );
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(fake.copies.load(Ordering::SeqCst), 3);
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn pipeline_copies_survive_restart_and_conflicting_replay_is_rejected() {
    let (_dir, app, job, fake, task) = fixture(false).await;
    let id = Uuid::new_v4();
    let body = serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm","copies":4});
    let path = format!("/api/v1/jobs/{}/pipelines", job.id);
    assert_eq!(
        pipeline_request(&app, &path, body.clone()).await.0,
        StatusCode::ACCEPTED
    );
    let mut changed = body;
    changed["copies"] = serde_json::json!(2);
    assert_eq!(
        pipeline_request(&app, &path, changed).await.0,
        StatusCode::CONFLICT
    );
    let mut reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    reopened.printers = app.printers.clone();
    jrip_server::pipeline::tick(&reopened).await.unwrap();
    assert_eq!(app.repo.delivery(id).await.unwrap().ticket.copies, 4);
    let ticket = app.repo.pipeline_ticket(id).await.unwrap();
    assert_eq!(ticket["ticket"]["physical"]["ticket"]["copies"], 4);
    jrip_server::delivery::tick(&reopened).await.unwrap();
    assert_eq!(fake.copies.load(Ordering::SeqCst), 4);
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    task.abort();
}

async fn ranges_request(
    app: &jrip_server::App,
    job: &Job,
    id: Uuid,
    ranges: serde_json::Value,
    preview: bool,
    approval: Option<Uuid>,
) -> (StatusCode, serde_json::Value) {
    let path = format!(
        "/api/v1/jobs/{}/{}",
        job.id,
        if preview {
            "delivery-preview"
        } else {
            "deliveries"
        }
    );
    let mut request = Request::post(path)
        .header("authorization", "Bearer token")
        .header("content-type", "application/json");
    if let Some(approval) = approval {
        request = request.header("x-jrip-approval-id", approval.to_string());
    }
    let response=jrip_server::router(app.clone()).oneshot(request.body(Body::from(serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm","pageRanges":ranges}).to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}
#[tokio::test]
async fn selected_pages_are_previewed_sent_in_order_and_replayed_canonically() {
    let (_dir, app, job, fake, task) = fixture_pages(false, 3).await;
    let id = Uuid::new_v4();
    let ranges = serde_json::json!([{"first":2,"last":3}]);
    let (status, preview) = ranges_request(&app, &job, id, ranges.clone(), true, None).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let physical = &preview["contract"]["ticket"];
    assert_eq!(physical["sourcePageCount"], 3);
    assert_eq!(physical["selectedPageCount"], 2);
    assert_eq!(physical["selectedSourcePages"], serde_json::json!([2, 3]));
    assert_eq!(physical["pageSelection"], "2-3");
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    let (status, queued) = ranges_request(&app, &job, id, ranges, false, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    assert_eq!(
        queued["ticket"]["pageRanges"],
        serde_json::json!([{"first":2,"last":3}])
    );
    assert_eq!(
        ranges_request(
            &app,
            &job,
            id,
            serde_json::json!([{"first":2,"last":2},{"first":3,"last":3}]),
            false,
            None
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        ranges_request(
            &app,
            &job,
            id,
            serde_json::json!([{"first":1,"last":1}]),
            false,
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    reopened.printers = app.printers.clone();
    jrip_server::delivery::tick(&reopened).await.unwrap();
    let source = app
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("raster");
    let mut expected = Vec::new();
    rip_raster::convert(
        &[
            source.join("page-000002.tiff"),
            source.join("page-000003.tiff"),
        ],
        &rip_raster::Options {
            dpi: 72,
            color: rip_raster::Color::DeviceRgb,
        },
        &mut expected,
    )
    .unwrap();
    for page in rip_raster::inspect(&expected).unwrap() {
        let name = b"custom_square_25.4x25.4mm";
        expected[page.offset + 1732..page.offset + 1732 + name.len()].copy_from_slice(name);
    }
    assert_eq!(*fake.document.lock().unwrap(), expected);
    assert_eq!(rip_raster::inspect(&expected).unwrap().len(), 2);
    assert_eq!(
        app.repo
            .artifacts(job.id)
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        3
    );
    jrip_server::delivery::tick(&reopened).await.unwrap();
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    fake.state.store(9, Ordering::SeqCst);
    jrip_server::delivery::tick(&reopened).await.unwrap();
    let receipt = app.repo.delivery_result_receipt(id).await.unwrap();
    assert_eq!(
        receipt["proof"]["result"]["request"]["ticket"]["pageRanges"],
        serde_json::json!([{"first":2,"last":3}])
    );
    task.abort();
}
#[tokio::test]
async fn page_ranges_reject_invalid_bounds_and_verify_only_selected_files() {
    let (_dir, app, job, fake, task) = fixture_pages(false, 3).await;
    for ranges in [
        serde_json::json!([{"first":0,"last":1}]),
        serde_json::json!([{"first":1,"last":4}]),
        serde_json::json!([{"first":2,"last":3},{"first":1,"last":1}]),
        serde_json::json!([{"first":1,"last":2},{"first":2,"last":3}]),
    ] {
        let id = Uuid::new_v4();
        assert!(
            ranges_request(&app, &job, id, ranges, false, None)
                .await
                .0
                .is_client_error()
        );
        assert!(app.repo.delivery(id).await.is_err());
    }
    let source = app
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("raster");
    std::fs::write(source.join("page-000002.tiff"), b"changed unselected page").unwrap();
    let id = Uuid::new_v4();
    assert_eq!(
        ranges_request(
            &app,
            &job,
            id,
            serde_json::json!([{"first":1,"last":1},{"first":3,"last":3}]),
            false,
            None
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(
        rip_raster::inspect(&fake.document.lock().unwrap())
            .unwrap()
            .len(),
        2
    );
    let id = Uuid::new_v4();
    assert_ne!(
        ranges_request(
            &app,
            &job,
            id,
            serde_json::json!([{"first":2,"last":2}]),
            false,
            None
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    assert!(app.repo.delivery(id).await.is_err());
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    task.abort();
}
#[tokio::test]
async fn pipeline_selection_survives_restart_and_exact_approval_is_required() {
    let (_dir, mut app, job, fake, task) = fixture_pages(false, 3).await;
    app.require_print_approval = true;
    app.credentials = vec![jrip_server::Credential {
        token: "token".into(),
        identity: "operator".into(),
        scopes: vec!["admin".into()],
        validity: None,
    }];
    let id = Uuid::new_v4();
    let ranges = serde_json::json!([{"first":2,"last":2}]);
    let body = serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm","pageRanges":ranges});
    assert_eq!(
        pipeline_request(&app, &format!("/api/v1/jobs/{}/pipelines", job.id), body)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    let response=jrip_server::router(app.clone()).oneshot(Request::post("/api/v1/approvals").header("authorization","Bearer token").header("content-type","application/json").body(Body::from(serde_json::json!({"identity":"operator","jobId":job.id,"expectedRevision":job.revision,"requestId":id,"printerId":"test","media":"custom_square_25.4x25.4mm","pageRanges":ranges,"reason":"only second page approved"}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let approval = Uuid::parse_str(value["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        ranges_request(
            &app,
            &job,
            id,
            serde_json::json!([{"first":1,"last":1}]),
            false,
            Some(approval)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ranges_request(&app, &job, id, serde_json::json!([]), false, Some(approval))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let mut reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    reopened.printers = app.printers.clone();
    reopened.credentials = app.credentials.clone();
    reopened.require_print_approval = true;
    // Resume binds this approval; the worker uses persisted page ranges.
    jrip_server::pipeline::tick(&reopened).await.unwrap();
    let pipeline = reopened.repo.pipeline(id).await.unwrap();
    assert_eq!(pipeline["state"], "WAITING_APPROVAL");
    assert_eq!(
        pipeline_request(
            &reopened,
            &format!("/api/v1/pipelines/{id}/resume"),
            serde_json::json!({"expectedRevision":pipeline["revision"],"approvalId":approval})
        )
        .await
        .0,
        StatusCode::OK
    );
    jrip_server::pipeline::tick(&reopened).await.unwrap();
    let ticket = reopened.repo.pipeline_ticket(id).await.unwrap();
    assert_eq!(
        ticket["ticket"]["physical"]["ticket"]["selectedSourcePages"],
        serde_json::json!([2])
    );
    jrip_server::delivery::tick(&reopened).await.unwrap();
    assert_eq!(
        rip_raster::inspect(&fake.document.lock().unwrap())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn physical_intent_copies_are_negotiated_committed_and_delivered_after_restart() {
    let (_dir, app, original, fake, task) = fixture(false).await;
    let document = "%PDF-1.7\nsource";
    let id = Uuid::new_v4();
    let mut intent = physical_intent(document);
    intent["priority"] = serde_json::json!({"mode":"required","value":83});
    intent["copies"] = serde_json::json!({"mode":"required","value":3});
    let (status, preflight) = pipeline_request(
        &app,
        "/api/v1/print/pipeline-preflights",
        serde_json::json!({"requestId":id,"intent":intent}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{preflight}");
    let capabilities = jrip_server::print::models(&app, Some("test"))
        .await
        .unwrap();
    assert_eq!(
        capabilities["deliveryIntersection"]["copies"],
        serde_json::json!([1, 1])
    );
    assert_eq!(
        capabilities["deliveryIntersection"]["deliveryCopies"],
        serde_json::json!([1, 5])
    );
    assert_eq!(preflight["ticket"]["manifest"]["priority"], 83);
    assert_eq!(preflight["workflowRequest"]["copies"], 3);
    assert_eq!(preflight["registrationTicket"]["request"]["copies"], 3);
    assert_eq!(preflight["ticket"]["copies"], 1);
    assert!(preflight["adjustments"].as_array().unwrap().is_empty());
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    let (status, committed) = commit_physical(&app, &preflight, document, false).await;
    assert!(status.is_success(), "{committed}");
    assert_eq!(
        commit_physical(&app, &preflight, document, false).await.1["jobId"],
        committed["jobId"]
    );
    let job_id = Uuid::parse_str(committed["jobId"].as_str().unwrap()).unwrap();
    // Supply a real hashed raster at the worker boundary; no Docker or physical printer.
    let path = app
        .root
        .join("jobs")
        .join(job_id.to_string())
        .join("raster");
    std::fs::create_dir(&path).unwrap();
    let page = path.join("page-000001.tiff");
    std::fs::copy(
        app.root
            .join("jobs")
            .join(original.id.to_string())
            .join("raster/page-000001.tiff"),
        &page,
    )
    .unwrap();
    let job = app.repo.get(job_id).await.unwrap();
    assert_eq!(job.manifest.priority, 83);
    let job = app
        .repo
        .transition(job, JobState::Ripping, None)
        .await
        .unwrap();
    let job = app
        .repo
        .transition(job, JobState::Spooling, None)
        .await
        .unwrap();
    app.repo
        .complete_with_artifacts(
            job,
            &jrip_server::artifacts::hash_pages(&[page]).await.unwrap(),
        )
        .await
        .unwrap();
    let mut reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    reopened.printers = app.printers.clone();
    assert_eq!(
        reopened.repo.get(job_id).await.unwrap().manifest.priority,
        83
    );
    jrip_server::pipeline::tick(&reopened).await.unwrap();
    assert_eq!(reopened.repo.delivery(id).await.unwrap().ticket.copies, 3);
    let contract = reopened.repo.pipeline_ticket(id).await.unwrap();
    assert_eq!(contract["ticket"]["physical"]["ticket"]["copies"], 3);
    jrip_server::delivery::tick(&reopened).await.unwrap();
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    assert_eq!(fake.copies.load(Ordering::SeqCst), 3);
    assert_eq!(
        rip_raster::inspect(&fake.document.lock().unwrap())
            .unwrap()
            .len(),
        1
    );
    task.abort();
}

#[tokio::test]
async fn physical_intent_copy_modes_reject_invalid_values_and_explain_fallbacks() {
    let (_dir, app, _, fake, task) = fixture(false).await;
    for (mode, count, expected_status, resolved) in [
        ("required", 5, StatusCode::CREATED, 5),
        ("required", 6, StatusCode::UNPROCESSABLE_ENTITY, 0),
        ("preferred", 6, StatusCode::CREATED, 1),
        ("automatic", 4, StatusCode::CREATED, 1),
        ("required", 0, StatusCode::UNPROCESSABLE_ENTITY, 0),
        ("preferred", 1000, StatusCode::UNPROCESSABLE_ENTITY, 0),
        ("automatic", 0, StatusCode::UNPROCESSABLE_ENTITY, 0),
    ] {
        let mut intent = physical_intent("%PDF-1.7\nsource");
        intent["copies"] = serde_json::json!({"mode":mode,"value":count});
        let (status, value) = pipeline_request(
            &app,
            "/api/v1/print/pipeline-preflights",
            serde_json::json!({"requestId":Uuid::new_v4(),"intent":intent}),
        )
        .await;
        assert_eq!(status, expected_status, "{mode} {count}: {value}");
        if status == StatusCode::CREATED {
            assert_eq!(
                value["workflowRequest"]["copies"].as_u64().unwrap_or(1),
                resolved
            );
            if count != resolved {
                assert!(
                    value["adjustments"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|a| a["field"] == "copies" && a["resolved"] == resolved)
                );
            }
        } else {
            assert!(
                value["conflicts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v["field"] == "copies")
            );
        }
    }
    let mut intent = physical_intent("%PDF-1.7\nsource");
    intent["copies"] = serde_json::json!({"mode":"required","value":3});
    let (_, preflight) = pipeline_request(
        &app,
        "/api/v1/print/pipeline-preflights",
        serde_json::json!({"requestId":Uuid::new_v4(),"intent":intent}),
    )
    .await;
    fake.copies_max.store(1, Ordering::SeqCst);
    assert_eq!(
        commit_physical(&app, &preflight, "%PDF-1.7\nsource", false)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn physical_intent_pages_commit_canonical_ranges_and_send_selected_document() {
    let (_dir, app, original, fake, task) = fixture_pages(false, 3).await;
    let document = "%PDF-1.7\nsource";
    let id = Uuid::new_v4();
    let mut intent = physical_intent(document);
    intent["copies"] = serde_json::json!({"mode":"required","value":2});
    intent["pageRanges"] =
        serde_json::json!({"mode":"required","value":[{"first":2,"last":2},{"first":3,"last":3}]});
    let (status, preflight) = pipeline_request(
        &app,
        "/api/v1/print/pipeline-preflights",
        serde_json::json!({"requestId":id,"intent":intent}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{preflight}");
    assert_eq!(
        preflight["workflowRequest"]["pageRanges"],
        serde_json::json!([{"first":2,"last":3}])
    );
    assert_eq!(
        preflight["registrationTicket"]["request"]["pageRanges"],
        preflight["workflowRequest"]["pageRanges"]
    );
    assert!(preflight["adjustments"].as_array().unwrap().is_empty());
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    let (status, committed) = commit_physical(&app, &preflight, document, false).await;
    assert!(status.is_success(), "{committed}");
    assert_eq!(
        commit_physical(&app, &preflight, document, false).await.1["jobId"],
        committed["jobId"]
    );
    let job_id = Uuid::parse_str(committed["jobId"].as_str().unwrap()).unwrap();
    let directory = app
        .root
        .join("jobs")
        .join(job_id.to_string())
        .join("raster");
    std::fs::create_dir(&directory).unwrap();
    let original_directory = app
        .root
        .join("jobs")
        .join(original.id.to_string())
        .join("raster");
    let mut pages = Vec::new();
    for page in 1..=2 {
        let name = format!("page-{page:06}.tiff");
        let target = directory.join(&name);
        std::fs::copy(
            original_directory.join(format!("page-{:06}.tiff", page + 1)),
            &target,
        )
        .unwrap();
        pages.push(target);
    }
    let job = app.repo.get(job_id).await.unwrap();
    let job = app
        .repo
        .transition(job, JobState::Ripping, None)
        .await
        .unwrap();
    let job = app
        .repo
        .transition(job, JobState::Spooling, None)
        .await
        .unwrap();
    let mut artifacts = jrip_server::artifacts::hash_pages(&pages).await.unwrap();
    for (i, a) in artifacts.iter_mut().enumerate() {
        a.source_page = Some(i as u32 + 2);
        a.source_page_count = Some(3);
    }
    app.repo
        .complete_with_artifacts(job, &artifacts)
        .await
        .unwrap();
    let mut reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    reopened.printers = app.printers.clone();
    jrip_server::pipeline::tick(&reopened).await.unwrap();
    let ticket = reopened.repo.pipeline_ticket(id).await.unwrap();
    assert_eq!(
        ticket["ticket"]["physical"]["ticket"]["selectedSourcePages"],
        serde_json::json!([2, 3])
    );
    assert_eq!(reopened.repo.delivery(id).await.unwrap().ticket.copies, 2);
    jrip_server::delivery::tick(&reopened).await.unwrap();
    let mut expected = Vec::new();
    rip_raster::convert(
        &pages,
        &rip_raster::Options {
            dpi: 72,
            color: rip_raster::Color::DeviceRgb,
        },
        &mut expected,
    )
    .unwrap();
    for page in rip_raster::inspect(&expected).unwrap() {
        let name = b"custom_square_25.4x25.4mm";
        expected[page.offset + 1732..page.offset + 1732 + name.len()].copy_from_slice(name);
    }
    assert_eq!(*fake.document.lock().unwrap(), expected);
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    assert_eq!(fake.copies.load(Ordering::SeqCst), 2);
    assert_eq!(
        app.repo
            .artifacts(job_id)
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    task.abort();
}

#[tokio::test]
async fn physical_intent_page_modes_are_explicit_and_invalid_ranges_never_fall_back() {
    let (_dir, app, _, fake, task) = fixture(false).await;
    for mode in ["required", "preferred", "automatic"] {
        let mut intent = physical_intent("%PDF-1.7\nsource");
        intent["pageRanges"] = serde_json::json!({"mode":mode,"value":[{"first":2,"last":3}]});
        let (status, value) = pipeline_request(
            &app,
            "/api/v1/print/pipeline-preflights",
            serde_json::json!({"requestId":Uuid::new_v4(),"intent":intent}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{value}");
        if mode == "automatic" {
            assert!(value["workflowRequest"].get("pageRanges").is_none());
            assert!(
                value["adjustments"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a["field"] == "pageRanges"
                        && a["resolved"] == serde_json::json!([])
                        && a["reasonCode"] == "AUTOMATIC_SELECTION")
            );
        } else {
            assert_eq!(
                value["workflowRequest"]["pageRanges"],
                serde_json::json!([{"first":2,"last":3}])
            );
        }
        for ranges in [
            serde_json::json!([{"first":0,"last":1}]),
            serde_json::json!([{"first":4,"last":3}]),
            serde_json::json!([{"first":1,"last":101}]),
            serde_json::json!([{"first":1,"last":3},{"first":3,"last":4}]),
            serde_json::json!([{"first":3,"last":4},{"first":1,"last":2}]),
        ] {
            let mut intent = physical_intent("%PDF-1.7\nsource");
            intent["pageRanges"] = serde_json::json!({"mode":mode,"value":ranges});
            let (status, value) = pipeline_request(
                &app,
                "/api/v1/print/pipeline-preflights",
                serde_json::json!({"requestId":Uuid::new_v4(),"intent":intent}),
            )
            .await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(
                value["conflicts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v["code"] == "invalid_page_ranges")
            );
        }
    }
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn physical_intent_inspection_bounds_ranges_and_checks_only_selected_media() {
    let (_dir, mut app, _, fake, task) = fixture(false).await;
    app.credentials = vec![jrip_server::Credential {
        validity: None,
        token: "token".into(),
        identity: "owner".into(),
        scopes: vec!["admin".into()],
    }];
    let document = "%PDF-1.7\nsource";
    let inspection = Uuid::new_v4();
    let report = serde_json::json!({"schemaVersion":1,"inspectionId":inspection,"inspectorRevision":"jrip-mupdf-inspection-v1","sourceSha256":format!("{:x}",Sha256::digest(document)),"sourceBytes":document.len(),"pages":[{"page":1,"widthPoints":144.0,"heightPoints":144.0,"rotation":0},{"page":2,"widthPoints":72.0,"heightPoints":72.0,"rotation":0}],"contentParsed":true,"repaired":false,"encrypted":false});
    app.repo
        .save_document_inspection(inspection, "owner", &report)
        .await
        .unwrap();
    for (ranges, expected) in [
        (
            serde_json::json!([{"first":2,"last":2}]),
            StatusCode::CREATED,
        ),
        (
            serde_json::json!([{"first":3,"last":3}]),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            serde_json::json!([{"first":1,"last":1}]),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (serde_json::json!([]), StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let mut intent = physical_intent(document);
        intent["pageRanges"] = serde_json::json!({"mode":"required","value":ranges});
        let (status,value)=pipeline_request(&app,"/api/v1/print/pipeline-preflights",serde_json::json!({"requestId":Uuid::new_v4(),"intent":intent,"inspectionId":inspection})).await;
        assert_eq!(status, expected, "{value}");
        if ranges == serde_json::json!([{"first":3,"last":3}]) {
            assert_eq!(value["error"]["code"], "page_range_out_of_bounds");
        }
    }
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

async fn duplex_request(
    app: &jrip_server::App,
    job: &Job,
    id: Uuid,
    sides: &str,
    preview: bool,
    approval: Option<Uuid>,
) -> (StatusCode, serde_json::Value) {
    let mut req = Request::post(format!(
        "/api/v1/jobs/{}/{}",
        job.id,
        if preview {
            "delivery-preview"
        } else {
            "deliveries"
        }
    ))
    .header("authorization", "Bearer token")
    .header("content-type", "application/json");
    if let Some(approval) = approval {
        req = req.header("x-jrip-approval-id", approval.to_string());
    }
    let response=jrip_server::router(app.clone()).oneshot(req.body(Body::from(serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm","sides":sides,"copies":2}).to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}
#[tokio::test]
async fn normal_duplex_preview_ticket_restart_and_native_sides_agree() {
    let (_dir, app, job, fake, task) = fixture_pages(false, 3).await;
    fake.duplex.store(1, Ordering::SeqCst);
    for (sides, binding) in [
        ("two-sided-long-edge", rip_raster::DuplexBinding::LongEdge),
        ("two-sided-short-edge", rip_raster::DuplexBinding::ShortEdge),
    ] {
        let id = Uuid::new_v4();
        let (status, preview) = duplex_request(&app, &job, id, sides, true, None).await;
        assert_eq!(status, StatusCode::OK, "{preview}");
        assert_eq!(preview["contract"]["ticket"]["sides"], sides);
        assert_eq!(preview["contract"]["ticket"]["sheetBack"], "normal");
        assert_eq!(
            duplex_request(&app, &job, id, sides, false, None).await.0,
            StatusCode::ACCEPTED
        );
        assert_eq!(
            duplex_request(&app, &job, id, "one-sided", false, None)
                .await
                .0,
            StatusCode::CONFLICT
        );
        let mut reopened = jrip_server::open(app.root.clone(), "token".into())
            .await
            .unwrap();
        reopened.printers = app.printers.clone();
        assert_eq!(
            reopened
                .repo
                .delivery(id)
                .await
                .unwrap()
                .ticket
                .sides
                .as_str(),
            sides
        );
        jrip_server::delivery::tick(&reopened).await.unwrap();
        assert_eq!(
            rip_raster::inspect_normal_duplex(&fake.document.lock().unwrap(), binding)
                .unwrap()
                .len(),
            3
        );
        assert_eq!(fake.copies.load(Ordering::SeqCst), 2);
    }
    assert_eq!(fake.prints.load(Ordering::SeqCst), 2);
    task.abort();
}
#[tokio::test]
async fn duplex_never_sends_without_exact_capability_and_approval() {
    let (_dir, mut app, job, fake, task) = fixture(false).await;
    let id = Uuid::new_v4();
    assert_eq!(
        duplex_request(&app, &job, id, "two-sided-long-edge", false, None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    fake.duplex.store(3, Ordering::SeqCst);
    assert_eq!(
        duplex_request(&app, &job, id, "two-sided-long-edge", false, None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    fake.duplex.store(1, Ordering::SeqCst);
    app.credentials = vec![jrip_server::Credential {
        validity: None,
        token: "token".into(),
        identity: "operator".into(),
        scopes: vec!["admin".into()],
    }];
    app.require_print_approval = true;
    let (status,approval)=pipeline_request(&app,"/api/v1/approvals",serde_json::json!({"identity":"operator","jobId":job.id,"expectedRevision":job.revision,"requestId":id,"printerId":"test","media":"custom_square_25.4x25.4mm","copies":2,"sides":"two-sided-long-edge","reason":"duplex proof"})).await;
    assert_eq!(status, StatusCode::CREATED);
    let approval = Uuid::parse_str(approval["id"].as_str().unwrap()).unwrap();
    for wrong in ["one-sided", "two-sided-short-edge"] {
        assert_eq!(
            duplex_request(&app, &job, id, wrong, false, Some(approval))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        duplex_request(&app, &job, id, "two-sided-long-edge", false, Some(approval))
            .await
            .0,
        StatusCode::ACCEPTED
    );
    fake.duplex.store(3, Ordering::SeqCst);
    jrip_server::delivery::tick(&app).await.unwrap();
    let rejected = app.repo.delivery(id).await.unwrap();
    assert_eq!(rejected.state, DeliveryState::Rejected);
    assert_eq!(
        rejected.reason.as_deref(),
        Some("unsupported_sides_or_sheet_back")
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}
#[tokio::test]
async fn physical_duplex_intent_and_follow_on_pipeline_preserve_binding() {
    let (_dir, app, job, fake, task) = fixture_pages(false, 3).await;
    fake.duplex.store(1, Ordering::SeqCst);
    let mut intent = physical_intent("%PDF-1.7\nsource");
    intent["sides"] = serde_json::json!({"mode":"required","value":"two-sided-short-edge"});
    let (status, pf) = pipeline_request(
        &app,
        "/api/v1/print/pipeline-preflights",
        serde_json::json!({"requestId":Uuid::new_v4(),"intent":intent}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{pf}");
    assert_eq!(pf["workflowRequest"]["sides"], "two-sided-short-edge");
    let (commit_status, committed) = commit_physical(&app, &pf, "%PDF-1.7\nsource", false).await;
    assert!(commit_status.is_success(), "{committed}");
    let workflow_id =
        Uuid::parse_str(pf["workflowRequest"]["requestId"].as_str().unwrap()).unwrap();
    assert_eq!(
        app.repo.pipeline(workflow_id).await.unwrap()["ticket"]["request"]["sides"],
        "two-sided-short-edge"
    );
    fake.duplex.store(3, Ordering::SeqCst);
    assert_eq!(
        pipeline_request(
            &app,
            "/api/v1/print/pipeline-preflights",
            serde_json::json!({"requestId":Uuid::new_v4(),"intent":intent})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    fake.duplex.store(1, Ordering::SeqCst);
    let id = Uuid::new_v4();
    assert_eq!(pipeline_request(&app,&format!("/api/v1/jobs/{}/pipelines",job.id),serde_json::json!({"requestId":id,"expectedRevision":job.revision,"printerId":"test","media":"custom_square_25.4x25.4mm","sides":"two-sided-short-edge","pageRanges":[{"first":2,"last":3}]})).await.0,StatusCode::ACCEPTED);
    let mut reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    reopened.printers = app.printers.clone();
    jrip_server::pipeline::tick(&reopened).await.unwrap();
    let ticket = reopened.repo.pipeline_ticket(id).await.unwrap();
    assert_eq!(
        ticket["ticket"]["physical"]["ticket"]["sides"],
        "two-sided-short-edge"
    );
    jrip_server::delivery::tick(&reopened).await.unwrap();
    assert_eq!(
        rip_raster::inspect_normal_duplex(
            &fake.document.lock().unwrap(),
            rip_raster::DuplexBinding::ShortEdge
        )
        .unwrap()
        .len(),
        2
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn all_sheet_back_modes_are_persisted_encoded_and_sent_with_exact_live_binding() {
    let (_dir, app, job, fake, task) =
        fixture_with_source_hash(false, 3, None, &format!("{:x}", Sha256::digest(b"source"))).await;
    let mut sent = 0;
    for mode in [1, 2, 4, 5] {
        for (sides, binding) in [
            ("two-sided-long-edge", rip_raster::DuplexBinding::LongEdge),
            ("two-sided-short-edge", rip_raster::DuplexBinding::ShortEdge),
        ] {
            fake.duplex.store(mode, Ordering::SeqCst);
            let id = Uuid::new_v4();
            let back: rip_core::print::SheetBack = fake_back(mode).parse().unwrap();
            let (status, preview) = duplex_request(&app, &job, id, sides, true, None).await;
            assert_eq!(status, StatusCode::OK, "{preview}");
            assert_eq!(preview["contract"]["ticket"]["sheetBack"], back.as_str());
            assert_eq!(
                duplex_request(&app, &job, id, sides, false, None).await.0,
                StatusCode::ACCEPTED
            );
            let mut reopened = jrip_server::open(app.root.clone(), "token".into())
                .await
                .unwrap();
            reopened.printers = app.printers.clone();
            assert_eq!(
                reopened.repo.delivery(id).await.unwrap().ticket.sheet_back,
                back
            );
            jrip_server::delivery::tick(&reopened).await.unwrap();
            sent += 1;
            let response = jrip_server::router(reopened.clone())
                .oneshot(
                    Request::get(format!("/api/v1/deliveries/{id}/artifact/descriptor"))
                        .header("authorization", "Bearer token")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let descriptor: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(descriptor["sheetBack"], back.as_str());
            assert_eq!(descriptor["sides"], sides);
            assert_eq!(descriptor["pageCount"], 3);

            assert_eq!(fake.prints.load(Ordering::SeqCst), sent);
            assert_eq!(
                rip_raster::inspect_duplex(
                    &fake.document.lock().unwrap(),
                    rip_raster::DuplexLayout {
                        binding,
                        sheet_back: back
                    }
                )
                .unwrap()
                .len(),
                3
            );
        }
    }
    // Normal/long-edge and manual-tumble/long-edge have identical raster transforms;
    // capability changes must still reject rather than infer mode from bytes.
    fake.duplex.store(1, Ordering::SeqCst);
    let id = Uuid::new_v4();
    assert_eq!(
        duplex_request(&app, &job, id, "two-sided-long-edge", false, None)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    fake.duplex.store(5, Ordering::SeqCst);
    jrip_server::delivery::tick(&app).await.unwrap();
    let rejected = app.repo.delivery(id).await.unwrap();
    assert_eq!(rejected.state, DeliveryState::Rejected);
    assert_eq!(
        rejected.reason.as_deref(),
        Some("unsupported_sides_or_sheet_back")
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), sent);
    task.abort();
}

#[tokio::test]
async fn rendered_subsets_keep_original_numbers_and_reject_missing_source_pages() {
    let ranges =
        serde_json::from_value(serde_json::json!([{"first":2,"last":2},{"first":5,"last":5}]))
            .unwrap();
    let (_dir, app, job, fake, task) = fixture_mapped_pages(false, 2, Some((ranges, 6))).await;
    for (request, expected) in [
        (serde_json::json!([]), serde_json::json!([2, 5])),
        (
            serde_json::json!([{"first":5,"last":5}]),
            serde_json::json!([5]),
        ),
    ] {
        let (status, value) = ranges_request(&app, &job, Uuid::new_v4(), request, true, None).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert_eq!(value["contract"]["ticket"]["sourcePageCount"], 6);
        assert_eq!(value["contract"]["ticket"]["renderedPageCount"], 2);
        assert_eq!(value["contract"]["ticket"]["selectedSourcePages"], expected);
    }
    let (status, value) = ranges_request(
        &app,
        &job,
        Uuid::new_v4(),
        serde_json::json!([{"first":3,"last":3}]),
        true,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    assert!(value.to_string().contains("page_not_rendered"));
    let id = Uuid::new_v4();
    assert_eq!(
        ranges_request(&app, &job, id, serde_json::json!([]), false, None)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    jrip_server::delivery::tick(&app).await.unwrap();
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    assert_eq!(
        rip_raster::inspect(&fake.document.lock().unwrap())
            .unwrap()
            .len(),
        2
    );
    task.abort();
}

#[tokio::test]
async fn source_delivery_history_is_private_bounded_stable_and_survives_restart() {
    let (dir, mut app, job, fake, task) = fixture(false).await;
    for id in [Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)] {
        assert_eq!(prepare(&app, &job, id).await, StatusCode::ACCEPTED);
    }
    app.credentials = vec![
        jrip_server::Credential {
            token: "reader".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
            validity: None,
        },
        jrip_server::Credential {
            token: "printer".into(),
            identity: "printer".into(),
            scopes: vec!["printers:print".into()],
            validity: None,
        },
    ];
    async fn get(
        app: &jrip_server::App,
        path: &str,
        token: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut req = Request::get(path);
        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        let response = jrip_server::router(app.clone())
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        if status == StatusCode::OK {
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
        (
            status,
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap(),
        )
    }
    let path = format!("/api/v1/jobs/{}/deliveries", job.id);
    assert_eq!(get(&app, &path, None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        get(&app, &path, Some("printer")).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, first) = get(&app, &format!("{path}?limit=1"), Some("reader")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["deliveries"][0]["id"], Uuid::from_u128(3).to_string());
    assert_eq!(first["hasMore"], true);
    let mut second_job = job.clone();
    second_job.id = Uuid::new_v4();
    app.repo.insert(&second_job).await.unwrap();
    let original = app.repo.delivery(Uuid::from_u128(3)).await.unwrap().ticket;
    let mut other_ticket = original.clone();
    other_ticket.source_job = second_job.id;
    app.repo
        .enqueue_delivery(Uuid::from_u128(5), other_ticket)
        .await
        .unwrap();
    app.repo
        .enqueue_delivery(Uuid::from_u128(4), original)
        .await
        .unwrap();
    let cursor = first["nextBefore"].as_str().unwrap();
    let (_, rest) = get(&app, &format!("{path}?before={cursor}"), Some("reader")).await;
    assert_eq!(rest["deliveries"].as_array().unwrap().len(), 2);
    assert_eq!(rest["hasMore"], false);
    assert!(rest["nextBefore"].is_null());
    for query in [
        "limit=0",
        "limit=101",
        "limit=bad",
        "before=bad",
        "before=-1:00000000-0000-0000-0000-000000000001",
        "offset=1",
    ] {
        let (status, value) = get(&app, &format!("{path}?{query}"), Some("reader")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["error"]["code"], "invalid_delivery_query");
    }
    let mut reopened = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    reopened.credentials = app.credentials.clone();
    assert_eq!(
        get(
            &reopened,
            &format!("{path}?before={cursor}"),
            Some("reader")
        )
        .await
        .1,
        rest
    );
    assert_eq!(
        get(
            &app,
            &format!("/api/v1/jobs/{}/deliveries", Uuid::new_v4()),
            Some("reader")
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
#[ignore = "requires Playwright Chromium and jrip-color:local Docker image"]
async fn operator_console_physical_delivery_is_reviewed_approved_recoverable_and_cancellable() {
    let (dir, mut app, job, fake, fake_task) = fixture_pages(false, 3).await;
    fake.duplex.store(1, Ordering::SeqCst);
    let profile = lcms2::Profile::new_srgb().icc().unwrap();
    let record = rip_color::registry::Registry::open(app.root.join("color-profiles"))
        .unwrap()
        .register(&profile)
        .unwrap();
    let color_id = Uuid::new_v4();
    let request = serde_json::json!({"requestId":color_id,"expectedRevision":job.revision,"inputProfileSha256":record.sha256,"outputProfileSha256":record.sha256,"policy":{"schemaVersion":1,"inputColor":"rgb","outputColor":"rgb","intent":"relative_colorimetric","blackPointCompensation":false,"blackPreservation":"none","tacPercent":null}});
    assert_eq!(
        pipeline_request(
            &app,
            &format!("/api/v1/jobs/{}/color-attempts", job.id),
            request
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    jrip_server::color::tick(&app).await.unwrap();
    assert_eq!(
        app.repo.color_attempt(color_id).await.unwrap()["state"],
        "COMPLETED"
    );
    app.printers[0].color_target = Some(jrip_server::delivery::ColorTarget {
        output_profile_sha256: Some(record.sha256),
        device_link_sha256: None,
    });
    app.require_print_approval = true;
    app.credentials = vec![
        jrip_server::Credential {
            token: "token".into(),
            identity: "operator".into(),
            scopes: vec!["admin".into()],
            validity: None,
        },
        jrip_server::Credential {
            token: "reader".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into(), "artifacts:read".into()],
            validity: None,
        },
    ];
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let worker_app = app.clone();
    let worker = tokio::spawn(async move {
        loop {
            jrip_server::delivery::tick(&worker_app).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });
    let control = fake.clone();
    let printer_control = fake.clone();
    let test_router = jrip_server::router(app).route(
        "/__test/device",
        post(move |axum::Json(value): axum::Json<serde_json::Value>| {
            let control = control.clone();
            async move {
                *control.reasons.lock().unwrap() = value["reasons"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect();
                control.progress_mode.store(
                    value["progressMode"].as_u64().unwrap_or(0) as usize,
                    Ordering::SeqCst,
                );
                control
                    .state
                    .store(value["state"].as_u64().unwrap() as usize, Ordering::SeqCst);
                StatusCode::NO_CONTENT
            }
        }),
    );
    let test_router = test_router.route(
        "/__test/printer",
        post(move |axum::Json(value): axum::Json<serde_json::Value>| {
            let control = printer_control.clone();
            async move {
                control.printer_state.store(
                    value["state"].as_u64().unwrap_or(3) as usize,
                    Ordering::SeqCst,
                );
                *control.printer_reasons.lock().unwrap() = value["reasons"]
                    .as_array()
                    .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
                    .unwrap_or_default();
                control.marker_mode.store(
                    value["markerMode"].as_u64().unwrap_or(0) as usize,
                    Ordering::SeqCst,
                );
                control.pwg_supply_mode.store(
                    value["pwgMode"].as_u64().unwrap_or(0) as usize,
                    Ordering::SeqCst,
                );
                control.queue_count.store(
                    value["queuedJobs"]
                        .as_u64()
                        .map(|n| n as usize)
                        .unwrap_or(usize::MAX),
                    Ordering::SeqCst,
                );
                StatusCode::NO_CONTENT
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, test_router).await.unwrap() });
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = tokio::process::Command::new("node")
        .arg("tests/ui/delivery.mjs")
        .current_dir(workspace)
        .env("JRIP_UI_URL", format!("http://{address}/console"))
        .env("JRIP_UI_TOKEN", "token")
        .env("JRIP_UI_COLOR", color_id.to_string())
        .status()
        .await
        .unwrap();
    worker.abort();
    server.abort();
    fake_task.abort();
    assert!(status.success(), "Delivery browser flow failed: {status}");
    assert_eq!(fake.prints.load(Ordering::SeqCst), 3);
    assert_eq!(fake.copies.load(Ordering::SeqCst), 1);
    drop(dir);
}
