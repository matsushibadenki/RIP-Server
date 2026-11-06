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
            validity: None,
            token: "admin-token".into(),
            identity: "admin".into(),
            scopes: vec!["admin".into()],
        },
        jrip_server::Credential {
            validity: None,
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
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::get(format!(
                "/api/v1/jobs/{}/color-attempts?state=COMPLETED",
                job.id
            ))
            .header("authorization", "Bearer admin-token")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.len() < 4096);
    let summaries: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(summaries["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(summaries["attempts"][0]["id"], id.to_string());
    assert_eq!(summaries["attempts"][0]["state"], "COMPLETED");
    assert_eq!(summaries["attempts"][0]["outputProfileSha256"], digest);
    assert_eq!(summaries["attempts"][0]["completedPages"], 2);
    assert!(summaries["attempts"][0].get("result").is_none());
    assert_eq!(attempt["completedPages"], 2);
    assert_eq!(attempt["result"]["pageCount"], 2);
    assert_eq!(attempt["result"]["gamutSummary"]["coverageComplete"], true);
    assert_eq!(attempt["result"]["gamutSummary"]["outOfGamutPixels"], 0);
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
            validity: None,
            token: "reader".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
        },
        jrip_server::Credential {
            validity: None,
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
        validity: None,
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

fn narrow_rgb_profile() -> Vec<u8> {
    let white = lcms2::CIExyY {
        x: 0.3127,
        y: 0.329,
        Y: 1.0,
    };
    let primaries = lcms2::CIExyYTRIPLE {
        Red: lcms2::CIExyY {
            x: 0.5,
            y: 0.33,
            Y: 1.0,
        },
        Green: lcms2::CIExyY {
            x: 0.3,
            y: 0.5,
            Y: 1.0,
        },
        Blue: lcms2::CIExyY {
            x: 0.2,
            y: 0.17,
            Y: 1.0,
        },
    };
    let gamma = lcms2::ToneCurve::new(2.2);
    lcms2::Profile::new_rgb(&white, &primaries, &[&gamma, &gamma, &gamma])
        .unwrap()
        .icc()
        .unwrap()
}
#[tokio::test]
#[ignore = "requires Docker and rebuilt jrip-color:local with gamut diagnostics"]
async fn isolated_worker_preserves_gamut_warnings_counts_and_immutable_result() {
    let (_dir, app, job, input) = seed().await;
    let registry = rip_color::registry::Registry::open(app.root.join("color-profiles")).unwrap();
    let output = registry.register(&narrow_rgb_profile()).unwrap();
    let id = Uuid::new_v4();
    let mut req = ticket(&job, id, &input);
    req["outputProfileSha256"] = json!(output.sha256);
    assert_eq!(
        call(
            &app,
            &format!("/api/v1/jobs/{}/color-attempts", job.id),
            req.clone()
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let originals = app.repo.artifacts(job.id).await.unwrap();
    assert!(jrip_server::color::tick(&app).await.unwrap());
    let attempt = app.repo.color_attempt(id).await.unwrap();
    assert_eq!(attempt["state"], "COMPLETED", "{attempt}");
    let result = &attempt["result"];
    assert_eq!(result["gamutSummary"]["coverageComplete"], true);
    assert_eq!(result["gamutSummary"]["checkedPixels"], 4);
    assert_eq!(result["gamutSummary"]["outOfGamutPixels"], 4);
    assert_eq!(result["gamutSummary"]["outOfGamutPercentOfChecked"], 100.0);
    for page in result["reports"].as_array().unwrap() {
        assert_eq!(page["colorReport"]["gamut"]["outOfGamutPixels"], 2);
        assert!(
            page["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w["code"] == "OUT_OF_GAMUT")
        );
        assert_eq!(page["colorReport"]["gamut"]["modifiesPixels"], false);
        for lang in ["en", "ja", "zh-CN"] {
            assert!(page["warnings"][0]["messages"][lang].is_string());
        }
    }
    assert_eq!(app.repo.artifacts(job.id).await.unwrap(), originals);
    let reopened = jrip_server::open(app.root.clone(), "admin-token".into())
        .await
        .unwrap();
    assert_eq!(reopened.repo.color_attempt(id).await.unwrap(), attempt);
    assert_eq!(
        call(
            &reopened,
            &format!("/api/v1/jobs/{}/color-attempts", job.id),
            req
        )
        .await
        .1,
        attempt
    );
    let response = jrip_server::router(reopened)
        .oneshot(
            Request::get(format!("/api/v1/color/attempts/{id}"))
                .header("authorization", "Bearer admin-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(payload["result"], *result);
}
#[tokio::test]
#[ignore = "requires Docker and rebuilt jrip-color:local with gamut diagnostics"]
async fn device_link_worker_reports_unassessed_gamut_instead_of_zero() {
    let (_dir, app, mut job, _) = seed().await;
    job.id = Uuid::new_v4();
    job.state = JobState::Spooling;
    job.revision = 0;
    job.manifest.color_mode = rip_core::ColorMode::Cmyk;
    app.repo.insert(&job).await.unwrap();
    let root = app
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("raster");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("page-000001.tiff");
    let mut encoder =
        tiff::encoder::TiffEncoder::new(std::fs::File::create(&path).unwrap()).unwrap();
    encoder
        .write_image::<tiff::encoder::colortype::CMYK8>(1, 1, &[0, 0, 0, 200])
        .unwrap();
    drop(encoder);
    app.repo
        .complete_with_artifacts(
            job.clone(),
            &jrip_server::artifacts::hash_pages(&[path]).await.unwrap(),
        )
        .await
        .unwrap();
    let job = app.repo.get(job.id).await.unwrap();
    let link = lcms2::Profile::ink_limiting(lcms2::ColorSpaceSignature::CmykData, 300.0)
        .unwrap()
        .icc()
        .unwrap();
    let profile = rip_color::registry::Registry::open(app.root.join("color-profiles"))
        .unwrap()
        .register(&link)
        .unwrap();
    let id = Uuid::new_v4();
    let req = json!({"requestId":id,"expectedRevision":job.revision,"inputProfileSha256":profile.sha256,"outputProfileSha256":null,"policy":{"schemaVersion":1,"inputColor":"cmyk","outputColor":"cmyk","intent":"perceptual","blackPointCompensation":false,"blackPreservation":"none","tacPercent":240}});
    assert_eq!(
        call(
            &app,
            &format!("/api/v1/jobs/{}/color-attempts", job.id),
            req
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    assert!(jrip_server::color::tick(&app).await.unwrap());
    let attempt = app.repo.color_attempt(id).await.unwrap();
    assert_eq!(attempt["state"], "COMPLETED", "{attempt}");
    let summary = &attempt["result"]["gamutSummary"];
    assert_eq!(summary["coverageComplete"], false);
    assert_eq!(summary["unassessedPages"], 1);
    assert_eq!(summary["checkedPixels"], 0);
    assert!(summary["outOfGamutPixels"].is_null());
    assert!(summary["outOfGamutPercentOfChecked"].is_null());
    assert!(
        attempt["result"]["reports"][0]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "GAMUT_NOT_ASSESSED")
    );
}

async fn preview_get(
    app: &jrip_server::App,
    url: &str,
    token: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::get(url);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    jrip_server::router(app.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn color_stage_preview_pins_profile_authenticates_and_survives_restart() {
    use sha2::{Digest, Sha256};
    let (dir, mut app, job, input_digest) = seed().await;
    let input_profile = rip_color::registry::Registry::open(app.root.join("color-profiles"))
        .unwrap()
        .read(&input_digest)
        .unwrap();
    let originals = app.repo.artifacts(job.id).await.unwrap();
    app.credentials = vec![
        jrip_server::Credential {
            validity: None,
            token: "admin-token".into(),
            identity: "admin".into(),
            scopes: vec!["admin".into()],
        },
        jrip_server::Credential {
            validity: None,
            token: "reader".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
        },
    ];
    for mode in ["rgb", "cmyk", "missing", "mismatch", "space", "link"] {
        let id = Uuid::new_v4();
        let profile = if mode == "cmyk" {
            include_bytes!("../../../crates/rip-raster/tests/fixtures/synthetic-cmyk.icc").to_vec()
        } else {
            lcms2::Profile::new_srgb().icc().unwrap()
        };
        let digest = format!("{:x}", Sha256::digest(&profile));
        let expected = if mode == "link" {
            Value::Null
        } else if mode == "mismatch" {
            json!("a".repeat(64))
        } else {
            json!(digest)
        };
        let ticket = json!({"sourceArtifacts":originals,"outputProfileSha256":expected,"policy":{"outputColor":if mode == "cmyk" || mode == "space" {"cmyk"} else {"rgb"}}});
        app.repo
            .create_color(id, job.id, job.revision, ticket)
            .await
            .unwrap();
        let url = format!("/api/v1/color/attempts/{id}/artifacts/page-000001.tiff/preview");
        assert_eq!(
            preview_get(&app, &url, None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            preview_get(&app, &url, Some("reader")).await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            preview_get(&app, &url, Some("admin-token")).await.status(),
            StatusCode::CONFLICT
        );
        let directory = app
            .root
            .join("jobs")
            .join(job.id.to_string())
            .join("color")
            .join(id.to_string());
        std::fs::create_dir_all(&directory).unwrap();
        let mut paths = Vec::new();
        for page in 1..=2 {
            let path = directory.join(format!("page-{page:06}.tiff"));
            if mode == "rgb" {
                let source = app
                    .root
                    .join("jobs")
                    .join(job.id.to_string())
                    .join("raster")
                    .join(format!("page-{page:06}.tiff"));
                let policy: rip_color::Policy = serde_json::from_value(json!({"schemaVersion":1,"inputColor":"rgb","outputColor":"rgb","intent":"relative_colorimetric","blackPointCompensation":false,"blackPreservation":"none","tacPercent":null})).unwrap();
                rip_color::raster::convert_tiff(
                    std::fs::File::open(source).unwrap(),
                    std::fs::File::create(&path).unwrap(),
                    &input_profile,
                    Some(&profile),
                    &policy,
                    300,
                )
                .unwrap();
                paths.push(path);
                continue;
            }
            let mut encoder =
                tiff::encoder::TiffEncoder::new(std::fs::File::create(&path).unwrap()).unwrap();
            if mode == "cmyk" {
                let mut image = encoder
                    .new_image::<tiff::encoder::colortype::CMYK8>(2, 1)
                    .unwrap();
                image
                    .encoder()
                    .write_tag(tiff::tags::Tag::Unknown(34675), profile.as_slice())
                    .unwrap();
                image.write_data(&[0, 0, 0, 0, 0, 0, 0, 255]).unwrap();
            } else {
                let mut image = encoder
                    .new_image::<tiff::encoder::colortype::RGB8>(2, 1)
                    .unwrap();
                if mode != "missing" {
                    image
                        .encoder()
                        .write_tag(tiff::tags::Tag::Unknown(34675), profile.as_slice())
                        .unwrap();
                }
                image.write_data(&[255, 0, 0, 0, 0, 255]).unwrap();
            }
            paths.push(path);
        }
        let artifacts = jrip_server::artifacts::hash_pages(&paths).await.unwrap();
        app.repo
            .update_color(id, 0, "RUNNING", 0, None, None)
            .await
            .unwrap();
        app.repo
            .update_color(id, 1, "RUNNING", 1, None, None)
            .await
            .unwrap();
        app.repo
            .update_color(id, 2, "RUNNING", 2, None, None)
            .await
            .unwrap();
        app.repo
            .update_color(
                id,
                3,
                "COMPLETED",
                2,
                Some(json!({"artifacts":artifacts})),
                None,
            )
            .await
            .unwrap();
        let response = preview_get(&app, &url, Some("admin-token")).await;
        if mode != "rgb" && mode != "cmyk" {
            assert_eq!(
                response.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{mode}"
            );
            let value: Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert!(value.to_string().contains("preview_unsupported"));
            continue;
        }
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "image/bmp");
        assert_eq!(
            response.headers()["x-jrip-source-sha256"],
            artifacts[0].sha256
        );
        assert_eq!(response.headers()["x-jrip-input-profile-sha256"], digest);
        assert_eq!(
            response.headers()["x-jrip-color-attempt-id"],
            id.to_string()
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        if mode == "rgb" {
            assert_eq!(&bytes[54..60], &[0, 0, 255, 255, 0, 0]);
        } else {
            assert!(bytes[54..57].iter().all(|v| *v > 245));
            assert!(bytes[57..60].iter().all(|v| *v < 10));
        }
        let reopened = jrip_server::open(dir.path().into(), "admin-token".into())
            .await
            .unwrap();
        let repeated = preview_get(&reopened, &url, Some("admin-token")).await;
        assert_eq!(repeated.status(), StatusCode::OK);
        assert_eq!(
            repeated.into_body().collect().await.unwrap().to_bytes(),
            bytes
        );
        let missing = url.replace("page-000001", "page-000003");
        assert_eq!(
            preview_get(&app, &missing, Some("admin-token"))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        std::fs::write(&paths[0], b"tampered").unwrap();
        assert_eq!(
            preview_get(&app, &url, Some("admin-token")).await.status(),
            StatusCode::CONFLICT
        );
    }
    assert_eq!(app.repo.artifacts(job.id).await.unwrap(), originals);
}

#[tokio::test]
async fn color_attempt_summaries_are_scoped_bounded_cursor_stable_and_durable() {
    let (dir, mut app, job, digest) = seed().await;
    let path = format!("/api/v1/jobs/{}/color-attempts", job.id);
    let ids = [Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)];
    for id in ids {
        assert_eq!(
            call(&app, &path, ticket(&job, id, &digest)).await.0,
            StatusCode::ACCEPTED
        );
    }
    app.repo
        .update_color(ids[1], 0, "CANCELLED", 0, None, None)
        .await
        .unwrap();
    let mut other = job.clone();
    other.id = Uuid::new_v4();
    app.repo.insert(&other).await.unwrap();
    app.repo
        .create_color(Uuid::new_v4(), other.id, other.revision, json!({"dpi":300}))
        .await
        .unwrap();
    app.credentials = vec![
        jrip_server::Credential {
            token: "summary-reader-token".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
            validity: None,
        },
        jrip_server::Credential {
            token: "summary-submit-token".into(),
            identity: "submitter".into(),
            scopes: vec!["jobs:submit".into()],
            validity: None,
        },
    ];
    async fn get(app: &jrip_server::App, path: &str, token: Option<&str>) -> (StatusCode, Value) {
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
            assert_eq!(response.headers()["vary"], "Authorization");
        }
        (
            status,
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap(),
        )
    }
    assert_eq!(get(&app, &path, None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        get(&app, &path, Some("summary-submit-token")).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, first) = get(
        &app,
        &format!("{path}?limit=1"),
        Some("summary-reader-token"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["attempts"][0]["id"], ids[2].to_string());
    assert_eq!(first["hasMore"], true);
    let summary = &first["attempts"][0];
    assert_eq!(summary["pageCount"], 2);
    assert_eq!(summary["inputProfileSha256"], digest);
    assert_eq!(summary["outputProfileSha256"], digest);
    assert_eq!(summary["outputColor"], "rgb");
    for key in [
        "ticket",
        "result",
        "sourceArtifacts",
        "artifacts",
        "reports",
    ] {
        assert!(summary.get(key).is_none());
    }
    // A newer insertion cannot shift or repeat already-consumed rows.
    app.repo
        .create_color(Uuid::from_u128(4), job.id, job.revision, json!({"dpi":300}))
        .await
        .unwrap();
    let cursor = first["nextBefore"].as_str().unwrap();
    let (_, rest) = get(
        &app,
        &format!("{path}?limit=100&before={cursor}"),
        Some("summary-reader-token"),
    )
    .await;
    assert_eq!(
        rest["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![ids[1].to_string(), ids[0].to_string()]
    );
    assert_eq!(rest["hasMore"], false);
    assert!(rest["nextBefore"].is_null());
    let (_, cancelled) = get(
        &app,
        &format!("{path}?state=CANCELLED"),
        Some("summary-reader-token"),
    )
    .await;
    assert_eq!(cancelled["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(cancelled["attempts"][0]["id"], ids[1].to_string());
    let (_, complete) = get(
        &app,
        &format!("{path}?state=COMPLETED"),
        Some("summary-reader-token"),
    )
    .await;
    assert_eq!(complete["attempts"], json!([]));
    for query in [
        "limit=0",
        "limit=101",
        "limit=-1",
        "limit=abc",
        "offset=1",
        "before=invalid",
        "before=-1:00000000-0000-0000-0000-000000000001",
        "before=9223372036854775808:00000000-0000-0000-0000-000000000001",
        "state=completed",
        "state=UNKNOWN",
    ] {
        let (status, value) = get(
            &app,
            &format!("{path}?{query}"),
            Some("summary-reader-token"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {value}");
        assert_eq!(value["error"]["code"], "invalid_color_query");
    }
    let unknown = format!("/api/v1/jobs/{}/color-attempts", Uuid::new_v4());
    assert_eq!(
        get(&app, &unknown, Some("summary-reader-token")).await.0,
        StatusCode::NOT_FOUND
    );
    let mut reopened = jrip_server::open(dir.path().into(), "admin-token".into())
        .await
        .unwrap();
    reopened.credentials = app.credentials.clone();
    assert_eq!(
        get(
            &reopened,
            &format!("{path}?state=CANCELLED"),
            Some("summary-reader-token")
        )
        .await
        .1,
        cancelled
    );
}
