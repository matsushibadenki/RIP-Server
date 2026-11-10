#[tokio::test]
async fn concurrent_schema_initialization_and_reopen_preserve_shared_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.sqlite3");
    let writer = rip_storage::Repository::open(&path).await.unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let finished = stop.clone();
    let producer = tokio::spawn(async move {
        let mut count = 0;
        while !finished.load(std::sync::atomic::Ordering::SeqCst) {
            writer
                .begin_printer_query("active", &"a".repeat(64))
                .await
                .unwrap();
            count += 1;
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        count
    });
    for _ in 0..3 {
        let mut tasks = Vec::new();
        for _ in 0..12 {
            let path = path.clone();
            tasks.push(tokio::spawn(async move {
                let repo = rip_storage::Repository::open(&path).await.unwrap();
                repo.configure_snapshot_budget(rip_storage::budget::DEFAULT_CAPACITY)
                    .await
                    .unwrap();
                repo
            }));
        }
        let mut opened = Vec::new();
        for task in tasks {
            opened.push(task.await.unwrap());
        }
        assert_eq!(opened.len(), 12);
        for repo in &opened {
            assert!(repo.next_queued().await.unwrap().is_none());
        }
        drop(opened);
    }
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(producer.await.unwrap() > 0);
    assert!(path.with_file_name("shared.sqlite3.init.lock").exists());
}
