use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;
const TOKEN: &str = "test-token-at-least-24-characters";
async fn app() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    let state = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    (dir, jrip_server::router(state))
}

#[tokio::test]
async fn acx_discovery_describes_the_real_rip_boundary() {
    let (_dir, app) = app().await;
    let response = app
        .oneshot(
            Request::get("/.well-known/acx.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let manifest: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let capability = &manifest["capabilities"][0];
    assert_eq!(manifest["acx"], "0.1");
    assert_eq!(capability["id"], "org.jrip.rip.export_tiff");
    assert_eq!(
        capability["extensions"]["org.jrip.print"]["physicalOutput"],
        false
    );
    assert_eq!(capability["bindings"][0]["endpoint"], "/api/v1/jobs");
}
fn upload(name: &str, format: &str, document: &str) -> Request<Body> {
    let manifest = serde_json::json!({"name":name,"format":format,"dpi":300});
    let body = format!(
        "--testboundary\r\nContent-Disposition: form-data; name=\"manifest\"\r\n\r\n{manifest}\r\n--testboundary\r\nContent-Disposition: form-data; name=\"document\"; filename=\"../../evil.ps\"\r\nContent-Type: application/octet-stream\r\n\r\n{document}\r\n--testboundary--\r\n"
    );
    Request::post("/api/v1/jobs")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "multipart/form-data; boundary=testboundary")
        .body(Body::from(body))
        .unwrap()
}
fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap()
}
#[tokio::test]
async fn job_lifecycle_and_storage_boundary() {
    let (dir, app) = app().await;
    let res = app
        .clone()
        .oneshot(upload(
            "縦書き-日本語.ps",
            "application/postscript",
            "%!PS-Adobe-3.0\nshowpage\n",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let job: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(job["manifest"]["engine"], "auto");
    let id = job["id"].as_str().unwrap();
    let path = format!("/api/v1/jobs/{id}");
    assert!(dir.path().join("jobs").join(id).join("document").exists());
    assert!(!dir.path().join("evil.ps").exists());
    for action in ["hold", "release", "cancel"] {
        assert_eq!(
            app.clone()
                .oneshot(request("POST", &format!("{path}/{action}")))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        app.clone()
            .oneshot(request("POST", &format!("{path}/hold")))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.clone()
            .oneshot(request("DELETE", &path))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.oneshot(request("GET", &path)).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn invalid_documents_leave_no_spool_and_auth_is_required() {
    let (dir, app) = app().await;
    assert_eq!(
        app.clone()
            .oneshot(Request::get("/api/v1/jobs").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for (format, document) in [
        ("application/pdf", "%!PS-Adobe-3.0"),
        ("application/pdf", "not a PDF"),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(upload("invalid", format, document))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        std::fs::read_dir(dir.path().join("jobs")).unwrap().count(),
        0
    );
}

#[tokio::test]
#[ignore = "requires Docker, jrip-mupdf:local and jrip-ghostscript:local images"]
async fn real_worker_uses_hybrid_engine_and_exports_tiff() {
    for (format, document, expected_engine) in [
        (
            "application/postscript",
            include_str!("../../../tests/corpus/basic.ps"),
            "ghostscript",
        ),
        (
            "application/pdf",
            include_str!("../../../tests/corpus/colors.pdf"),
            "mupdf",
        ),
    ] {
        let (dir, app) = app().await;
        let res = app
            .clone()
            .oneshot(upload("fixture", format, document))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let job: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(job["manifest"]["engine"], "auto");
        let id = job["id"].as_str().unwrap();
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_jrip-worker"))
            .arg(id)
            .env("JRIP_DATA_ROOT", dir.path())
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let path = dir
            .path()
            .join("jobs")
            .join(id)
            .join("raster/page-000001.tiff");
        let bytes = std::fs::read(path).unwrap();
        let artifact_response = app
            .clone()
            .oneshot(request("GET", &format!("/api/v1/jobs/{id}/artifacts")))
            .await
            .unwrap();
        assert_eq!(artifact_response.status(), StatusCode::OK);
        let artifacts: serde_json::Value = serde_json::from_slice(
            &artifact_response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes(),
        )
        .unwrap();
        use sha2::{Digest, Sha256};
        assert_eq!(
            artifacts[0]["sha256"],
            format!("{:x}", Sha256::digest(&bytes))
        );
        assert_eq!(artifacts[0]["bytes"], bytes.len());

        assert!(bytes.starts_with(b"II\x2a\0") || bytes.starts_with(b"MM\0\x2a"));
        let res = app
            .oneshot(request("GET", &format!("/api/v1/jobs/{id}")))
            .await
            .unwrap();
        let job: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(job["state"], "COMPLETED");
        assert_eq!(job["selected_engine"], expected_engine);
    }
}

#[tokio::test]
async fn print_intent_negotiation_is_explicit_and_does_not_submit() {
    let (dir, app) = app().await;
    let intent = serde_json::json!({
        "document":{"uri":"https://example.org/proof.pdf","mediaType":"application/pdf","sha256":"a".repeat(64)},
        "copies":{"mode":"required","value":1},"output":{"destination":"artifact"},
        "dpi":{"mode":"preferred","value":9600},"colorMode":{"mode":"required","value":"gray"}
    });
    for (authorized, physical, expected) in [
        (false, false, StatusCode::UNAUTHORIZED),
        (true, false, StatusCode::OK),
        (true, true, StatusCode::OK),
    ] {
        let mut body = intent.clone();
        if physical {
            body["output"] = serde_json::json!({"destination":"printer","printerId":"office"});
        }
        let mut req =
            Request::post("/api/v1/print/resolve").header("content-type", "application/json");
        if authorized {
            req = req.header("authorization", format!("Bearer {TOKEN}"));
        }
        let res = app
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), expected);
        if authorized {
            let value: serde_json::Value =
                serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(value["executable"], !physical);
            assert_eq!(value["authorizationGranted"], false);
            if physical {
                assert!(value["ticket"].is_null());
            } else {
                assert_eq!(value["ticket"]["manifest"]["dpi"], 600);
                assert_eq!(value["ticket"]["manifest"]["color_mode"], "gray");
                assert_eq!(value["ticket"]["selectedEngine"], "mupdf");
                assert_eq!(value["adjustments"][0]["requested"], 9600);
            }
        }
    }
    let mut required = intent;
    required["dpi"]["mode"] = serde_json::json!("required");
    let result = rip_acx::intent::resolve(serde_json::from_value(required).unwrap());
    assert_eq!(result["executable"], false);
    assert_eq!(result["conflicts"][0]["code"], "unsupported_dpi");
    assert_eq!(
        std::fs::read_dir(dir.path().join("jobs")).unwrap().count(),
        0
    );
}

#[tokio::test]
async fn preflight_survives_restart_and_expired_records_are_rejected() {
    let (dir, app) = app().await;
    let intent = serde_json::json!({"document":{"uri":"https://example.org/proof.pdf","mediaType":"application/pdf","sha256":"a".repeat(64)},"copies":{"mode":"required","value":1},"output":{"destination":"artifact"}});
    let response = app
        .oneshot(
            Request::post("/api/v1/print/preflights")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(intent.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let created: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(created["commitAvailable"], true);
    let state = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let expired = uuid::Uuid::new_v4();
    state
        .repo
        .save_preflight(expired, &serde_json::json!({"expiresAt":0}), 0)
        .await
        .unwrap();
    let restarted = jrip_server::router(state);
    let response = restarted
        .clone()
        .oneshot(request(
            "GET",
            &format!(
                "/api/v1/print/preflights/{}",
                created["preflightId"].as_str().unwrap()
            ),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let loaded: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(loaded, created);
    use sha2::{Digest, Sha256};
    let mut unsigned = loaded.clone();
    let hash = unsigned
        .as_object_mut()
        .unwrap()
        .remove("preflightDigest")
        .unwrap();
    assert_eq!(
        hash,
        format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&unsigned).unwrap())
        )
    );
    assert_eq!(
        restarted
            .oneshot(request(
                "GET",
                &format!("/api/v1/print/preflights/{expired}")
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::GONE
    );
}

#[tokio::test]
async fn commit_checks_source_and_digest_and_deduplicates() {
    use sha2::{Digest, Sha256};
    let (dir, app) = app().await;
    let document = "%!PS-Adobe-3.0\nshowpage\n";
    let intent = serde_json::json!({"document":{"uri":"https://example.org/proof.ps","mediaType":"application/postscript","sha256":format!("{:x}",Sha256::digest(document))},"copies":{"mode":"required","value":1},"output":{"destination":"artifact"}});
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/print/preflights")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(intent.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let pf: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let path = format!(
        "/api/v1/print/preflights/{}/commit",
        pf["preflightId"].as_str().unwrap()
    );
    let make = |source: &str, digest: &str| {
        let body = format!(
            "--bound\r\nContent-Disposition: form-data; name=\"document\"; filename=\"proof.ps\"\r\n\r\n{source}\r\n--bound--\r\n"
        );
        Request::post(&path)
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("x-jrip-preflight-digest", digest)
            .header("content-type", "multipart/form-data; boundary=bound")
            .body(Body::from(body))
            .unwrap()
    };
    let hash = pf["preflightDigest"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(make(document, "tampered"))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.clone()
            .oneshot(make("%!PS-Adobe-3.0\nwrong\n", hash))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("jobs")).unwrap().count(),
        0
    );
    let (a, b) = tokio::join!(
        app.clone().oneshot(make(document, hash)),
        app.clone().oneshot(make(document, hash))
    );
    let mut ids = Vec::new();
    for response in [a.unwrap(), b.unwrap()] {
        assert!(matches!(
            response.status(),
            StatusCode::OK | StatusCode::CREATED
        ));
        let value: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        ids.push(value["job"]["id"].clone());
    }
    assert_eq!(ids[0], ids[1]);
    let job_id = ids[0].as_str().unwrap();
    let receipt_path = format!("/api/v1/jobs/{job_id}/receipt");
    assert_eq!(
        app.clone()
            .oneshot(Request::get(&receipt_path).body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let response = app
        .clone()
        .oneshot(request("GET", &receipt_path))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let receipt: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(receipt["status"], "accepted");
    assert_eq!(receipt["requestHash"], pf["requestHash"]);
    assert_eq!(receipt["proof"]["preflightDigest"], pf["preflightDigest"]);
    assert_eq!(
        receipt["proof"]["sourceSha256"],
        format!("{:x}", Sha256::digest(document))
    );
    assert_eq!(receipt["proof"]["signed"], false);
    let reopened = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let uuid = uuid::Uuid::parse_str(job_id).unwrap();
    let job = reopened.repo.get(uuid).await.unwrap();
    let canceled = reopened
        .repo
        .transition(job, rip_core::JobState::Cancelled, None)
        .await
        .unwrap();
    let result_receipt = reopened.repo.get_result_receipt(uuid).await.unwrap();
    assert_eq!(result_receipt["status"], "cancelled");
    assert_eq!(result_receipt["requestHash"], receipt["requestHash"]);
    assert_eq!(
        result_receipt["resultHash"],
        format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&result_receipt["proof"]["result"]).unwrap())
        )
    );
    reopened.repo.delete(canceled).await.unwrap();
    assert_eq!(
        reopened.repo.get_result_receipt(uuid).await.unwrap(),
        result_receipt
    );
    for outcome in ["completed", "failed", "expired"] {
        let mut job: rip_core::Job = reopened.repo.get(uuid).await.unwrap_or_else(|_| serde_json::from_value(serde_json::json!({
            "id":uuid,"manifest":pf["ticket"]["manifest"],"state":"QUEUED","source_sha256":receipt["proof"]["sourceSha256"],
            "created_at":rip_storage::now(),"updated_at":rip_storage::now(),"revision":0,"error_code":null
        })).unwrap());
        job.id = uuid::Uuid::new_v4();
        job.state = rip_core::JobState::Queued;
        job.revision = 0;
        let pid = uuid::Uuid::new_v4();
        reopened
            .repo
            .save_preflight(pid, &pf, rip_storage::now() + 300)
            .await
            .unwrap();
        reopened
            .repo
            .insert_with_preflight(&job, Some(pid))
            .await
            .unwrap();
        let terminal = if outcome == "expired" {
            let run = uuid::Uuid::new_v4();
            reopened
                .repo
                .claim_queued(job.clone(), run, 0)
                .await
                .unwrap();
            reopened
                .repo
                .recover_expired_leases(rip_storage::now())
                .await
                .unwrap();
            reopened.repo.get(job.id).await.unwrap()
        } else if outcome == "failed" {
            reopened
                .repo
                .transition(
                    job.clone(),
                    rip_core::JobState::Failed,
                    Some("TEST_FAILURE".into()),
                )
                .await
                .unwrap()
        } else {
            let ripping = reopened
                .repo
                .transition(job.clone(), rip_core::JobState::Ripping, None)
                .await
                .unwrap();
            let spool = reopened
                .repo
                .transition(ripping, rip_core::JobState::Spooling, None)
                .await
                .unwrap();
            reopened
                .repo
                .complete_with_artifacts(
                    spool,
                    &[rip_core::Artifact {
                        page: 1,
                        name: "page-000001.tiff".into(),
                        media_type: "image/tiff".into(),
                        bytes: 4,
                        sha256: "a".repeat(64),
                    }],
                )
                .await
                .unwrap()
        };
        let result = reopened.repo.get_result_receipt(terminal.id).await.unwrap();
        assert_eq!(
            result["status"],
            if outcome == "completed" {
                "succeeded"
            } else {
                "failed"
            }
        );
        assert_eq!(
            result["proof"]["result"]["artifactHashesVerified"],
            outcome == "completed"
        );
        if outcome == "completed" {
            assert_eq!(result["proof"]["result"]["pageCount"], 1);
            assert_eq!(
                result["proof"]["result"]["artifacts"][0]["sha256"],
                "a".repeat(64)
            );
        }
        if outcome == "expired" {
            assert_eq!(result["proof"]["result"]["errorCode"], "LEASE_EXPIRED");
        }
    }

    assert_eq!(
        reopened.repo.get_print_receipt(uuid).await.unwrap(),
        receipt
    );

    assert_eq!(
        std::fs::read_dir(dir.path().join("jobs")).unwrap().count(),
        1
    );
}

#[tokio::test]
async fn artifact_hashing_is_bounded_and_preserves_page_order() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let mut pages = Vec::new();
    for page in 1..=2 {
        let path = dir.path().join(format!("page-{page:06}.tiff"));
        tokio::fs::write(&path, vec![page as u8; 70000])
            .await
            .unwrap();
        pages.push(path);
    }
    let artifacts = jrip_server::artifacts::hash_pages(&pages).await.unwrap();
    assert_eq!(artifacts[1].page, 2);
    assert_eq!(artifacts[0].bytes, 70000);
    assert_eq!(
        artifacts[0].sha256,
        format!("{:x}", Sha256::digest(vec![1; 70000]))
    );
    pages.reverse();
    assert!(jrip_server::artifacts::hash_pages(&pages).await.is_err());
    assert!(jrip_server::artifacts::hash_pages(&[]).await.is_err());
}

#[tokio::test]
async fn artifact_download_verifies_content_and_rejects_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let state = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let manifest: rip_core::Manifest =
        serde_json::from_value(serde_json::json!({"name":"proof","format":"application/pdf"}))
            .unwrap();
    let job = rip_jobs::submit_bytes(&state.repo, &state.root, manifest, b"%PDF-1.4\n")
        .await
        .unwrap();
    let raster = state
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("raster");
    tokio::fs::create_dir(&raster).await.unwrap();
    let path = raster.join("page-000001.tiff");
    let data = vec![42u8; 140000];
    tokio::fs::write(&path, &data).await.unwrap();
    let artifacts = jrip_server::artifacts::hash_pages(std::slice::from_ref(&path))
        .await
        .unwrap();
    let ripping = state
        .repo
        .transition(job, rip_core::JobState::Ripping, None)
        .await
        .unwrap();
    let spooling = state
        .repo
        .transition(ripping, rip_core::JobState::Spooling, None)
        .await
        .unwrap();
    let completed = state
        .repo
        .complete_with_artifacts(spooling, &artifacts)
        .await
        .unwrap();
    let app = jrip_server::router(state);
    let url = format!("/api/v1/jobs/{}/artifacts/page-000001.tiff", completed.id);
    assert_eq!(
        app.clone()
            .oneshot(Request::get(&url).body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let response = app.clone().oneshot(request("GET", &url)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/tiff");
    assert_eq!(response.headers()["content-length"], data.len().to_string());
    // Alter the source after verification; the response must still return the snapshot.
    tokio::fs::write(&path, vec![43u8; data.len()])
        .await
        .unwrap();
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        data
    );
    assert_eq!(
        app.clone()
            .oneshot(request("GET", &url))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.clone()
            .oneshot(request("POST", &format!("{url}/verify")))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    tokio::fs::write(&path, &data).await.unwrap();
    assert_eq!(
        app.clone()
            .oneshot(request("POST", &format!("{url}/verify")))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.oneshot(request("GET", &url.replace("page-000001", "page-999999")))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}
