use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use rip_core::{Job, JobState};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
async fn seed() -> (tempfile::TempDir, jrip_server::App, Job, String) {
    let directory = tempfile::tempdir().unwrap();
    let app = jrip_server::open(directory.path().into(), "admin-token".into())
        .await
        .unwrap();
    let job = Job {
        id: Uuid::new_v4(),
        manifest: serde_json::from_value(
            json!({"name":"source.pdf","format":"application/pdf","dpi":300,"color_mode":"rgb"}),
        )
        .unwrap(),
        state: JobState::Spooling,
        source_sha256: "a".repeat(64),
        created_at: rip_storage::now(),
        updated_at: rip_storage::now(),
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
    let mut files = Vec::new();
    let profile = lcms2::Profile::new_srgb().icc().unwrap();
    for page in 1..=2 {
        let file = path.join(format!("page-{page:06}.tiff"));
        let mut encoder =
            tiff::encoder::TiffEncoder::new(std::fs::File::create(&file).unwrap()).unwrap();
        let mut image = encoder
            .new_image::<tiff::encoder::colortype::RGB8>(2, 1)
            .unwrap();
        image.resolution(
            tiff::tags::ResolutionUnit::Inch,
            tiff::encoder::Rational { n: 300, d: 1 },
        );
        image
            .encoder()
            .write_tag(tiff::tags::Tag::Unknown(34675), profile.as_slice())
            .unwrap();
        image.write_data(&[255, 0, 0, 0, 0, 255]).unwrap();
        files.push(file);
    }
    app.repo
        .complete_with_artifacts(
            job.clone(),
            &jrip_server::artifacts::hash_pages(&files).await.unwrap(),
        )
        .await
        .unwrap();
    let job = app.repo.get(job.id).await.unwrap();
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::post("/api/v1/color/profiles")
                .header("authorization", "Bearer admin-token")
                .body(Body::from(profile))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let value: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (
        directory,
        app,
        job,
        value["sha256"].as_str().unwrap().into(),
    )
}
fn ticket(job: &Job, id: Uuid, digest: &str) -> Value {
    json!({"requestId":id,"expectedRevision":job.revision,"inputProfileSha256":digest,"outputProfileSha256":digest,"policy":{"schemaVersion":1,"inputColor":"rgb","outputColor":"rgb","intent":"relative_colorimetric","blackPointCompensation":false,"blackPreservation":"none","tacPercent":null}})
}
async fn call(app: &jrip_server::App, path: &str, body: Value) -> (StatusCode, Value) {
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::post(path)
                .header("authorization", "Bearer admin-token")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, body)
}
#[tokio::test]
async fn color_queue_is_scoped_idempotent_cancellable_and_pins_source_retention() {
    let (_dir, mut app, job, digest) = seed().await;
    app.credentials = vec![
        jrip_server::Credential {
            token: "admin-token".into(),
            identity: "admin".into(),
            scopes: vec!["admin".into()],
        },
        jrip_server::Credential {
            token: "reader".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
        },
    ];
    let denied = jrip_server::router(app.clone())
        .oneshot(
            Request::post("/api/v1/color/profiles")
                .header("authorization", "Bearer reader")
                .body(Body::from(vec![0; 132]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let id = Uuid::new_v4();
    let path = format!("/api/v1/jobs/{}/color-attempts", job.id);
    let request = ticket(&job, id, &digest);
    assert_eq!(
        call(&app, &path, request.clone()).await.0,
        StatusCode::ACCEPTED
    );
    assert_eq!(call(&app, &path, request.clone()).await.1["revision"], 0);
    let mut changed = request;
    changed["policy"]["blackPointCompensation"] = json!(true);
    assert_eq!(call(&app, &path, changed).await.0, StatusCode::CONFLICT);
    assert!(app.repo.delete(job.clone()).await.is_err());
    assert!(
        app.repo
            .claim_retention(job.id, rip_storage::now() + 1)
            .await
            .is_err()
    );
    let cancel = format!("/api/v1/color/attempts/{id}/cancel");
    assert_eq!(
        call(&app, &cancel, json!({"expectedRevision":1})).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(&app, &cancel, json!({"expectedRevision":0})).await.1["state"],
        "CANCELLED"
    );
    app.repo.delete(job).await.unwrap();
    assert_eq!(
        app.repo.color_events(id, 0, 100).await.unwrap()["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[tokio::test]
#[ignore = "requires locally built jrip-color:local Docker image"]
async fn isolated_worker_publishes_two_pages_reports_and_preserves_original() {
    let (_dir, app, job, digest) = seed().await;
    let originals = app.repo.artifacts(job.id).await.unwrap();
    let id = Uuid::new_v4();
    assert_eq!(
        call(
            &app,
            &format!("/api/v1/jobs/{}/color-attempts", job.id),
            ticket(&job, id, &digest)
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    assert!(jrip_server::color::tick(&app).await.unwrap());
    let attempt = app.repo.color_attempt(id).await.unwrap();
    assert_eq!(attempt["state"], "COMPLETED", "{attempt}");
    assert_eq!(attempt["completedPages"], 2);
    assert_eq!(attempt["result"]["pageCount"], 2);
    assert_eq!(
        attempt["result"]["reports"][0]["sourceProfileEvidence"],
        "embedded-match"
    );
    assert_eq!(attempt["result"]["reports"][0]["warnings"], json!([]));
    assert_eq!(app.repo.artifacts(job.id).await.unwrap(), originals);
    let output = app
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("color")
        .join(id.to_string());
    for page in 1..=2 {
        let file = output.join(format!("page-{page:06}.tiff"));
        let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(file).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (2, 1));
        assert!(
            decoder
                .find_tag(tiff::tags::Tag::Unknown(34675))
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(
        app.repo.color_events(id, 0, 100).await.unwrap()["events"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    let history = app.repo.color_events(id, 0, 100).await.unwrap();
    let events = history["events"].as_array().unwrap();
    assert_eq!(events[2]["progressPercent"], 50);
    assert_eq!(events[3]["progressPercent"], 100);
    assert_eq!(events[3]["state"], "RUNNING");
    assert_eq!(events[3]["totalPages"], 2);
    assert_eq!(events[3]["progressPhase"], "color-conversion");
    assert!(events[3]["completionKind"].is_null());
    assert_eq!(events[4]["completionKind"], "color-managed-tiff-export");
    assert!(!jrip_server::color::tick(&app).await.unwrap());
    // Tampering is caught before native interpretation and leaves no published color output.
    let failed = Uuid::new_v4();
    assert_eq!(
        call(
            &app,
            &format!("/api/v1/jobs/{}/color-attempts", job.id),
            ticket(&job, failed, &digest)
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    std::fs::write(
        app.root
            .join("jobs")
            .join(job.id.to_string())
            .join("raster/page-000001.tiff"),
        b"tampered",
    )
    .unwrap();
    jrip_server::color::tick(&app).await.unwrap();
    assert_eq!(
        app.repo.color_attempt(failed).await.unwrap()["state"],
        "FAILED"
    );
    assert!(!output.parent().unwrap().join(failed.to_string()).exists());
}
