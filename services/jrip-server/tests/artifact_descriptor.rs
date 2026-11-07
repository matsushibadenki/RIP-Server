use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use rip_core::{Job, JobState};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
async fn seed() -> (
    tempfile::TempDir,
    jrip_server::App,
    Job,
    std::path::PathBuf,
    String,
) {
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
    let manifest=serde_json::from_value(json!({"name":"source.pdf","format":"application/pdf","dpi":300,"color_mode":"rgb","page_ranges":[{"first":7,"last":7}]})).unwrap();
    let job = Job {
        id: uuid::Uuid::new_v4(),
        manifest,
        state: JobState::Spooling,
        source_sha256: "a".repeat(64),
        created_at: 0,
        updated_at: 0,
        revision: 0,
        error_code: None,
        selected_engine: Some(rip_core::EngineKind::MuPdf),
        worker_run_id: None,
    };
    app.repo.insert(&job).await.unwrap();
    let directory = app
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("raster");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("page-000001.tiff");
    let profile = lcms2::Profile::new_srgb().icc().unwrap();
    {
        let mut encoder =
            tiff::encoder::TiffEncoder::new(std::fs::File::create(&path).unwrap()).unwrap();
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
    }
    let mut artifacts = jrip_server::artifacts::hash_pages(std::slice::from_ref(&path))
        .await
        .unwrap();
    artifacts[0].source_page = Some(7);
    artifacts[0].source_page_count = Some(10);
    let job = app
        .repo
        .complete_with_artifacts(job, &artifacts)
        .await
        .unwrap();
    (
        dir,
        app,
        job,
        path,
        format!("{:x}", Sha256::digest(profile)),
    )
}
async fn get(
    app: &jrip_server::App,
    job: &Job,
    name: &str,
    token: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::get(format!(
        "/api/v1/jobs/{}/artifacts/{name}/descriptor",
        job.id
    ));
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    jrip_server::router(app.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}
async fn value(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
#[tokio::test]
async fn descriptor_verifies_samples_mapping_profile_provenance_and_retention_without_payloads() {
    let (dir, app, job, path, icc) = seed().await;
    for (token, status) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some("job"), StatusCode::FORBIDDEN),
        (Some("artifact"), StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            get(&app, &job, "page-000001.tiff", token).await.status(),
            status
        );
    }
    let response = get(&app, &job, "page-000001.tiff", Some("admin")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    let descriptor = value(response).await;
    assert_eq!(descriptor["kind"], "RasterArtifact");
    assert_eq!(
        descriptor["sha256"],
        format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()))
    );
    assert_eq!(
        descriptor["contentId"],
        format!("sha256:{}", descriptor["sha256"].as_str().unwrap())
    );
    assert_eq!(descriptor["bytes"], std::fs::metadata(&path).unwrap().len());
    assert_eq!(descriptor["raster"]["width"], 2);
    assert_eq!(descriptor["raster"]["height"], 1);
    assert_eq!(descriptor["raster"]["colorSpace"], "rgb");
    assert_eq!(descriptor["raster"]["resolution"]["x"], json!([300, 1]));
    assert_eq!(descriptor["raster"]["profileSha256"], icc);
    assert_eq!(descriptor["raster"]["profileState"], "VALIDATED_ENVELOPE");
    assert_eq!(descriptor["raster"]["samplesVerified"], true);
    assert_eq!(descriptor["sourcePage"], 7);
    assert_eq!(descriptor["sourcePageCount"], 10);
    assert_eq!(descriptor["pageCount"], 1);
    assert_eq!(descriptor["provenance"]["engine"], "mupdf");
    assert!(descriptor["retention"]["retainUntil"].is_null());
    assert!(
        !descriptor
            .to_string()
            .contains(dir.path().to_str().unwrap())
    );
    assert!(descriptor.to_string().len() < 4096);
    let until = rip_storage::now() + 3600;
    app.repo
        .extend_artifact_retention(job.id, 0, until, "admin", "proof retention")
        .await
        .unwrap();
    let mut reopened = jrip_server::open(dir.path().into(), "admin".into())
        .await
        .unwrap();
    reopened.credentials = app.credentials.clone();
    let held = value(get(&reopened, &job, "page-000001.tiff", Some("admin")).await).await;
    assert_eq!(held["contentId"], descriptor["contentId"]);
    assert_eq!(held["retention"]["retainUntil"], until);
    assert_eq!(held["retention"]["revision"], 1);
    std::fs::write(&path, b"tampered").unwrap();
    assert_eq!(
        get(&app, &job, "page-000001.tiff", Some("admin"))
            .await
            .status(),
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn descriptor_rejects_registered_non_tiff_and_symlink_replacements() {
    let (_dir, app, job, path, _) = seed().await;
    let bytes = std::fs::read(&path).unwrap();
    #[cfg(unix)]
    {
        let other = path.with_file_name("other.tiff");
        std::fs::write(&other, &bytes).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&other, &path).unwrap();
        assert_eq!(
            get(&app, &job, "page-000001.tiff", Some("admin"))
                .await
                .status(),
            StatusCode::CONFLICT
        );
        std::fs::remove_file(&path).unwrap();
    }
    // A byte-hash match is insufficient to describe unsupported raster samples.
    std::fs::write(&path, b"not a TIFF").unwrap();
    let artifacts = jrip_server::artifacts::hash_pages(std::slice::from_ref(&path))
        .await
        .unwrap();
    let another = Job {
        id: uuid::Uuid::new_v4(),
        state: JobState::Spooling,
        manifest: serde_json::from_value(json!({"name":"bad.pdf","format":"application/pdf"}))
            .unwrap(),
        ..job.clone()
    };
    app.repo.insert(&another).await.unwrap();
    let directory = app
        .root
        .join("jobs")
        .join(another.id.to_string())
        .join("raster");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("page-000001.tiff"), b"not a TIFF").unwrap();
    let another = app
        .repo
        .complete_with_artifacts(another, &artifacts)
        .await
        .unwrap();
    assert_eq!(
        get(&app, &another, "page-000001.tiff", Some("admin"))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
