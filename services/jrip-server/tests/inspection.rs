use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
async fn submit(app: &jrip_server::App, id: Uuid, document: &str) -> (StatusCode, Value) {
    let body = format!(
        "--inspection\r\nContent-Disposition: form-data; name=\"document\"\r\n\r\n{document}\r\n--inspection--\r\n"
    );
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::post(format!("/api/v1/print/document-inspection-requests/{id}"))
                .header("authorization", "Bearer token")
                .header("content-type", "multipart/form-data; boundary=inspection")
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
async fn queue_replay_conflicts_owner_and_invalid_sources_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    app.credentials = vec![jrip_server::Credential {
        token: "token".into(),
        identity: "owner".into(),
        scopes: vec!["jobs:submit".into()],
    }];
    let id = Uuid::new_v4();
    let (a, b) = tokio::join!(
        submit(&app, id, "%PDF-1.4\nsource"),
        submit(&app, id, "%PDF-1.4\nsource")
    );
    assert_eq!(a.0, StatusCode::ACCEPTED, "{a:?}");
    assert_eq!(b.0, StatusCode::ACCEPTED, "{b:?}");
    assert_eq!(a.1, b.1);
    assert_eq!(app.repo.inspection_work().await.unwrap().len(), 1);
    assert_eq!(
        std::fs::read_dir(app.root.join("inspection-queue"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        submit(&app, id, "%PDF-1.4\nchanged").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        submit(&app, Uuid::new_v4(), "%!PS-Adobe\n").await.0,
        StatusCode::BAD_REQUEST
    );
    app.credentials[0].identity = "other".into();
    assert!(app.repo.inspection_request(id, "other").await.is_err());
    assert_eq!(
        submit(&app, id, "%PDF-1.4\nsource").await.0,
        StatusCode::CONFLICT
    );
    app.credentials[0].scopes = vec!["jobs:read".into()];
    assert_eq!(
        submit(&app, Uuid::new_v4(), "%PDF-1.4\nsource").await.0,
        StatusCode::FORBIDDEN
    );
    assert!(app.repo.list(100, 0).await.unwrap().is_empty());
}
#[tokio::test]
async fn reports_and_terminal_state_commit_together_and_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    let id = Uuid::new_v4();
    let result = submit(&app, id, "%PDF-1.4\nsource").await;
    assert_eq!(result.0, StatusCode::ACCEPTED);
    app.repo
        .set_inspection_state(id, 0, "RUNNING", None, None)
        .await
        .unwrap();
    let mut report = json!({"inspectionId":id,"sourceSha256":result.1["sourceSha256"],"sourceBytes":result.1["sourceBytes"],"pages":[{"page":1,"widthPoints":72,"heightPoints":72}]});
    report["sourceSha256"] = json!("wrong");
    assert!(
        app.repo
            .set_inspection_state(id, 1, "COMPLETED", None, Some(report.clone()))
            .await
            .is_err()
    );
    assert!(
        app.repo
            .document_inspection(id, "legacy-local")
            .await
            .is_err()
    );
    report["sourceSha256"] = result.1["sourceSha256"].clone();
    app.repo
        .set_inspection_state(id, 1, "COMPLETED", None, Some(report.clone()))
        .await
        .unwrap();
    let reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    let history = reopened
        .repo
        .inspection_events(id, "legacy-local", 0, 100)
        .await
        .unwrap();
    let states: Vec<_> = history["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, vec!["QUEUED", "RUNNING", "COMPLETED"]);
    assert_eq!(history["events"][2]["reportAvailable"], true);
    let first = reopened
        .repo
        .inspection_events(id, "legacy-local", 0, 1)
        .await
        .unwrap();
    assert_eq!(first["hasMore"], true);
    assert_eq!(
        reopened
            .repo
            .inspection_events(
                id,
                "legacy-local",
                first["nextAfter"].as_i64().unwrap(),
                100
            )
            .await
            .unwrap()["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        reopened
            .repo
            .inspection_events(id, "other", 0, 100)
            .await
            .is_err()
    );
    assert_eq!(
        submit(&reopened, id, "%PDF-1.4\nsource").await.0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        reopened
            .repo
            .inspection_events(id, "legacy-local", 0, 100)
            .await
            .unwrap(),
        history
    );
    for (suffix, status) in [
        ("?after=-1", StatusCode::BAD_REQUEST),
        ("?limit=101", StatusCode::BAD_REQUEST),
        ("", StatusCode::OK),
    ] {
        let response = jrip_server::router(reopened.clone())
            .oneshot(
                Request::get(format!(
                    "/api/v1/print/document-inspection-requests/{id}/events{suffix}"
                ))
                .header("authorization", "Bearer token")
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    assert_eq!(
        reopened
            .repo
            .inspection_request(id, "legacy-local")
            .await
            .unwrap()["state"],
        "COMPLETED"
    );
    assert_eq!(
        reopened
            .repo
            .document_inspection(id, "legacy-local")
            .await
            .unwrap(),
        report
    );
    assert!(
        reopened
            .repo
            .set_inspection_state(id, 1, "FAILED", None, None)
            .await
            .is_err()
    );
}
#[tokio::test]
#[ignore = "requires Docker and rebuilt jrip-mupdf:local"]
async fn worker_finishes_replays_recovers_interruption_and_cleans_sources() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    let id = Uuid::new_v4();
    let pdf = include_str!("../../../tests/corpus/colors.pdf");
    assert_eq!(submit(&app, id, pdf).await.0, StatusCode::ACCEPTED);
    assert!(jrip_server::inspection::tick(&app).await.unwrap());
    assert_eq!(
        app.repo
            .inspection_request(id, "legacy-local")
            .await
            .unwrap()["state"],
        "COMPLETED"
    );
    assert_eq!(
        app.repo
            .document_inspection(id, "legacy-local")
            .await
            .unwrap()["pages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        std::fs::read_dir(app.root.join("inspection-queue"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(submit(&app, id, pdf).await.0, StatusCode::ACCEPTED);
    assert!(!jrip_server::inspection::tick(&app).await.unwrap());
    let interrupted = Uuid::new_v4();
    assert_eq!(submit(&app, interrupted, pdf).await.0, StatusCode::ACCEPTED);
    app.repo
        .set_inspection_state(interrupted, 0, "RUNNING", None, None)
        .await
        .unwrap();
    let reopened = jrip_server::open(app.root.clone(), "token".into())
        .await
        .unwrap();
    jrip_server::inspection::tick(&reopened).await.unwrap();
    assert_eq!(
        app.repo
            .inspection_request(interrupted, "legacy-local")
            .await
            .unwrap()["reasonCode"],
        "INSPECTION_WORKER_INTERRUPTED"
    );
    assert!(
        app.repo
            .document_inspection(interrupted, "legacy-local")
            .await
            .is_err()
    );
    let orphan = app
        .root
        .join("inspection-queue")
        .join(Uuid::new_v4().to_string());
    std::fs::create_dir(&orphan).unwrap();
    std::fs::write(orphan.join("document"), b"orphan").unwrap();
    jrip_server::inspection::tick(&reopened).await.unwrap();
    assert!(!orphan.exists());
    let cli = app.root.join("inspection-recovery-test");
    std::fs::create_dir(&cli).unwrap();
    std::fs::write(cli.join("inspection-attempt"), Uuid::new_v4().to_string()).unwrap();
    let active = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(cli.join("upload.lock"))
        .unwrap();
    active.lock().unwrap();
    jrip_server::inspection::tick(&app).await.unwrap();
    assert!(cli.exists());
    drop(active);
    jrip_server::inspection::tick(&app).await.unwrap();
    assert!(!cli.exists());
    let bad = Uuid::new_v4();
    assert_eq!(
        submit(&app, bad, "%PDF-1.4\ninvalid").await.0,
        StatusCode::ACCEPTED
    );
    jrip_server::inspection::tick(&app).await.unwrap();
    assert_eq!(
        app.repo
            .inspection_request(bad, "legacy-local")
            .await
            .unwrap()["state"],
        "FAILED"
    );
    assert!(
        app.repo
            .document_inspection(bad, "legacy-local")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn queued_inspection_capacity_is_enforced_transactionally() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    for _ in 0..8 {
        assert!(
            app.repo
                .enqueue_inspection(
                    Uuid::new_v4(),
                    "owner",
                    &"a".repeat(64),
                    32 * 1024 * 1024,
                    Uuid::new_v4()
                )
                .await
                .unwrap()
        );
    }
    assert!(
        app.repo
            .enqueue_inspection(Uuid::new_v4(), "owner", &"a".repeat(64), 1, Uuid::new_v4())
            .await
            .is_err()
    );
    assert_eq!(app.repo.inspection_work().await.unwrap().len(), 8);
}
