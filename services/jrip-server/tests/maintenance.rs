use rip_core::{Job, JobState};
use rip_storage::delivery::{DeliveryState, DeliveryTicket};
use uuid::Uuid;
async fn seed(app: &jrip_server::App, state: JobState, age: i64) -> Job {
    let job = Job {
        id: Uuid::new_v4(),
        manifest: serde_json::from_value(
            serde_json::json!({"name":"test.pdf","format":"application/pdf","dpi":300}),
        )
        .unwrap(),
        state,
        source_sha256: "a".repeat(64),
        created_at: rip_storage::now() - age,
        updated_at: rip_storage::now() - age,
        revision: 0,
        error_code: None,
        selected_engine: None,
        worker_run_id: None,
    };
    app.repo.insert(&job).await.unwrap();
    std::fs::create_dir_all(app.root.join("jobs").join(job.id.to_string())).unwrap();
    std::fs::write(
        app.root
            .join("jobs")
            .join(job.id.to_string())
            .join("document"),
        b"%PDF-1.4",
    )
    .unwrap();
    job
}
fn ticket(job: &Job) -> DeliveryTicket {
    DeliveryTicket {
        source_job: job.id,
        color_attempt_id: None,
        printer_id: "test".into(),
        artifact_sha256: "b".repeat(64),
        artifact_bytes: 100,
        dpi: 300,
        document_type: "rgb_8".into(),
        media: "iso_a4_210x297mm".into(),
    }
}
#[tokio::test]
async fn retention_is_opt_in_preserves_active_jobs_and_resumes_interrupted_purges() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), String::new())
        .await
        .unwrap();
    let expired = seed(&app, JobState::Completed, 3 * 86400).await;
    let recent_delivery_source = seed(&app, JobState::Completed, 3 * 86400).await;
    let finished = app
        .repo
        .enqueue_delivery(Uuid::new_v4(), ticket(&recent_delivery_source))
        .await
        .unwrap();
    let run = Uuid::new_v4();
    app.repo.claim_delivery(finished.id, 0, run).await.unwrap();
    let accepted = app
        .repo
        .finish_delivery(finished.id, run, DeliveryState::Accepted, Some(42), None)
        .await
        .unwrap();
    app.repo
        .observe_delivery(finished.id, accepted.revision, DeliveryState::Completed)
        .await
        .unwrap();
    let delivery_dir = app.root.join("deliveries");
    std::fs::create_dir(&delivery_dir).unwrap();
    for suffix in ["pwg", "json"] {
        std::fs::write(
            delivery_dir.join(format!("{}.{}", finished.id, suffix)),
            b"expired",
        )
        .unwrap();
    }
    let fresh = seed(&app, JobState::Completed, 0).await;
    let queued = seed(&app, JobState::Queued, 3 * 86400).await;
    let active = seed(&app, JobState::Completed, 3 * 86400).await;
    let delivery = app
        .repo
        .enqueue_delivery(Uuid::new_v4(), ticket(&active))
        .await
        .unwrap();
    let run = Uuid::new_v4();
    app.repo.claim_delivery(delivery.id, 0, run).await.unwrap();
    app.repo
        .finish_delivery(
            delivery.id,
            run,
            DeliveryState::Unknown,
            None,
            Some("OUTCOME_UNKNOWN".into()),
        )
        .await
        .unwrap();
    let cutoff = rip_storage::now() - 86400;
    let dry = jrip_server::maintenance::run(&app, cutoff, false)
        .await
        .unwrap();
    assert_eq!(dry["candidateJobs"], serde_json::json!([expired.id]));
    assert!(app.repo.get(expired.id).await.is_ok());
    app.repo.claim_retention(expired.id, cutoff).await.unwrap();
    // Process restart between database claim and file removal.
    let reopened = jrip_server::open(dir.path().into(), String::new())
        .await
        .unwrap();
    let applied = jrip_server::maintenance::run(&reopened, cutoff, true)
        .await
        .unwrap();
    assert_eq!(applied["purgedJobs"], serde_json::json!([expired.id]));
    assert!(!app.root.join("jobs").join(expired.id.to_string()).exists());
    assert!(reopened.repo.events(expired.id, 0, 100).await.is_ok());
    for job in [fresh, queued, active, recent_delivery_source] {
        assert!(app.repo.get(job.id).await.is_ok());
        assert!(app.root.join("jobs").join(job.id.to_string()).exists());
    }
    // The source is old, but the derived delivery spool was just completed.
    assert!(delivery_dir.join(format!("{}.pwg", finished.id)).exists());
    assert!(app.repo.delivery(finished.id).await.is_ok());
    assert!(app.repo.pending_retention().await.unwrap().is_empty());
    assert!(
        jrip_server::maintenance::run(&app, rip_storage::now(), true)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn expired_admissions_are_pruned_without_deleting_journal_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), String::new())
        .await
        .unwrap();
    let preflight = Uuid::new_v4();
    app.repo
        .save_preflight(preflight, &serde_json::json!({}), rip_storage::now() - 10)
        .await
        .unwrap();
    let approval = Uuid::new_v4();
    app.repo
        .save_approval(approval, &serde_json::json!({}), rip_storage::now() - 10)
        .await
        .unwrap();
    let dry = app
        .repo
        .prune_expired_admissions(rip_storage::now(), false)
        .await
        .unwrap();
    assert_eq!(dry["expiredPreflights"], 1);
    let applied = app
        .repo
        .prune_expired_admissions(rip_storage::now(), true)
        .await
        .unwrap();
    assert_eq!(applied["expiredApprovals"], 1);
    assert_eq!(
        app.repo
            .prune_expired_admissions(rip_storage::now(), false)
            .await
            .unwrap()["expiredPreflights"],
        0
    );
}
#[cfg(unix)]
#[tokio::test]
async fn retention_unlinks_job_symlink_without_touching_external_data() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), String::new())
        .await
        .unwrap();
    let job = seed(&app, JobState::Cancelled, 3 * 86400).await;
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep"), b"keep").unwrap();
    let path = app.root.join("jobs").join(job.id.to_string());
    std::fs::remove_dir_all(&path).unwrap();
    std::os::unix::fs::symlink(outside.path(), &path).unwrap();
    jrip_server::maintenance::run(&app, rip_storage::now() - 86400, true)
        .await
        .unwrap();
    assert_eq!(std::fs::read(outside.path().join("keep")).unwrap(), b"keep");
}

#[tokio::test]
async fn deleted_metadata_orphans_require_known_terminal_history() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), String::new())
        .await
        .unwrap();
    let job = seed(&app, JobState::Cancelled, 3 * 86400).await;
    app.repo.delete(job.clone()).await.unwrap();
    let anonymous = app.root.join("jobs").join(Uuid::new_v4().to_string());
    std::fs::create_dir(&anonymous).unwrap();
    std::fs::write(anonymous.join("source.part"), b"upload").unwrap();
    let cutoff = rip_storage::now() - 86400;
    assert_eq!(
        app.repo.orphan_retention_candidates(cutoff).await.unwrap(),
        vec![job.id]
    );
    jrip_server::maintenance::run(&app, cutoff, true)
        .await
        .unwrap();
    assert!(!app.root.join("jobs").join(job.id.to_string()).exists());
    assert!(anonymous.exists());
    assert!(
        app.repo
            .orphan_retention_candidates(cutoff)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn snapshot_cleanup_skips_active_locks_and_unrelated_files() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), String::new())
        .await
        .unwrap();
    let snapshots = app.root.join("artifact-snapshots");
    std::fs::create_dir(&snapshots).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    let active = snapshots.join("artifact-snapshot-active.part");
    let locked = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&active)
        .unwrap();
    locked.try_lock().unwrap();
    locked
        .set_times(std::fs::FileTimes::new().set_modified(old))
        .unwrap();
    let abandoned = snapshots.join("artifact-snapshot-abandoned.part");
    let file = std::fs::File::create(&abandoned).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(old))
        .unwrap();
    drop(file);
    let unrelated = snapshots.join("keep.pdf");
    std::fs::write(&unrelated, b"keep").unwrap();
    let cutoff = rip_storage::now() - 86400;
    assert_eq!(
        jrip_server::maintenance::run(&app, cutoff, false)
            .await
            .unwrap()["abandonedSnapshots"],
        1
    );
    assert!(abandoned.exists());
    assert_eq!(
        jrip_server::maintenance::run(&app, cutoff, true)
            .await
            .unwrap()["abandonedSnapshots"],
        1
    );
    assert!(!abandoned.exists());
    assert!(active.exists());
    assert!(unrelated.exists());
    drop(locked);
}

#[tokio::test]
async fn abandoned_color_stages_are_cleaned_only_when_writer_lock_is_free() {
    let root = tempfile::tempdir().unwrap();
    let app = jrip_server::open(root.path().into(), String::new())
        .await
        .unwrap();
    let directory = app.root.join("color-staging");
    std::fs::create_dir(&directory).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    let active = directory.join("color-stage-active");
    let abandoned = directory.join("color-stage-abandoned");
    for path in [&active, &abandoned] {
        std::fs::create_dir(path).unwrap();
        std::fs::write(path.join("writer.lock"), b"").unwrap();
        std::fs::write(path.join("source.tiff"), b"private").unwrap();
        std::fs::File::open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
    }
    let writer = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(active.join("writer.lock"))
        .unwrap();
    writer.try_lock().unwrap();
    let dry = jrip_server::maintenance::run(&app, rip_storage::now() - 86400, false)
        .await
        .unwrap();
    assert_eq!(dry["abandonedColorStages"], 1);
    assert!(abandoned.exists());
    jrip_server::maintenance::run(&app, rip_storage::now() - 86400, true)
        .await
        .unwrap();
    assert!(active.exists());
    assert!(!abandoned.exists());
    drop(writer);
    jrip_server::maintenance::run(&app, rip_storage::now() - 86400, true)
        .await
        .unwrap();
    assert!(!active.exists());
}

#[tokio::test]
async fn orphan_upload_cleanup_protects_live_legacy_registered_and_recent_sources() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), String::new())
        .await
        .unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86400);
    let make = |marked: bool, old_directory: bool| {
        let id = Uuid::new_v4();
        let path = app.root.join("jobs").join(id.to_string());
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("source.part"), b"%PDF-1.4 partial").unwrap();
        if marked {
            let guard = rip_jobs::upload_guard(&path).unwrap();
            drop(guard);
        }
        if old_directory {
            std::fs::File::open(&path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(old))
                .unwrap();
        }
        (id, path)
    };
    let (abandoned, abandoned_path) = make(true, true);
    let (active, active_path) = make(true, true);
    let (_legacy, legacy_path) = make(false, true);
    let (_recent, recent_path) = make(true, false);
    let writer = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(active_path.join("upload-writer.lock"))
        .unwrap();
    writer.lock().unwrap();
    let registered = seed(&app, JobState::Queued, 3 * 86400).await;
    let path = app.root.join("jobs").join(registered.id.to_string());
    drop(rip_jobs::upload_guard(&path).unwrap());
    std::fs::File::open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old))
        .unwrap();
    #[cfg(unix)]
    {
        let linked = app.root.join("jobs").join(Uuid::new_v4().to_string());
        std::os::unix::fs::symlink(&abandoned_path, linked).unwrap();
    }
    let cutoff = rip_storage::now() - 86400;
    let dry = jrip_server::maintenance::run(&app, cutoff, false)
        .await
        .unwrap();
    assert_eq!(dry["abandonedUploads"], serde_json::json!([abandoned]));
    assert!(abandoned_path.exists());
    let applied = jrip_server::maintenance::run(&app, cutoff, true)
        .await
        .unwrap();
    assert_eq!(applied["abandonedUploads"], serde_json::json!([abandoned]));
    assert!(!abandoned_path.exists());
    for path in [&active_path, &legacy_path, &recent_path, &path] {
        assert!(path.exists());
    }
    drop(writer);
    let next = jrip_server::maintenance::run(&app, cutoff, true)
        .await
        .unwrap();
    assert_eq!(next["abandonedUploads"], serde_json::json!([active]));
    assert!(!active_path.exists());
    assert!(
        jrip_server::maintenance::run(&app, cutoff, true)
            .await
            .unwrap()["abandonedUploads"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn orphan_upload_claim_resumes_even_after_marker_removal() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), String::new())
        .await
        .unwrap();
    let id = Uuid::new_v4();
    let path = app.root.join("jobs").join(id.to_string());
    std::fs::create_dir(&path).unwrap();
    drop(rip_jobs::upload_guard(&path).unwrap());
    std::fs::write(path.join("source.part"), b"partial").unwrap();
    app.repo.claim_upload_retention(id).await.unwrap();
    assert!(app.repo.claim_upload_retention(id).await.is_err());
    std::fs::remove_file(path.join("upload-writer.lock")).unwrap();
    let reopened = jrip_server::open(app.root.clone(), String::new())
        .await
        .unwrap();
    let result = jrip_server::maintenance::run(&reopened, rip_storage::now() - 86400, true)
        .await
        .unwrap();
    assert_eq!(result["purgedJobs"], serde_json::json!([id]));
    assert!(!path.exists());
    assert!(reopened.repo.pending_retention().await.unwrap().is_empty());
    let registered = seed(&reopened, JobState::Completed, 3 * 86400).await;
    assert!(
        reopened
            .repo
            .claim_upload_retention(registered.id)
            .await
            .is_err()
    );
    reopened.repo.delete(registered.clone()).await.unwrap();
    // Historical registration remains protected even after metadata deletion.
    assert!(
        reopened
            .repo
            .claim_upload_retention(registered.id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn explicit_family_retention_is_scoped_revisioned_and_preserves_orphan_files() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    let job = seed(&app, JobState::Completed, 3 * 86400).await;
    let color_dir = app
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("color/example");
    std::fs::create_dir_all(&color_dir).unwrap();
    std::fs::write(color_dir.join("page.tiff"), b"retained").unwrap();
    app.credentials = vec![
        jrip_server::Credential {
            validity: None,
            token: "reader".into(),
            identity: "reader".into(),
            scopes: vec!["jobs:read".into()],
        },
        jrip_server::Credential {
            validity: None,
            token: "writer".into(),
            identity: "operator".into(),
            scopes: vec!["settings:write".into()],
        },
    ];
    let path = format!("/api/v1/jobs/{}/retention", job.id);
    let put = |token: &str, revision: i64, until: i64| {
        Request::put(&path).header("authorization",format!("Bearer {token}")).header("content-type","application/json").body(Body::from(json!({"expectedRevision":revision,"retainUntil":until,"reason":"keep proof artifacts"}).to_string())).unwrap()
    };
    let cutoff = rip_storage::now() - 86400;
    let until = rip_storage::now() + 86400;
    assert_eq!(
        jrip_server::router(app.clone())
            .oneshot(put("reader", 0, until))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let response = jrip_server::router(app.clone())
        .oneshot(put("writer", 0, until))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let policy: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(policy["revision"], 1);
    assert_eq!(policy["actor"], "operator");
    assert_eq!(
        jrip_server::router(app.clone())
            .oneshot(put("writer", 0, until + 10))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        jrip_server::router(app.clone())
            .oneshot(put("writer", 1, until - 10))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        jrip_server::router(app.clone())
            .oneshot(put("writer", 1, 0))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(
        app.repo
            .retention_candidates(cutoff)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(app.repo.claim_retention(job.id, cutoff).await.is_err());
    // Metadata deletion is not physical artifact deletion; orphan maintenance obeys the hold.
    app.repo.delete(job.clone()).await.unwrap();
    let mut reopened = jrip_server::open(dir.path().into(), "token".into())
        .await
        .unwrap();
    reopened.credentials = app.credentials.clone();
    assert_eq!(
        reopened
            .repo
            .artifact_retention_policy(job.id)
            .await
            .unwrap(),
        policy
    );
    let preview = jrip_server::maintenance::run(&reopened, cutoff, false)
        .await
        .unwrap();
    assert!(preview["candidateJobs"].as_array().unwrap().is_empty());
    let result = jrip_server::maintenance::run(&reopened, cutoff, true)
        .await
        .unwrap();
    assert!(result["purgedJobs"].as_array().unwrap().is_empty());
    assert!(color_dir.join("page.tiff").exists());
    let response = jrip_server::router(reopened)
        .oneshot(
            Request::get(path)
                .header("authorization", "Bearer reader")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let read: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(read, policy);
}
