//! Follow advertised routes rather than duplicating client endpoint knowledge.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
const OWNER: &str = "alice-test-token-at-least-24-characters";
const OTHER: &str = "bob-test-token-at-least-24-characters";
fn request(method: &str, path: &str, token: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(body)
        .unwrap()
}
async fn value(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
fn credentials() -> Vec<jrip_server::Credential> {
    [(OWNER, "alice"), (OTHER, "bob")]
        .into_iter()
        .map(|(token, identity)| jrip_server::Credential {
            token: token.into(),
            identity: identity.into(),
            scopes: vec!["jobs:submit".into(), "jobs:read".into()],
        })
        .collect()
}
#[tokio::test]
async fn advertised_settings_lifecycle_binds_hash_actor_and_durable_acceptance() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = jrip_server::open(dir.path().into(), OWNER.into())
        .await
        .unwrap();
    state.credentials = credentials();
    let router = jrip_server::router(state.clone());
    let discovery = router
        .clone()
        .oneshot(request(
            "GET",
            "/.well-known/acx.json",
            OWNER,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(discovery.status(), StatusCode::OK);
    let manifest = value(discovery).await;
    let lifecycle = &manifest["capabilities"][0]["extensions"]["org.jrip.print"]["lifecycle"];
    assert_eq!(lifecycle["experimental"], true);
    assert_eq!(
        lifecycle["authorization"]["preflightGrantsExecution"],
        false
    );
    let document = "%!PS-Adobe-3.0\nshowpage\n";
    let intent = json!({"document":{"uri":"https://example.org/source.ps","mediaType":"application/postscript","sha256":format!("{:x}",Sha256::digest(document))},"copies":{"mode":"required","value":1},"dpi":{"mode":"required","value":72},"output":{"destination":"artifact"}});
    let mut preflight = request(
        "POST",
        lifecycle["preflightEndpoint"].as_str().unwrap(),
        OWNER,
        Body::from(intent.to_string()),
    );
    preflight
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    let response = router.clone().oneshot(preflight).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let pf = value(response).await;
    assert_eq!(pf["profile"], lifecycle["preflightProfile"]);
    assert_eq!(pf["digestProfile"], lifecycle["digestProfile"]);
    assert_eq!(pf["documentVerified"], false);
    assert_eq!(
        pf["requestHash"],
        rip_digest::digest(&pf["ticket"]).unwrap()
    );
    let mut unsigned = pf.clone();
    unsigned.as_object_mut().unwrap().remove("preflightDigest");
    assert_eq!(
        pf["preflightDigest"],
        rip_digest::digest(&unsigned).unwrap()
    );
    assert!(state.repo.list(10, 0).await.unwrap().is_empty());
    let id = pf["preflightId"].as_str().unwrap();
    let commit_path = lifecycle["commitTemplate"]
        .as_str()
        .unwrap()
        .replace("{preflightId}", id);
    let commit = |token: &str, digest: &str, bytes: &str| {
        let mut req = request(
            "POST",
            &commit_path,
            token,
            Body::from(format!(
                "--bound\r\nContent-Disposition: form-data; name=\"document\"\r\n\r\n{bytes}\r\n--bound--\r\n"
            )),
        );
        req.headers_mut().insert(
            "content-type",
            "multipart/form-data; boundary=bound".parse().unwrap(),
        );
        req.headers_mut().insert(
            lifecycle["digestHeader"]
                .as_str()
                .unwrap()
                .parse::<axum::http::HeaderName>()
                .unwrap(),
            digest.parse().unwrap(),
        );
        req
    };
    let digest = pf["preflightDigest"].as_str().unwrap();
    for (req, status) in [
        (commit(OTHER, digest, document), StatusCode::FORBIDDEN),
        (
            commit(OWNER, "sha256:wrong", document),
            StatusCode::CONFLICT,
        ),
        (
            commit(OWNER, digest, "%!PS-Adobe-3.0\nchanged\n"),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(router.clone().oneshot(req).await.unwrap().status(), status);
        assert!(state.repo.list(10, 0).await.unwrap().is_empty());
    }
    let (a, b) = tokio::join!(
        router.clone().oneshot(commit(OWNER, digest, document)),
        router.clone().oneshot(commit(OWNER, digest, document))
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert!([StatusCode::CREATED, StatusCode::OK].contains(&a.status()));
    assert!([StatusCode::CREATED, StatusCode::OK].contains(&b.status()));
    let a = value(a).await;
    let b = value(b).await;
    assert_eq!(a["job"]["id"], b["job"]["id"]);
    assert_eq!(state.repo.list(10, 0).await.unwrap().len(), 1);
    let mut reopened = jrip_server::open(dir.path().into(), OWNER.into())
        .await
        .unwrap();
    reopened.credentials = credentials();
    let router = jrip_server::router(reopened);
    let response = router
        .clone()
        .oneshot(commit(OWNER, digest, document))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let replay = value(response).await;
    assert_eq!(replay["job"]["id"], a["job"]["id"]);
    assert_eq!(replay["replayed"], true);
    let job_id = a["job"]["id"].as_str().unwrap();
    let receipt_path = lifecycle["acceptanceReceiptTemplate"]
        .as_str()
        .unwrap()
        .replace("{jobId}", job_id);
    assert_eq!(a["receiptUrl"], receipt_path);
    let receipt = router
        .clone()
        .oneshot(request("GET", &receipt_path, OWNER, Body::empty()))
        .await
        .unwrap();
    assert_eq!(receipt.status(), StatusCode::OK);
    let receipt = value(receipt).await;
    assert_eq!(receipt["requestHash"], pf["requestHash"]);
    assert_eq!(receipt["proof"]["preflightDigest"], pf["preflightDigest"]);
    assert_eq!(receipt["proof"]["actor"], "alice");
    assert!(!receipt.to_string().contains(OWNER));
    let result_path = lifecycle["resultReceiptTemplate"]
        .as_str()
        .unwrap()
        .replace("{jobId}", job_id);
    assert_eq!(
        router
            .oneshot(request("GET", &result_path, OWNER, Body::empty()))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
#[ignore = "requires Python jsonschema and Node.js for independent ACX verification"]
async fn independent_acx_schemas_and_signed_terminal_profiles_reject_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), OWNER.into())
        .await
        .unwrap();
    let private =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let signer = rip_storage::receipts::ReceiptSigner::from_pkcs8(private.as_ref()).unwrap();
    let public = dir.path().join("public.raw");
    std::fs::write(&public, signer.public_key()).unwrap();
    app.repo = rip_storage::Repository::open_with_receipt_signer(
        &app.root.join("jobs.sqlite3"),
        Some(signer),
    )
    .await
    .unwrap();
    app.credentials = credentials();
    let router = jrip_server::router(app.clone());
    let manifest = value(
        router
            .clone()
            .oneshot(request(
                "GET",
                "/.well-known/acx.json",
                OWNER,
                Body::empty(),
            ))
            .await
            .unwrap(),
    )
    .await;
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/check-acx-lifecycle.py");
    let bundle_path = dir.path().join("bundle.json");
    let run = |bundle: &Value| {
        std::fs::write(&bundle_path, serde_json::to_vec(bundle).unwrap()).unwrap();
        std::process::Command::new("python3")
            .arg(&script)
            .arg(&bundle_path)
            .arg(&public)
            .output()
            .unwrap()
    };
    for outcome in ["cancelled", "failed", "completed"] {
        let document = "%!PS-Adobe-3.0\nshowpage\n";
        let intent = json!({"document":{"uri":"https://example.org/source.ps","mediaType":"application/postscript","sha256":format!("{:x}",Sha256::digest(document))},"copies":{"mode":"required","value":1},"output":{"destination":"artifact"}});
        let mut req = request(
            "POST",
            "/api/v1/print/preflights",
            OWNER,
            Body::from(intent.to_string()),
        );
        req.headers_mut()
            .insert("content-type", "application/json".parse().unwrap());
        let response = router.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let pf = value(response).await;
        let mut req = request(
            "POST",
            &format!(
                "/api/v1/print/preflights/{}/commit",
                pf["preflightId"].as_str().unwrap()
            ),
            OWNER,
            Body::from(format!(
                "--bound\r\nContent-Disposition: form-data; name=\"document\"\r\n\r\n{document}\r\n--bound--\r\n"
            )),
        );
        req.headers_mut().insert(
            "content-type",
            "multipart/form-data; boundary=bound".parse().unwrap(),
        );
        req.headers_mut().insert(
            "x-jrip-preflight-digest",
            pf["preflightDigest"].as_str().unwrap().parse().unwrap(),
        );
        let response = router.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let committed = value(response).await;
        let id = committed["job"]["id"].as_str().unwrap().parse().unwrap();
        let job = app.repo.get(id).await.unwrap();
        if outcome == "completed" {
            // Exercise the production receipt publisher with synthetic artifact metadata.
            // This is explicitly not an interpreter or physical-output acceptance test.
            let job = app
                .repo
                .transition(job, rip_core::JobState::Ripping, None)
                .await
                .unwrap();
            let job = app
                .repo
                .transition(job, rip_core::JobState::Spooling, None)
                .await
                .unwrap();
            app.repo
                .complete_with_artifacts(
                    job,
                    &[rip_core::Artifact {
                        page: 1,
                        name: "page-000001.tiff".into(),
                        media_type: "image/tiff".into(),
                        bytes: 4,
                        sha256: "a".repeat(64),
                    }],
                )
                .await
                .unwrap();
        } else {
            app.repo
                .transition(
                    job,
                    if outcome == "failed" {
                        rip_core::JobState::Failed
                    } else {
                        rip_core::JobState::Cancelled
                    },
                    if outcome == "failed" {
                        Some("TEST_FAILURE".into())
                    } else {
                        None
                    },
                )
                .await
                .unwrap();
        }
        let accepted = app.repo.get_print_receipt(id).await.unwrap();
        let result = app.repo.get_result_receipt(id).await.unwrap();
        let bundle =
            json!({"manifest":manifest,"preflight":pf,"accepted":accepted,"result":result});
        let output = run(&bundle);
        assert!(
            output.status.success(),
            "outcome={outcome}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        for pointer in [
            "/accepted/provider",
            "/accepted/requestHash",
            "/accepted/proof/actor",
            "/result/proof/acceptanceReceiptId",
            "/result/resultHash",
            "/result/proof/result/sourceSha256",
            "/result/proof/result/completionMeaning",
            "/result/status",
            "/result/issuedAt",
            "/preflight/requestHash",
        ] {
            let mut bad = bundle.clone();
            *bad.pointer_mut(pointer).unwrap() = json!("tampered");
            // A valid provider signature must not hide broken cross-record semantics.
            for field in ["accepted", "result"] {
                let record = &mut bad[field];
                record.as_object_mut().unwrap().remove("signature");
                let signer =
                    rip_storage::receipts::ReceiptSigner::from_pkcs8(private.as_ref()).unwrap();
                signer.sign(record).unwrap();
            }
            assert!(!run(&bad).status.success(), "accepted mutation {pointer}");
        }
    }
}
