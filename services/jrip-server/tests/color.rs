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

#[tokio::test]
async fn immutable_color_policies_are_scoped_durable_and_used_by_attempts() {
    let (dir, mut app, job, icc) = seed().await;
    let definition = json!({"schemaVersion":1,"labels":{"en":"Proof","ja":"校正","zh-CN":"校样"},"inputProfileSha256":icc,"outputProfileSha256":icc,"policy":ticket(&job,Uuid::new_v4(),&icc)["policy"]});
    let save = json!({"definition":definition,"reason":"approved proof settings"});
    let (status, stored) = call(&app, "/api/v1/color/policies", save.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{stored}");
    let digest = stored["digest"].as_str().unwrap();
    assert_eq!(
        format!("sha256:{digest}"),
        rip_digest::digest(&definition).unwrap()
    );
    let (_, replayed) = call(
        &app,
        "/api/v1/color/policies",
        json!({"definition":definition,"reason":"retry"}),
    )
    .await;
    assert_eq!(stored, replayed);
    let mut revised = definition.clone();
    revised["policy"]["blackPointCompensation"] = json!(true);
    let (_, new_version) = call(
        &app,
        "/api/v1/color/policies",
        json!({"definition":revised,"reason":"new proof variant"}),
    )
    .await;
    assert_ne!(new_version["digest"], stored["digest"]);
    let audits = app.repo.audit_records(0, 100).await.unwrap();
    assert_eq!(
        audits["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["operation"] == "color-policy-create")
            .count(),
        2
    );
    let mut bad = definition.clone();
    bad["inputProfileSha256"] = json!("f".repeat(64));
    assert_eq!(
        call(
            &app,
            "/api/v1/color/policies",
            json!({"definition":bad,"reason":"invalid"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut bad = save.clone();
    bad["reason"] = json!("");
    assert_eq!(
        call(&app, "/api/v1/color/policies", bad).await.0,
        StatusCode::BAD_REQUEST
    );
    app.credentials = vec![
        jrip_server::Credential {
            token: "reader".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
        },
        jrip_server::Credential {
            token: "admin-token".into(),
            identity: "operator".into(),
            scopes: vec!["jobs:submit".into(), "artifacts:read".into()],
        },
    ];
    assert_eq!(
        call(&app, "/api/v1/color/policies", save).await.0,
        StatusCode::FORBIDDEN
    );
    let get = |path: &str| {
        Request::get(path)
            .header("authorization", "Bearer reader")
            .body(Body::empty())
            .unwrap()
    };
    let response = jrip_server::router(app.clone())
        .oneshot(get("/api/v1/color/policies?limit=1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(page["policies"].as_array().unwrap().len(), 1);
    let cursor = page["nextAfter"].as_str().unwrap();
    let response = jrip_server::router(app.clone())
        .oneshot(get(&format!(
            "/api/v1/color/policies?limit=1&after={cursor}"
        )))
        .await
        .unwrap();
    let page2: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(page2["policies"].as_array().unwrap().len(), 1);
    assert!(page2["nextAfter"].is_null());
    assert_ne!(
        page["policies"][0]["digest"],
        page2["policies"][0]["digest"]
    );
    assert_eq!(
        jrip_server::router(app.clone())
            .oneshot(get("/api/v1/color/policies?limit=101"))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let mut reopened = jrip_server::open(dir.path().into(), "admin-token".into())
        .await
        .unwrap();
    reopened.credentials = app.credentials.clone();
    assert_eq!(reopened.repo.color_policy(digest).await.unwrap(), stored);
    let request_id = Uuid::new_v4();
    let req = json!({"requestId":request_id,"expectedRevision":job.revision,"policyDigest":digest});
    let path = format!("/api/v1/jobs/{}/color-attempts/from-policy", job.id);
    let (status, attempt) = call(&reopened, &path, req.clone()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{attempt}");
    assert_eq!(attempt["ticket"]["policyReference"], digest);
    assert_eq!(attempt["ticket"]["policy"], definition["policy"]);
    assert_eq!(call(&reopened, &path, req).await.1, attempt);
    let changed = json!({"requestId":request_id,"expectedRevision":job.revision,"policyDigest":new_version["digest"]});
    assert_eq!(
        call(&reopened, &path, changed).await.0,
        StatusCode::CONFLICT
    );
    // A policy cannot be combined with inline overrides.
    let mixed = json!({"requestId":Uuid::new_v4(),"expectedRevision":job.revision,"policyDigest":digest,"policy":definition["policy"]});
    assert_eq!(
        jrip_server::router(reopened)
            .oneshot(
                Request::post(&path)
                    .header("authorization", "Bearer admin-token")
                    .header("content-type", "application/json")
                    .body(Body::from(mixed.to_string()))
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn input_profile_evidence_is_hash_verified_scoped_and_never_selects_implicitly() {
    let (_dir, mut app, job, icc) = seed().await;
    let path = format!("/api/v1/jobs/{}/color-input-profile", job.id);
    let get = |token: &str| {
        Request::get(&path)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };
    let response = jrip_server::router(app.clone())
        .oneshot(get("admin-token"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let report: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(report["status"], "EXPLICIT_PROFILE_AVAILABLE");
    assert_eq!(report["candidateProfileSha256"], icc);
    assert_eq!(report["automaticSelection"], false);
    assert_eq!(report["evidence"]["pages"].as_array().unwrap().len(), 2);
    assert_eq!(
        report["evidenceDigest"],
        rip_digest::digest(&report["evidence"]).unwrap()
    );
    assert!(app.repo.next_color().await.unwrap().is_none());
    app.credentials = vec![jrip_server::Credential {
        token: "reader".into(),
        identity: "reader".into(),
        scopes: vec!["jobs:read".into()],
    }];
    assert_eq!(
        jrip_server::router(app.clone())
            .oneshot(get("reader"))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    app.credentials.clear();
    std::fs::remove_file(app.root.join("color-profiles").join(format!("{icc}.icc"))).unwrap();
    let response = jrip_server::router(app.clone())
        .oneshot(get("admin-token"))
        .await
        .unwrap();
    let report: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(report["status"], "EXPLICIT_SELECTION_REQUIRED");
    assert!(report["candidateProfileSha256"].is_null());
    assert_eq!(report["evidence"]["pages"][0]["registered"], false);
    let mut mixed_job = job.clone();
    mixed_job.id = Uuid::new_v4();
    mixed_job.state = JobState::Spooling;
    mixed_job.revision = 0;
    app.repo.insert(&mixed_job).await.unwrap();
    let root = app
        .root
        .join("jobs")
        .join(mixed_job.id.to_string())
        .join("raster");
    std::fs::create_dir_all(&root).unwrap();
    let registry = rip_color::registry::Registry::open(app.root.join("color-profiles")).unwrap();
    let mut files = Vec::new();
    for page in 1..=2 {
        let mut profile = lcms2::Profile::new_srgb().icc().unwrap();
        profile[64..68].copy_from_slice(&(page as u32 - 1).to_be_bytes());
        registry.register(&profile).unwrap();
        let file = root.join(format!("page-{page:06}.tiff"));
        let mut encoder =
            tiff::encoder::TiffEncoder::new(std::fs::File::create(&file).unwrap()).unwrap();
        let mut image = encoder
            .new_image::<tiff::encoder::colortype::RGB8>(1, 1)
            .unwrap();
        image
            .encoder()
            .write_tag(tiff::tags::Tag::Unknown(34675), profile.as_slice())
            .unwrap();
        image.write_data(&[0, 0, 0]).unwrap();
        files.push(file);
    }
    app.repo
        .complete_with_artifacts(
            mixed_job.clone(),
            &jrip_server::artifacts::hash_pages(&files).await.unwrap(),
        )
        .await
        .unwrap();
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::get(format!("/api/v1/jobs/{}/color-input-profile", mixed_job.id))
                .header("authorization", "Bearer admin-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mixed: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(mixed["mixedProfiles"], true);
    assert!(mixed["candidateProfileSha256"].is_null());
    assert_eq!(mixed["status"], "EXPLICIT_SELECTION_REQUIRED");
    // Tampering cannot turn an untrusted new profile into evidence about the original artifact.
    std::fs::write(
        app.root
            .join("jobs")
            .join(job.id.to_string())
            .join("raster/page-000001.tiff"),
        b"corrupt",
    )
    .unwrap();
    assert_ne!(
        jrip_server::router(app.clone())
            .oneshot(get("admin-token"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}
