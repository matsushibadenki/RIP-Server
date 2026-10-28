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
    let (_dir, mut app, job, fake, task) = fixture(false).await;
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
    prints: Arc<AtomicUsize>,
    state: Arc<AtomicUsize>,
    malformed: bool,
    job_name: Arc<std::sync::Mutex<String>>,
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
            r.group(4);
            r.text(0x49, "document-format-supported", "image/pwg-raster");
            r.attribute(0x22, "printer-is-accepting-jobs", &[1]);
            r.attribute(0x23, "printer-state", &3i32.to_be_bytes());
            r.resolution_dpi("pwg-raster-document-resolution-supported", 72);
            r.text(0x44, "pwg-raster-document-type-supported", "rgb_8");
            r.text(0x44, "media-supported", "custom_square_25.4x25.4mm");
        }
        rip_ipp::PRINT_JOB => {
            assert_eq!(q.text(1, "document-format", 0x49), Some("image/pwg-raster"));
            assert_eq!(q.text(2, "media", 0x44), Some("custom_square_25.4x25.4mm"));
            assert_eq!(rip_raster::inspect(q.document).unwrap().len(), 1);
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
            r.group(2);
            r.integer("job-id", 42);
            r.text(0x42, "job-name", &f.job_name.lock().unwrap());
            r.attribute(
                0x23,
                "job-state",
                &(f.state.load(Ordering::SeqCst) as i32).to_be_bytes(),
            );
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
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    let f = Fake {
        prints: Arc::new(AtomicUsize::new(0)),
        state: Arc::new(AtomicUsize::new(3)),
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
    let job = Job {
        id: Uuid::new_v4(),
        manifest: serde_json::from_value::<Manifest>(
            serde_json::json!({"name":"test.pdf","format":"application/pdf","dpi":72,"color_mode":"rgb"}),
        )
        .unwrap(),
        state: JobState::Spooling,
        source_sha256: "source".into(),
        created_at: 0,
        updated_at: 0,
        revision: 0,
        error_code: None,
        selected_engine: None,
        worker_run_id: None,
    };
    app.repo.insert(&job).await.unwrap();
    let path = app
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("raster");
    std::fs::create_dir_all(&path).unwrap();
    let file = path.join("page-000001.tiff");
    tiff::encoder::TiffEncoder::new(std::fs::File::create(&file).unwrap())
        .unwrap()
        .write_image::<tiff::encoder::colortype::RGB8>(72, 72, &vec![128; 72 * 72 * 3])
        .unwrap();
    let bytes = std::fs::read(file).unwrap();
    app.repo
        .complete_with_artifacts(
            job.clone(),
            &[Artifact {
                page: 1,
                name: "page-000001.tiff".into(),
                media_type: "image/tiff".into(),
                bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(bytes)),
            }],
        )
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
    f.state.store(9, Ordering::SeqCst);
    jrip_server::delivery::tick(&reopened).await.unwrap();
    jrip_server::pipeline::tick(&reopened).await.unwrap();
    assert_eq!(app.repo.pipeline(id).await.unwrap()["state"], "COMPLETED");
    assert_eq!(f.prints.load(Ordering::SeqCst), 1);
    let complete = reopened.repo.pipeline_ticket(id).await.unwrap();
    assert_eq!(complete["execution"]["completed"], true);
    let receipt = app.repo.delivery_result_receipt(id).await.unwrap();
    assert_eq!(receipt["status"], "succeeded");
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
