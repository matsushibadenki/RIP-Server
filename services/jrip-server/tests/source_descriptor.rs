use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use rip_core::{Job, Manifest};
use serde_json::{Value, json};
use tower::ServiceExt;
const PDF: &[u8] = b"%PDF-1.4\nnot structurally validated\n";
async fn seed() -> (tempfile::TempDir, jrip_server::App, Job, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), "admin".into())
        .await
        .unwrap();
    app.credentials = [
        ("admin", vec!["admin"]),
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
    let manifest: Manifest =
        serde_json::from_value(json!({"name":"input.pdf","format":"application/pdf"})).unwrap();
    let job = rip_jobs::submit_bytes(&app.repo, &app.root, manifest, PDF)
        .await
        .unwrap();
    let path = app
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("document");
    (dir, app, job, path)
}
async fn get(
    app: &jrip_server::App,
    id: uuid::Uuid,
    descriptor: bool,
    token: Option<&str>,
    range: Option<&str>,
    if_range: Option<&str>,
) -> axum::response::Response {
    let url = format!(
        "/api/v1/jobs/{id}/source{}",
        if descriptor { "/descriptor" } else { "" }
    );
    let mut req = Request::get(url);
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    if let Some(range) = range {
        req = req.header("range", range);
    }
    if let Some(if_range) = if_range {
        req = req.header("if-range", if_range);
    }
    jrip_server::router(app.clone())
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap()
}
async fn value(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
#[tokio::test]
async fn source_identity_is_not_structural_validation_and_reads_are_scoped_durable_and_bounded() {
    let (dir, app, job, path) = seed().await;
    for descriptor in [true, false] {
        for (token, status) in [
            (None, StatusCode::UNAUTHORIZED),
            (Some("job"), StatusCode::FORBIDDEN),
            (Some("artifact"), StatusCode::FORBIDDEN),
        ] {
            assert_eq!(
                get(&app, job.id, descriptor, token, None, None)
                    .await
                    .status(),
                status
            );
        }
    }
    let response = get(&app, job.id, true, Some("admin"), None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    let descriptor = value(response).await;
    assert_eq!(descriptor["kind"], "SourceDocument");
    assert_eq!(descriptor["uri"], format!("jrip://jobs/{}/source", job.id));
    assert_eq!(descriptor["sha256"], job.source_sha256);
    assert_eq!(descriptor["bytes"], PDF.len());
    assert_eq!(descriptor["mediaType"], "application/pdf");
    assert_eq!(descriptor["integrityVerified"], true);
    assert_eq!(descriptor["structureVerified"], false);
    assert!(descriptor["pageCount"].is_null());
    assert!(descriptor["validation"].is_null());
    assert!(descriptor["colorSpace"].is_null());
    assert!(descriptor.to_string().len() < 4096);
    assert!(
        !descriptor
            .to_string()
            .contains(dir.path().to_str().unwrap())
    );
    let baseline = app.artifact_budget.available_permits();
    let response = get(&app, job.id, false, Some("admin"), Some("bytes=0-7"), None).await;
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()["content-type"], "application/pdf");
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"source.pdf\""
    );
    assert_eq!(
        response.headers()["etag"],
        format!("\"sha256:{}\"", job.source_sha256)
    );
    assert!(app.artifact_budget.available_permits() < baseline);
    // The returned bytes must come from the verified snapshot even after a replacement.
    std::fs::write(&path, b"tampered").unwrap();
    assert_eq!(
        &response.into_body().collect().await.unwrap().to_bytes()[..],
        &PDF[..8]
    );
    assert_eq!(app.artifact_budget.available_permits(), baseline);
    assert_eq!(
        get(&app, job.id, true, Some("admin"), None, None)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    std::fs::write(&path, PDF).unwrap();
    let mut changed = PDF.to_vec();
    let last = changed.len() - 2;
    changed[last] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    for descriptor in [true, false] {
        assert_eq!(
            get(&app, job.id, descriptor, Some("admin"), None, None)
                .await
                .status(),
            StatusCode::CONFLICT
        );
    }
    std::fs::write(&path, PDF).unwrap();
    let response = get(
        &app,
        job.id,
        false,
        Some("admin"),
        Some("bytes=0-7"),
        Some("\"stale\""),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &response.into_body().collect().await.unwrap().to_bytes()[..],
        PDF
    );
    assert_eq!(
        get(
            &app,
            job.id,
            false,
            Some("admin"),
            Some("bytes=9999-"),
            None
        )
        .await
        .status(),
        StatusCode::RANGE_NOT_SATISFIABLE
    );
    let held = app
        .artifact_budget
        .clone()
        .try_acquire_many_owned(baseline as u32)
        .unwrap();
    assert_eq!(
        get(&app, job.id, true, Some("admin"), None, None)
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    drop(held);
    let mut reopened = jrip_server::open(dir.path().into(), "admin".into())
        .await
        .unwrap();
    reopened.credentials = app.credentials.clone();
    assert_eq!(
        value(get(&reopened, job.id, true, Some("admin"), None, None).await).await,
        descriptor
    );
    assert_eq!(
        reopened.repo.get(job.id).await.unwrap().revision,
        job.revision
    );
    assert!(reopened.repo.source_validation(job.id).await.is_err());
}
#[tokio::test]
async fn source_descriptor_rejects_symlinks_empty_oversized_and_wrong_declared_format() {
    use sha2::{Digest, Sha256};
    let (_dir, app, job, path) = seed().await;
    #[cfg(unix)]
    {
        let target = path.with_file_name("other");
        std::fs::write(&target, PDF).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(target, &path).unwrap();
        assert_eq!(
            get(&app, job.id, true, Some("admin"), None, None)
                .await
                .status(),
            StatusCode::CONFLICT
        );
        std::fs::remove_file(&path).unwrap();
    }
    std::fs::write(&path, []).unwrap();
    assert_eq!(
        get(&app, job.id, true, Some("admin"), None, None)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(rip_jobs::MAX_DOCUMENT_BYTES + 1)
        .unwrap();
    assert_eq!(
        get(&app, job.id, true, Some("admin"), None, None)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    std::fs::remove_file(&path).unwrap();
    let missing = get(&app, job.id, true, Some("admin"), None, None).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(value(missing).await["error"]["code"], "source_unavailable");
    let ps = b"%!PS-Adobe-3.0\nshowpage\n";
    let legacy = Job {
        id: uuid::Uuid::new_v4(),
        source_sha256: format!("{:x}", Sha256::digest(ps)),
        ..job
    };
    app.repo.insert(&legacy).await.unwrap();
    let folder = app.root.join("jobs").join(legacy.id.to_string());
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("document"), ps).unwrap();
    assert_eq!(
        get(&app, legacy.id, true, Some("admin"), None, None)
            .await
            .status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn source_download_preserves_each_admitted_format_without_running_an_interpreter() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), "admin".into())
        .await
        .unwrap();
    for (mime, extension, document) in [
        ("application/pdf", "pdf", b"%PDF-1.4\n".as_slice()),
        (
            "application/postscript",
            "ps",
            b"%!PS-Adobe-3.0\nshowpage\n".as_slice(),
        ),
        (
            "application/eps",
            "eps",
            b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 72 72\nshowpage\n".as_slice(),
        ),
    ] {
        let manifest = serde_json::from_value(json!({"name":"uploaded","format":mime})).unwrap();
        let job = rip_jobs::submit_bytes(&app.repo, &app.root, manifest, document)
            .await
            .unwrap();
        let response = get(&app, job.id, false, Some("admin"), None, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], mime);
        assert_eq!(
            response.headers()["content-disposition"],
            format!("attachment; filename=\"source.{extension}\"")
        );
        assert_eq!(
            &response.into_body().collect().await.unwrap().to_bytes()[..],
            document
        );
        let descriptor = value(get(&app, job.id, true, Some("admin"), None, None).await).await;
        assert_eq!(descriptor["mediaType"], mime);
        assert_eq!(descriptor["boundedInterpretationVerified"], false);
        assert_eq!(
            app.repo.get(job.id).await.unwrap().state,
            rip_core::JobState::Queued
        );
    }
}
