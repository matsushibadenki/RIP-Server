use rip_core::{ColorMode, DocumentFormat, EnginePreference, Job, JobState, Manifest};
use rip_storage::{Repository, now};
use uuid::Uuid;

#[cfg(unix)]
#[tokio::test]
async fn dispatcher_only_starts_queued_jobs_and_stops_if_worker_does_not_claim() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("jobs")).unwrap();
    let repo = Repository::open(&dir.path().join("jobs.sqlite3"))
        .await
        .unwrap();
    let job = Job {
        id: Uuid::new_v4(),
        manifest: Manifest {
            name: "proof.pdf".into(),
            format: DocumentFormat::Pdf,
            dpi: 600,
            color_mode: ColorMode::Cmyk,
            priority: 50,
            engine: EnginePreference::Auto,
        },
        state: JobState::Held,
        source_sha256: "hash".into(),
        created_at: now(),
        updated_at: now(),
        revision: 0,
        error_code: None,
        selected_engine: None,
        worker_run_id: None,
    };
    repo.insert(&job).await.unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.path().join("dispatcher.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let run = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_jrip-dispatcher"))
            .arg("--once")
            .env("JRIP_DATA_ROOT", dir.path())
            .env("JRIP_WORKER_BIN", "/usr/bin/true")
            .output()
            .unwrap()
    };
    assert!(!run().status.success());
    drop(lock);
    assert!(run().status.success());
    let queued = repo.transition(job, JobState::Queued, None).await.unwrap();
    let output = run();
    assert!(!output.status.success());
    assert_eq!(repo.get(queued.id).await.unwrap().state, JobState::Queued);
}

#[tokio::test]
async fn dispatcher_only_fails_its_own_exited_worker() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::open(&dir.path().join("jobs.sqlite3"))
        .await
        .unwrap();
    let own_run = Uuid::new_v4();
    let other_run = Uuid::new_v4();
    let job = Job {
        id: Uuid::new_v4(),
        manifest: Manifest {
            name: "proof.pdf".into(),
            format: DocumentFormat::Pdf,
            dpi: 600,
            color_mode: ColorMode::Cmyk,
            priority: 50,
            engine: EnginePreference::Auto,
        },
        state: JobState::Queued,
        source_sha256: "hash".into(),
        created_at: now(),
        updated_at: now(),
        revision: 0,
        error_code: None,
        selected_engine: None,
        worker_run_id: None,
    };
    repo.insert(&job).await.unwrap();
    let mut claimed = job;
    claimed.worker_run_id = Some(own_run);
    let ripping = repo
        .transition(claimed, JobState::Ripping, None)
        .await
        .unwrap();
    assert_eq!(
        jrip_server::dispatch::reconcile_worker_exit(&repo, ripping.id, other_run)
            .await
            .unwrap()
            .state,
        JobState::Ripping
    );
    let failed = jrip_server::dispatch::reconcile_worker_exit(&repo, ripping.id, own_run)
        .await
        .unwrap();
    assert_eq!(failed.state, JobState::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("WORKER_EXIT"));
}
