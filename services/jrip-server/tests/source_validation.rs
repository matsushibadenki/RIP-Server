use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use rip_core::{JobState, Manifest};
use tower::ServiceExt;

#[tokio::test]
#[ignore = "requires Docker and jrip-mupdf:local/jrip-ghostscript:local images"]
async fn default_worker_validates_all_supported_formats_and_rejects_late_errors() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let app = jrip_server::open(root.clone(), "token".into())
        .await
        .unwrap();
    let pdf = include_bytes!("../../../tests/corpus/colors.pdf").as_slice();
    let ps = b"%!PS-Adobe-3.0\nshowpage\n".as_slice();
    let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 72 72\nshowpage\n".as_slice();
    let bad_ps = b"%!PS-Adobe-3.0\nshowpage\nTHIS_OPERATOR_DOES_NOT_EXIST\n".as_slice();
    let repaired = &pdf[..pdf.len() - 30];
    let bad_eps=b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 72 72\nshowpage\nTHIS_OPERATOR_DOES_NOT_EXIST\n".as_slice();
    let cases = [
        ("application/pdf", pdf, true),
        ("application/postscript", ps, true),
        ("application/eps", eps, true),
        ("application/pdf", b"%PDF-1.4\nbroken\n".as_slice(), false),
        ("application/postscript", bad_ps, false),
        ("application/eps", bad_eps, false),
        ("application/pdf", repaired, false),
    ];
    for (format, data, valid) in cases {
        let manifest: Manifest =
            serde_json::from_value(serde_json::json!({"name":"source","format":format,"dpi":72}))
                .unwrap();
        let committed = format == "application/postscript" && valid;
        let job = if committed {
            use sha2::{Digest, Sha256};
            let intent = serde_json::json!({"document":{"uri":"https://example.org/proof.ps","mediaType":format,"sha256":format!("{:x}",Sha256::digest(data))},"dpi":{"mode":"required","value":72},"copies":{"mode":"required","value":1},"output":{"destination":"artifact"}});
            let response = jrip_server::router(app.clone())
                .oneshot(
                    Request::post("/api/v1/print/preflights")
                        .header("authorization", "Bearer token")
                        .header("content-type", "application/json")
                        .body(Body::from(intent.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
            let pf: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            let body = format!(
                "--bound\r\nContent-Disposition: form-data; name=\"document\"\r\n\r\n{}\r\n--bound--\r\n",
                std::str::from_utf8(data).unwrap()
            );
            let response = jrip_server::router(app.clone())
                .oneshot(
                    Request::post(format!(
                        "/api/v1/print/preflights/{}/commit",
                        pf["preflightId"].as_str().unwrap()
                    ))
                    .header("authorization", "Bearer token")
                    .header("content-type", "multipart/form-data; boundary=bound")
                    .header(
                        "x-jrip-preflight-digest",
                        pf["preflightDigest"].as_str().unwrap(),
                    )
                    .body(Body::from(body))
                    .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
            let result: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            serde_json::from_value(result["job"].clone()).unwrap()
        } else {
            rip_jobs::submit_bytes(&app.repo, &root, manifest, data)
                .await
                .unwrap()
        };
        let status = tokio::process::Command::new(env!("CARGO_BIN_EXE_jrip-worker"))
            .arg(job.id.to_string())
            .env("JRIP_DATA_ROOT", &root)
            .output()
            .await
            .unwrap();
        let current = app.repo.get(job.id).await.unwrap();
        assert_eq!(
            status.status.success(),
            valid,
            "format={format} stderr={} state={current:?}",
            String::from_utf8_lossy(&status.stderr)
        );
        assert_eq!(
            current.state,
            if valid {
                JobState::Completed
            } else {
                JobState::Failed
            }
        );
        let response = jrip_server::router(app.clone())
            .oneshot(
                Request::get(format!("/api/v1/jobs/{}/source-validation", job.id))
                    .header("authorization", "Bearer token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if valid {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            }
        );
        if valid {
            let report: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(report["sourceSha256"], job.source_sha256);
            assert_eq!(
                report["pageCount"],
                if format == "application/pdf" { 2 } else { 1 }
            );
            assert_eq!(report["strictPdfInspection"], format == "application/pdf");
            if committed {
                let result = app.repo.get_result_receipt(job.id).await.unwrap();
                assert_eq!(result["proof"]["result"]["sourceValidation"], report);
                let admission = app.repo.get_print_receipt(job.id).await.unwrap();
                assert_eq!(admission["status"], "accepted");
            }
        } else {
            assert!(
                !root
                    .join("jobs")
                    .join(job.id.to_string())
                    .join("raster")
                    .exists()
            );
        }
        assert!(
            !root
                .join("jobs")
                .join(job.id.to_string())
                .join("source-validation")
                .exists()
        );
    }
    // Explicit PDF Ghostscript still requires the same strict PDF gate.
    let manifest:Manifest=serde_json::from_value(serde_json::json!({"name":"explicit.pdf","format":"application/pdf","dpi":72,"engine":"ghostscript"})).unwrap();
    let job = rip_jobs::submit_bytes(&app.repo, &root, manifest, pdf)
        .await
        .unwrap();
    assert!(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_jrip-worker"))
            .arg(job.id.to_string())
            .env("JRIP_DATA_ROOT", &root)
            .output()
            .await
            .unwrap()
            .status
            .success()
    );
    assert_eq!(
        app.repo.source_validation(job.id).await.unwrap()["strictPdfInspection"],
        true
    );
    let manifest: Manifest = serde_json::from_value(
        serde_json::json!({"name":"changed.ps","format":"application/postscript","dpi":72}),
    )
    .unwrap();
    let job = rip_jobs::submit_bytes(&app.repo, &root, manifest, ps)
        .await
        .unwrap();
    std::fs::write(
        root.join("jobs").join(job.id.to_string()).join("document"),
        b"%!PS-Adobe-3.0\nshowpage showpage\n",
    )
    .unwrap();
    assert!(
        !tokio::process::Command::new(env!("CARGO_BIN_EXE_jrip-worker"))
            .arg(job.id.to_string())
            .env("JRIP_DATA_ROOT", &root)
            .output()
            .await
            .unwrap()
            .status
            .success()
    );
    assert_eq!(
        app.repo.get(job.id).await.unwrap().error_code.as_deref(),
        Some("source_integrity_mismatch")
    );
    assert!(app.repo.source_validation(job.id).await.is_err());
}

#[tokio::test]
async fn validation_evidence_is_read_only_and_scope_checked() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    app.credentials = vec![
        jrip_server::Credential {
            token: "reader".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
        },
        jrip_server::Credential {
            token: "submitter".into(),
            identity: "submitter".into(),
            scopes: vec!["jobs:submit".into()],
        },
    ];
    let router = jrip_server::router(app);
    let url = format!("/api/v1/jobs/{}/source-validation", uuid::Uuid::new_v4());
    for (token, status) in [
        ("reader", StatusCode::NOT_FOUND),
        ("submitter", StatusCode::FORBIDDEN),
        ("unknown", StatusCode::UNAUTHORIZED),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::get(&url)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    assert_eq!(
        router
            .oneshot(
                Request::post(url)
                    .header("authorization", "Bearer reader")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}
