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
            token: "admin-token-at-least-24-characters".into(),
            identity: "operator".into(),
            scopes: vec!["admin".into()],
        },
        jrip_server::Credential {
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
    assert_eq!(f.prints.load(Ordering::SeqCst), 0);
    task.abort();
}
