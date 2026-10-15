//! Independent CUPS client -> real HTTP IPP -> dispatcher -> Docker -> registered TIFF.
use rip_core::JobState;
use std::time::Duration;
async fn ipptool(uri: &str, file: &std::path::Path, test: &std::path::Path) {
    let output = tokio::process::Command::new("ipptool")
        .args(["-tv", "-f"])
        .arg(file)
        .arg(uri)
        .arg(test)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[tokio::test]
#[ignore = "requires ipptool, Docker and jrip-mupdf:local"]
async fn independent_ipp_pdf_to_tiff_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let uri = format!("ipp://{}/ipp/print", listener.local_addr().unwrap());
    let app = jrip_ipp::open(dir.path().into(), uri.clone())
        .await
        .unwrap();
    let repo = app.repo.clone();
    let server =
        tokio::spawn(async move { axum::serve(listener, jrip_ipp::router(app)).await.unwrap() });
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    for pages in [1, 2] {
        let pdf = dir.path().join(format!("{pages}.pdf"));
        if pages == 2 {
            std::fs::copy(corpus.join("colors.pdf"), &pdf).unwrap();
        } else {
            // Generate a valid one-page PDF including accurate xref offsets.
            let stream = "1 0 0 rg 0 0 20 20 re f\n";
            let content = format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len());
            let objects = [
                "<< /Type /Catalog /Pages 2 0 R >>",
                "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Resources << >> /Contents 4 0 R >>",
                content.as_str(),
            ];
            let mut bytes = b"%PDF-1.4\n".to_vec();
            let mut offsets = vec![0];
            for (i, obj) in objects.iter().enumerate() {
                offsets.push(bytes.len());
                bytes.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
            }
            let xref = bytes.len();
            bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
            for offset in &offsets[1..] {
                bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
            }
            bytes.extend_from_slice(
                format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n")
                    .as_bytes(),
            );
            std::fs::write(&pdf, bytes).unwrap();
        }
        ipptool(&uri, &pdf, &corpus.join("ipp-pdf-proof.test")).await;
        let output = tokio::time::timeout(
            Duration::from_secs(90),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_jrip-dispatcher"))
                .arg("--once")
                .env("JRIP_DATA_ROOT", dir.path())
                .env("JRIP_WORKER_BIN", env!("CARGO_BIN_EXE_jrip-worker"))
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let jobs = repo.list_ipp_jobs().await.unwrap();
        let (ipp_id, job) = jobs.iter().max_by_key(|(id, _)| id).unwrap();
        assert_eq!(
            job.state,
            JobState::Completed,
            "{job:?} worker stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let validation = repo.source_validation(job.id).await.unwrap();
        assert_eq!(validation["allPagesInterpreted"], true);
        assert_eq!(validation["strictPdfInspection"], true);
        assert_eq!(validation["sourceSha256"], job.source_sha256);
        let artifacts: Vec<rip_core::Artifact> =
            serde_json::from_value(repo.artifacts(job.id).await.unwrap()).unwrap();
        assert_eq!(artifacts.len(), pages);
        for a in &artifacts {
            let file = std::fs::File::open(
                dir.path()
                    .join("jobs")
                    .join(job.id.to_string())
                    .join("raster")
                    .join(&a.name),
            )
            .unwrap();
            let mut decoder = tiff::decoder::Decoder::new(file).unwrap();
            assert_eq!(decoder.dimensions().unwrap(), (167, 167));
            decoder.read_image().unwrap();
        }
        let restarted = jrip_ipp::open(dir.path().into(), uri.clone())
            .await
            .unwrap();
        assert_eq!(
            restarted.repo.get_ipp_job(*ipp_id).await.unwrap().id,
            job.id
        );
        let check = dir.path().join("complete.test");
        std::fs::write(&check,format!("{{\n NAME \"Completed TIFF export\"\n VERSION 1.1\n OPERATION Get-Job-Attributes\n GROUP operation\n ATTR charset attributes-charset utf-8\n ATTR language attributes-natural-language en\n ATTR uri printer-uri $uri\n ATTR integer job-id {ipp_id}\n STATUS successful-ok\n EXPECT job-state WITH-VALUE 9\n EXPECT job-state-message WITH-VALUE \"TIFF file export completed; no paper output\"\n}}\n")).unwrap();
        ipptool(&uri, &pdf, &check).await;
    }
    server.abort();
}

#[tokio::test]
#[ignore = "requires Docker and jrip-mupdf:local"]
async fn abandoned_container_cleanup_is_source_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let job_id = uuid::Uuid::new_v4();
    let directory = dir.path().join("jobs").join(job_id.to_string());
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("document");
    std::fs::write(&source, b"%PDF-1.4").unwrap();
    let source = std::fs::canonicalize(source).unwrap();
    let create = |path: std::path::PathBuf| async move {
        let output = tokio::process::Command::new("docker")
            .args([
                "create",
                "--network=none",
                "--read-only",
                "--label=org.jrip.interpreter=1",
            ])
            .arg(format!("--label=org.jrip.source={}", path.display()))
            .args(["--entrypoint", "/bin/true", "jrip-mupdf:local"])
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    let target = create(source.clone()).await;
    let snapshot = directory.join("source-validation");
    std::fs::create_dir(&snapshot).unwrap();
    let nested = snapshot.join("document");
    std::fs::write(&nested, b"%PDF-1.4").unwrap();
    let nested_container = create(std::fs::canonicalize(nested).unwrap()).await;

    let unrelated = create(dir.path().join("different-source")).await;
    let repo = rip_storage::Repository::open(&dir.path().join("jobs.sqlite3"))
        .await
        .unwrap();
    let job:rip_core::Job=serde_json::from_value(serde_json::json!({"id":job_id,"manifest":{"name":"orphan.pdf","format":"application/pdf","dpi":72},"state":"FAILED","source_sha256":"f".repeat(64),"created_at":0,"updated_at":0,"revision":0,"error_code":"container_cleanup_failed","selected_engine":"mupdf","worker_run_id":null})).unwrap();
    repo.insert(&job).await.unwrap();
    let recovered = tokio::process::Command::new(env!("CARGO_BIN_EXE_jrip-dispatcher"))
        .arg("--once")
        .env("JRIP_DATA_ROOT", dir.path())
        .output()
        .await
        .unwrap();
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    let missing = tokio::process::Command::new("docker")
        .args(["inspect", &target])
        .output()
        .await
        .unwrap();
    let nested_missing = tokio::process::Command::new("docker")
        .args(["inspect", &nested_container])
        .output()
        .await
        .unwrap();
    assert!(!nested_missing.status.success());
    assert!(!snapshot.exists());
    let kept = tokio::process::Command::new("docker")
        .args(["inspect", &unrelated])
        .output()
        .await
        .unwrap();
    let cleanup = tokio::process::Command::new("docker")
        .args(["rm", "-f", &unrelated])
        .output()
        .await
        .unwrap();
    assert!(cleanup.status.success());
    assert!(!missing.status.success());
    assert!(kept.status.success());
}
