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
    let ticket_path = format!("{path}/ticket");
    let response = app
        .clone()
        .oneshot(request("GET", &ticket_path))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ticket: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(ticket["jobId"], id);
    assert_eq!(ticket["selectedEngine"], "ghostscript");
    assert_eq!(ticket["document"]["sha256"], job["source_sha256"]);
    assert_eq!(ticket["document"]["integrityVerified"], true);
    assert_eq!(ticket["document"]["structureVerified"], false);
    assert_eq!(ticket["completionMeaning"], "raster_export");
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
        app.clone()
            .oneshot(request("GET", &path))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let restarted = jrip_server::router(
        jrip_server::open(dir.path().into(), TOKEN.into())
            .await
            .unwrap(),
    );
    let response = restarted
        .oneshot(request("GET", &ticket_path))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let retained: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(retained, ticket);
}

#[tokio::test]
async fn common_capabilities_and_ticket_validation_are_explicit() {
    let (dir, app) = app().await;
    let response = app
        .clone()
        .oneshot(request("GET", "/api/v1/print/capabilities"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let model: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(model["stages"].as_array().unwrap().len(), 3);
    assert_eq!(model["executionPaths"]["automaticPhysicalPipeline"], false);
    assert_eq!(model["authorizationGranted"], false);
    let intent = serde_json::json!({"document":{"uri":"https://example.org/test.pdf","mediaType":"application/pdf","sha256":"a".repeat(64)},"copies":{"mode":"required","value":1},"output":{"destination":"artifact"}});
    let resolved = rip_acx::intent::resolve(serde_json::from_value(intent).unwrap());
    let mut ticket = resolved["ticket"].clone();
    for expected in [true, false] {
        if !expected {
            ticket["documentVerified"] = serde_json::json!(true);
        }
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/v1/print/validate-ticket")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("content-type", "application/json")
                    .body(Body::from(ticket.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(result["valid"], expected);
        assert_eq!(result["documentVerified"], false);
    }
    assert_eq!(
        std::fs::read_dir(dir.path().join("jobs")).unwrap().count(),
        0
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
    let mut unsigned = loaded.clone();
    let hash = unsigned
        .as_object_mut()
        .unwrap()
        .remove("preflightDigest")
        .unwrap();
    assert_eq!(hash, rip_digest::digest(&unsigned).unwrap());
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
    let budget = state.artifact_budget.clone();
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
    for (range, first, last) in [
        ("bytes=2-7", 2, 7),
        ("bytes=-3", data.len() - 3, data.len() - 1),
        ("bytes=139990-", 139990, data.len() - 1),
    ] {
        let req = Request::get(&url)
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("range", range)
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            res.headers()["content-range"],
            format!("bytes {first}-{last}/{}", data.len())
        );
        assert_eq!(
            res.into_body().collect().await.unwrap().to_bytes().as_ref(),
            &data[first..=last]
        );
    }
    for range in ["bytes=999999-", "bytes=0-1,4-5", "bytes=-0", "bytes=+1-2"] {
        let req = Request::get(&url)
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("range", range)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::RANGE_NOT_SATISFIABLE
        );
    }
    let req = Request::get(&url)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("range", "bytes=1-2")
        .header("if-range", "\"old\"")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.into_body().collect().await.unwrap().to_bytes().len(),
        data.len()
    );
    let reservation = budget.clone().acquire_many_owned(1024).await.unwrap();
    assert_eq!(
        app.clone()
            .oneshot(request("GET", &url))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    drop(reservation);
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

#[tokio::test]
async fn events_resume_after_restart_and_metadata_deletion() {
    let (dir, app) = app().await;
    let response = app
        .clone()
        .oneshot(upload(
            "events.ps",
            "application/postscript",
            "%!PS\nshowpage\n",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let job: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let path = format!("/api/v1/jobs/{}", job["id"].as_str().unwrap());
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
    let response = app
        .clone()
        .oneshot(request("GET", &format!("{path}/events?limit=2")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let first: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(first["events"].as_array().unwrap().len(), 2);
    assert_eq!(first["hasMore"], true);
    let cursor = first["nextCursor"].as_i64().unwrap();
    assert_eq!(first["events"][0]["state"], "QUEUED");
    assert_eq!(first["events"][1]["state"], "HELD");
    assert_eq!(first["events"][1]["stage"], "QUEUED_FOR_RIP");
    assert_eq!(first["events"][1]["reasonCode"], "HELD_BY_USER");
    assert!(first["events"][1]["progressPercent"].is_null());
    // Another job's later event must not enter this job's resumed history.
    assert_eq!(
        app.clone()
            .oneshot(upload(
                "other.ps",
                "application/postscript",
                "%!PS\nshowpage\n"
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        app.clone()
            .oneshot(request("DELETE", &path))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    drop(app);
    let app = jrip_server::router(
        jrip_server::open(dir.path().into(), TOKEN.into())
            .await
            .unwrap(),
    );
    let response = app
        .clone()
        .oneshot(request("GET", &format!("{path}/events?after={cursor}")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let rest: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(rest["events"].as_array().unwrap().len(), 2);
    assert_eq!(rest["hasMore"], false);
    assert_eq!(rest["events"][0]["state"], "QUEUED");
    assert_eq!(rest["events"][1]["state"], "CANCELLED");
    assert_eq!(rest["events"][1]["reasonCode"], "CANCELED_BY_USER");
    assert_eq!(rest["events"][1]["stage"], "CANCELED");
    assert!(rest["events"][0]["sequence"].as_i64().unwrap() > cursor);
    let end = rest["nextCursor"].as_i64().unwrap();
    let response = app
        .clone()
        .oneshot(request("GET", &format!("{path}/events?after={end}")))
        .await
        .unwrap();
    let empty: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(empty["events"], serde_json::json!([]));
    assert_eq!(empty["nextCursor"], end);
    for query in ["after=-1", "limit=0", "limit=101"] {
        assert_eq!(
            app.clone()
                .oneshot(request("GET", &format!("{path}/events?{query}")))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        app.clone()
            .oneshot(
                Request::get(format!("{path}/events"))
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let unknown = format!("/api/v1/jobs/{}/events", uuid::Uuid::new_v4());
    assert_eq!(
        app.oneshot(request("GET", &unknown))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn scoped_api_credentials_enforce_operations_and_reject_legacy_bypass() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let reader = "reader-token-at-least-24-characters";
    let submitter = "submit-token-at-least-24-characters";
    state.credentials = vec![
        jrip_server::Credential {
            token: reader.into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
        },
        jrip_server::Credential {
            token: submitter.into(),
            identity: "submitter".into(),
            scopes: vec!["jobs:submit".into()],
        },
    ];
    let app = jrip_server::router(state);
    let id = uuid::Uuid::new_v4();
    for (token, method, path, status) in [
        (reader, "GET", "/api/v1/jobs".into(), StatusCode::OK),
        (
            TOKEN,
            "GET",
            "/api/v1/jobs".into(),
            StatusCode::UNAUTHORIZED,
        ),
        (
            submitter,
            "GET",
            "/api/v1/jobs".into(),
            StatusCode::FORBIDDEN,
        ),
        (
            reader,
            "POST",
            format!("/api/v1/jobs/{id}/hold"),
            StatusCode::FORBIDDEN,
        ),
        (
            reader,
            "DELETE",
            format!("/api/v1/jobs/{id}"),
            StatusCode::FORBIDDEN,
        ),
        (
            reader,
            "GET",
            format!("/api/v1/jobs/{id}/artifacts"),
            StatusCode::FORBIDDEN,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    let response = app
        .oneshot(
            Request::post(format!("/api/v1/jobs/{id}/retry"))
                .header("authorization", format!("Bearer {submitter}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"requestId":uuid::Uuid::new_v4(),"expectedRevision":0})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn concurrent_retries_deduplicate_and_reject_tampered_sources() {
    let (dir, app) = app().await;
    let response = app
        .clone()
        .oneshot(upload(
            "retry.ps",
            "application/postscript",
            "%!PS-Adobe-3.0\nshowpage\n",
        ))
        .await
        .unwrap();
    let job: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = job["id"].as_str().unwrap();
    assert_eq!(
        app.clone()
            .oneshot(request("POST", &format!("/api/v1/jobs/{id}/cancel")))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let key = uuid::Uuid::new_v4();
    let make = |revision, request_id| {
        Request::post(format!("/api/v1/jobs/{id}/retry"))
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"expectedRevision":revision,"requestId":request_id}).to_string(),
            ))
            .unwrap()
    };
    assert_eq!(
        app.clone()
            .oneshot(make(0, uuid::Uuid::new_v4()))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let (first, second) = tokio::join!(
        app.clone().oneshot(make(1, key)),
        app.clone().oneshot(make(1, key))
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    let first: serde_json::Value =
        serde_json::from_slice(&first.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let second: serde_json::Value =
        serde_json::from_slice(&second.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(first["job"]["id"], second["job"]["id"]);
    assert_eq!(
        std::fs::read_dir(dir.path().join("jobs")).unwrap().count(),
        2
    );
    assert_eq!(
        app.clone().oneshot(make(0, key)).await.unwrap().status(),
        StatusCode::CONFLICT
    );
    tokio::fs::write(
        dir.path().join("jobs").join(id).join("document"),
        b"%!PS-Adobe-3.0\nchanged\n",
    )
    .await
    .unwrap();
    let response = app.oneshot(make(1, uuid::Uuid::new_v4())).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        std::fs::read_dir(dir.path().join("jobs")).unwrap().count(),
        2
    );
}

#[tokio::test]
async fn audit_is_durable_redacted_and_requires_admin() {
    let (dir, router) = app().await;
    let response = router
        .clone()
        .oneshot(request("GET", "/api/v1/jobs?secret=never-record-this"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().contains_key("x-jrip-correlation-id"));
    let response = router
        .clone()
        .oneshot(Request::get("/api/v1/jobs").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let repo = rip_storage::Repository::open(&dir.path().join("jobs.sqlite3"))
        .await
        .unwrap();
    let evidence = repo.audit_records(0, 100).await.unwrap();
    assert_eq!(evidence["records"].as_array().unwrap().len(), 2);
    assert_eq!(evidence["records"][0]["identity"], "legacy-local");
    assert_eq!(evidence["records"][0]["status"], 200);
    assert_eq!(evidence["records"][1]["identity"], "unauthenticated");
    assert_eq!(evidence["records"][1]["status"], 401);
    assert!(!evidence.to_string().contains(TOKEN));
    assert!(!evidence.to_string().contains("never-record-this"));
    let response = router
        .clone()
        .oneshot(request("GET", "/api/v1/audit?limit=1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["hasMore"], true);
    assert_eq!(body["records"].as_array().unwrap().len(), 1);
    let response = router
        .oneshot(Request::get("/api/v1/audit").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let pending = repo
        .begin_audit(uuid::Uuid::new_v4(), "worker", "interrupted", None)
        .await
        .unwrap();
    let reopened = rip_storage::Repository::open(&dir.path().join("jobs.sqlite3"))
        .await
        .unwrap();
    let rows = reopened.audit_records(pending - 1, 1).await.unwrap();
    assert!(rows["records"][0]["finishedAt"].is_null());
}

#[tokio::test]
async fn preflight_digest_versions_survive_restart_and_commit_uses_stored_profile() {
    use sha2::{Digest, Sha256};
    for legacy in [false, true] {
        let (dir, api) = app().await;
        let document = "%!PS-Adobe-3.0\nshowpage\n";
        let intent = serde_json::json!({"document":{"uri":"https://example.org/proof.ps","mediaType":"application/postscript","sha256":format!("{:x}",Sha256::digest(document))},"copies":{"mode":"required","value":1},"output":{"destination":"artifact"}});
        let mut request = Request::post("/api/v1/print/preflights")
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("content-type", "application/json");
        if legacy {
            request = request.header("x-jrip-digest-profile", "org.jrip.serde-json-v1");
        }
        let response = api
            .oneshot(request.body(Body::from(intent.to_string())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let pf: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(
            pf["profile"],
            if legacy {
                "org.jrip.settings-preflight-v1"
            } else {
                "org.jrip.settings-preflight-v2"
            }
        );
        let expected = if legacy {
            "org.jrip.serde-json-v1"
        } else {
            "org.jrip.jcs-safe-v1"
        };
        assert_eq!(pf["digestProfile"], expected);
        let mut unsigned = pf.clone();
        unsigned.as_object_mut().unwrap().remove("preflightDigest");
        let computed = if legacy {
            format!(
                "sha256:{:x}",
                Sha256::digest(serde_json::to_vec(&unsigned).unwrap())
            )
        } else {
            rip_digest::digest(&unsigned).unwrap()
        };
        assert_eq!(pf["preflightDigest"], computed);
        let reopened = jrip_server::open(dir.path().into(), TOKEN.into())
            .await
            .unwrap();
        let response=jrip_server::router(reopened.clone()).oneshot(Request::post(format!("/api/v1/print/preflights/{}/commit",pf["preflightId"].as_str().unwrap())).header("authorization",format!("Bearer {TOKEN}")).header("x-jrip-preflight-digest",&computed)
            .header("x-jrip-digest-profile",if legacy{"org.jrip.jcs-safe-v1"}else{"org.jrip.serde-json-v1"})
            .header("content-type","multipart/form-data; boundary=bound").body(Body::from(format!("--bound\r\nContent-Disposition: form-data; name=\"document\"\r\n\r\n{document}\r\n--bound--\r\n"))).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let result: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let receipt = reopened
            .repo
            .get_print_receipt(
                uuid::Uuid::parse_str(result["job"]["id"].as_str().unwrap()).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(receipt["proof"]["requestHashDigestProfile"], expected);
        assert_eq!(receipt["proof"]["preflightDigestProfile"], expected);
    }
}

#[tokio::test]
async fn settings_preflight_owner_scope_rotation_and_receipt_survive_restart() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let mut state = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let credentials = vec![
        jrip_server::Credential {
            token: TOKEN.into(),
            identity: "alice".into(),
            scopes: vec!["jobs:submit".into()],
        },
        jrip_server::Credential {
            token: "rotated-token-at-least-24-characters".into(),
            identity: "alice".into(),
            scopes: vec!["jobs:submit".into()],
        },
        jrip_server::Credential {
            token: "bob-admin-token-at-least-24-characters".into(),
            identity: "bob".into(),
            scopes: vec!["admin".into()],
        },
        jrip_server::Credential {
            token: "reader-token-at-least-24-characters".into(),
            identity: "alice".into(),
            scopes: vec!["jobs:read".into()],
        },
    ];
    state.credentials = credentials.clone();
    let document = "%!PS-Adobe-3.0\nshowpage\n";
    let intent = serde_json::json!({"document":{"uri":"https://example.org/proof.ps","mediaType":"application/postscript","sha256":format!("{:x}",Sha256::digest(document))},"copies":{"mode":"required","value":1},"output":{"destination":"artifact"}});
    let response = jrip_server::router(state)
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
    let pf: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(pf["actor"], "alice");
    assert_eq!(pf["authorizationEvidence"]["mechanism"], "scoped-bearer");
    assert_eq!(
        pf["authorizationEvidence"]["requiredScopes"],
        serde_json::json!(["jobs:submit"])
    );
    assert_eq!(pf["authorizationGranted"], false);
    let id = pf["preflightId"].as_str().unwrap();
    let mut reopened = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    reopened.credentials = credentials;
    let router = jrip_server::router(reopened.clone());
    let commit = |token: &str| {
        Request::post(format!("/api/v1/print/preflights/{id}/commit")).header("authorization",format!("Bearer {token}")).header("x-jrip-preflight-digest",pf["preflightDigest"].as_str().unwrap()).header("content-type","multipart/form-data; boundary=bound").body(Body::from(format!("--bound\r\nContent-Disposition: form-data; name=\"document\"\r\n\r\n{document}\r\n--bound--\r\n"))).unwrap()
    };
    for token in [
        "bob-admin-token-at-least-24-characters",
        "reader-token-at-least-24-characters",
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/print/preflights/{id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            router
                .clone()
                .oneshot(commit(token))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert!(reopened.repo.list(100, 0).await.unwrap().is_empty());
    let response = router
        .clone()
        .oneshot(commit("rotated-token-at-least-24-characters"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let result: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let receipt = reopened
        .repo
        .get_print_receipt(result["job"]["id"].as_str().unwrap().parse().unwrap())
        .await
        .unwrap();
    assert_eq!(receipt["proof"]["authorization"], "scoped-bearer");
    assert_eq!(receipt["proof"]["actor"], "alice");
    assert_eq!(
        receipt["proof"]["authorizationEvidence"],
        pf["authorizationEvidence"]
    );
    assert!(!receipt.to_string().contains(TOKEN));
    assert!(!receipt.to_string().contains("rotated-token"));
    assert_eq!(
        router
            .clone()
            .oneshot(commit("bob-admin-token-at-least-24-characters"))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        router.oneshot(commit(TOKEN)).await.unwrap().status(),
        StatusCode::OK
    );
    let job_id: uuid::Uuid = result["job"]["id"].as_str().unwrap().parse().unwrap();
    let source = reopened.repo.get(job_id).await.unwrap();
    let cancelled = reopened
        .repo
        .transition(source, rip_core::JobState::Cancelled, None)
        .await
        .unwrap();
    let mut retry_app = reopened.clone();
    retry_app.credentials[0].scopes.push("jobs:control".into());
    let retry_api = jrip_server::router(retry_app);
    let retry_id = uuid::Uuid::new_v4();
    let retry_request = |token: &str| {
        Request::post(format!("/api/v1/jobs/{job_id}/retry"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"expectedRevision":cancelled.revision,"requestId":retry_id})
                    .to_string(),
            ))
            .unwrap()
    };
    let response = retry_api
        .clone()
        .oneshot(retry_request(TOKEN))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let retried: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let receipt = reopened
        .repo
        .get_print_receipt(retried["job"]["id"].as_str().unwrap().parse().unwrap())
        .await
        .unwrap();
    assert_eq!(receipt["proof"]["actor"], "alice");
    assert_eq!(
        receipt["proof"]["authorizationEvidence"]["requiredScopes"],
        serde_json::json!(["jobs:submit", "jobs:control"])
    );
    assert_eq!(
        retry_api
            .oneshot(retry_request("bob-admin-token-at-least-24-characters"))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    // Historical actor-less records remain usable only in legacy shared-token mode.
    let legacy_id = uuid::Uuid::new_v4();
    let mut legacy = pf.clone();
    legacy["preflightId"] = serde_json::json!(legacy_id);
    legacy.as_object_mut().unwrap().remove("actor");
    legacy
        .as_object_mut()
        .unwrap()
        .remove("authorizationEvidence");
    legacy.as_object_mut().unwrap().remove("preflightDigest");
    legacy["preflightDigest"] = serde_json::json!(rip_digest::digest(&legacy).unwrap());
    reopened
        .repo
        .save_preflight(legacy_id, &legacy, rip_storage::now() + 300)
        .await
        .unwrap();
    let response = jrip_server::router(reopened.clone())
        .oneshot(
            Request::get(format!("/api/v1/print/preflights/{legacy_id}"))
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    reopened.credentials.clear();
    let response = jrip_server::router(reopened)
        .oneshot(
            Request::get(format!("/api/v1/print/preflights/{legacy_id}"))
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
