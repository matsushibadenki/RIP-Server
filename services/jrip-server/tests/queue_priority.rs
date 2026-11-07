use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const DOCUMENT: &str = "%!PS-Adobe-3.0\nshowpage\n";
async fn post(app: &jrip_server::App, path: &str, value: Value) -> (StatusCode, Value) {
    let response = jrip_server::router(app.clone())
        .oneshot(
            Request::post(path)
                .header("authorization", "Bearer token")
                .header("content-type", "application/json")
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (
        status,
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap(),
    )
}
fn intent(priority: Value) -> Value {
    json!({"document":{"uri":"https://example.org/job.ps","mediaType":"application/postscript",
        "sha256":format!("{:x}",Sha256::digest(DOCUMENT))},
        "copies":{"mode":"required","value":1},"output":{"destination":"artifact"},
        "priority":priority})
}
#[tokio::test]
async fn negotiated_priority_is_bound_committed_durable_and_used_by_the_rip_queue() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    let (status, rejected) = post(
        &app,
        "/api/v1/print/preflights",
        intent(json!({"mode":"required","value":101})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        rejected["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["field"] == "priority")
    );
    for invalid in [json!(-1), json!(1.5), json!("high"), json!(4294967296u64)] {
        let response = jrip_server::router(app.clone())
            .oneshot(
                Request::post("/api/v1/print/preflights")
                    .header("authorization", "Bearer token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        intent(json!({"mode":"required","value":invalid})).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    assert!(app.repo.list(100, 0).await.unwrap().is_empty());
    let mut jobs = vec![];
    let mut hashes = vec![];
    for (requirement, resolved, adjusted) in [
        (json!({"mode":"required","value":0}), 0, false),
        (json!({"mode":"required","value":100}), 100, false),
        (json!({"mode":"preferred","value":75}), 75, false),
        (json!({"mode":"preferred","value":101}), 50, true),
        (json!({"mode":"automatic","value":100}), 50, true),
    ] {
        let (status, pf) = post(&app, "/api/v1/print/preflights", intent(requirement)).await;
        assert_eq!(status, StatusCode::CREATED, "{pf}");
        assert_eq!(pf["ticket"]["manifest"]["priority"], resolved);
        assert_eq!(
            pf["adjustments"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["field"] == "priority"),
            adjusted
        );
        assert_eq!(
            pf["requestHash"],
            rip_digest::digest(&pf["ticket"]).unwrap()
        );
        hashes.push(pf["requestHash"].clone());
        let path = format!(
            "/api/v1/print/preflights/{}/commit",
            pf["preflightId"].as_str().unwrap()
        );
        let request = || {
            Request::post(&path).header("authorization","Bearer token")
            .header("content-type","multipart/form-data; boundary=bound")
            .header("x-jrip-preflight-digest",pf["preflightDigest"].as_str().unwrap())
            .body(Body::from(format!("--bound\r\nContent-Disposition: form-data; name=\"document\"\r\n\r\n{DOCUMENT}\r\n--bound--\r\n"))).unwrap()
        };
        let response = jrip_server::router(app.clone())
            .oneshot(request())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let result: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let job: rip_core::Job = serde_json::from_value(result["job"].clone()).unwrap();
        assert_eq!(job.manifest.priority, resolved);
        let replay = jrip_server::router(app.clone())
            .oneshot(request())
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::OK);
        let replay: Value =
            serde_json::from_slice(&replay.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(replay["job"]["id"], job.id.to_string());
        jobs.push(job);
    }
    assert_ne!(hashes[0], hashes[1]);
    assert_eq!(app.repo.list(100, 0).await.unwrap().len(), 5);
    let reopened = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    for job in &jobs {
        assert_eq!(
            reopened.repo.get(job.id).await.unwrap().manifest.priority,
            job.manifest.priority
        );
    }
    assert_eq!(
        reopened.repo.next_queued().await.unwrap().unwrap().id,
        jobs[1].id
    );
    reopened
        .repo
        .transition(jobs[1].clone(), rip_core::JobState::Held, None)
        .await
        .unwrap();
    assert_eq!(
        reopened.repo.next_queued().await.unwrap().unwrap().id,
        jobs[2].id
    );
    let capability = jrip_server::print::models(&reopened, None).await.unwrap();
    let rip = capability["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["capability"]["stage"] == "rip")
        .unwrap();
    assert_eq!(
        rip["capability"]["extensions"]["org.jrip.scheduling"]["agingIntervalSeconds"],
        60
    );
}
