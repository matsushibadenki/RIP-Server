//! Real HTTP + Chromium operator-console regression (no printer or Docker).
use rip_core::JobState;
#[tokio::test]
#[ignore = "requires npm ci --prefix tests/ui and Playwright Chromium"]
async fn operator_console_real_browser_flow() {
    let dir = tempfile::tempdir().unwrap();
    let token = "console-test-token-at-least-24-characters";
    let mut app = jrip_server::open(dir.path().into(), token.into())
        .await
        .unwrap();
    app.credentials = vec![
        jrip_server::Credential {
            token: token.into(),
            identity: "operator".into(),
            scopes: vec!["admin".into()],
            validity: None,
        },
        jrip_server::Credential {
            token: "console-readonly-test-token".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into(), "artifacts:read".into()],
            validity: None,
        },
    ];
    let manifest=serde_json::from_value(serde_json::json!({"name":"ready-proof.pdf","format":"application/pdf","color_mode":"rgb","dpi":300})).unwrap();
    let job = rip_jobs::submit_bytes(&app.repo, &app.root, manifest, b"%PDF-1.4\n")
        .await
        .unwrap();
    let directory = app
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("raster");
    std::fs::create_dir(&directory).unwrap();
    let mut paths = vec![];
    for page in 1..=2 {
        let path = directory.join(format!("page-{page:06}.tiff"));
        let mut pixels = vec![255u8; 240 * 300 * 3];
        for y in 30..270 {
            for x in 22..218 {
                let color = if page == 1 {
                    if x < 120 {
                        [245, 25, 30]
                    } else {
                        [10, 65, 245]
                    }
                } else {
                    [20, 150, 80]
                };
                if x != 119 && x != 120 {
                    pixels[(y * 240 + x) * 3..(y * 240 + x) * 3 + 3].copy_from_slice(&color);
                }
            }
        }
        tiff::encoder::TiffEncoder::new(std::fs::File::create(&path).unwrap())
            .unwrap()
            .write_image::<tiff::encoder::colortype::RGB8>(240, 300, &pixels)
            .unwrap();
        paths.push(path);
    }
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
            &jrip_server::artifacts::hash_pages(&paths).await.unwrap(),
        )
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, jrip_server::router(app))
            .await
            .unwrap()
    });
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = tokio::process::Command::new("node")
        .arg("tests/ui/console.mjs")
        .current_dir(workspace)
        .env("JRIP_UI_URL", format!("http://{address}/console"))
        .env("JRIP_UI_TOKEN", token)
        .status()
        .await
        .unwrap();
    server.abort();
    assert!(status.success(), "Browser flow failed: {status}");
}
