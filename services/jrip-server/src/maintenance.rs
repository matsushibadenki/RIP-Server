use super::*;
pub async fn run(
    app: &App,
    cutoff: i64,
    apply: bool,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if cutoff < 0 || cutoff > rip_storage::now() - 86400 {
        return Err("retention cutoff must be at least one day old".into());
    }
    let mut candidates = app.repo.retention_candidates(cutoff).await?;
    candidates.extend(app.repo.orphan_retention_candidates(cutoff).await?);
    let pending = app.repo.pending_retention().await?;
    let abandoned_snapshots = sweep_snapshots(&app.root, false)?;
    let abandoned_color_stages = sweep_color_stages(&app.root, false)?;
    let abandoned_uploads = sweep_uploads(app, cutoff, false).await?;
    let admissions = app
        .repo
        .prune_expired_admissions(rip_storage::now(), false)
        .await?;
    if !apply {
        return Ok(
            serde_json::json!({"artifactRetentionProfile":"org.jrip.artifact-family-retention-v1","ageBasis":"latest-related-stage-activity","candidateJobs":candidates,"pendingPurges":pending,"admissions":admissions,"abandonedSnapshots":abandoned_snapshots,"abandonedColorStages":abandoned_color_stages,"abandonedUploads":abandoned_uploads,"applied":false}),
        );
    }
    let sequence = app
        .repo
        .begin_audit(Uuid::new_v4(), "local-maintenance", "retention", None)
        .await?;
    let result = async {
        let abandoned_snapshots=sweep_snapshots(&app.root,true)?;
        let abandoned_color_stages=sweep_color_stages(&app.root,true)?;
        let abandoned_uploads=sweep_uploads(app,cutoff,true).await?;
        let admissions = app
            .repo
            .prune_expired_admissions(rip_storage::now(), true)
            .await?;
        for id in &candidates {
            match app.repo.claim_retention(*id, cutoff).await {
                Ok(()) | Err(StorageError::Conflict) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let trash = app.root.join("retention-trash");
        tokio::fs::create_dir_all(&trash).await?;
        if tokio::fs::canonicalize(&trash).await? != trash {
            return Err("invalid trash directory".into());
        }
        let jobs = app.root.join("jobs");
        if tokio::fs::canonicalize(&jobs).await? != jobs {
            return Err("invalid jobs directory".into());
        }
        let mut purged = vec![];
        for id in app.repo.pending_retention().await? {
            let source = jobs.join(id.to_string());
            let target = trash.join(id.to_string());
            match tokio::fs::symlink_metadata(&source).await {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    tokio::fs::remove_file(&source).await?;
                }
                Ok(_) => tokio::fs::rename(&source, &target).await?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            match tokio::fs::symlink_metadata(&target).await {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    tokio::fs::remove_file(&target).await?
                }
                Ok(metadata) if metadata.is_dir() => tokio::fs::remove_dir_all(&target).await?,
                Ok(_) => tokio::fs::remove_file(&target).await?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let deliveries = app.root.join("deliveries");
            if tokio::fs::try_exists(&deliveries).await? {
                if tokio::fs::canonicalize(&deliveries).await? != deliveries {
                    return Err("invalid delivery directory".into());
                }
                for delivery in app.repo.terminal_deliveries_for_job(id).await? {
                    for suffix in ["pwg", "json"] {
                        match tokio::fs::remove_file(
                            deliveries.join(format!("{delivery}.{suffix}")),
                        )
                        .await
                        {
                            Ok(()) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) => return Err(error.into()),
                        }
                    }
                }
            }
            app.repo.finish_retention(id).await?;
            purged.push(id);
        }
        Ok::<_, Box<dyn std::error::Error>>(
            serde_json::json!({"artifactRetentionProfile":"org.jrip.artifact-family-retention-v1","ageBasis":"latest-related-stage-activity","purgedJobs":purged,"admissions":admissions,"abandonedSnapshots":abandoned_snapshots,"abandonedColorStages":abandoned_color_stages,"abandonedUploads":abandoned_uploads,"applied":true}),
        )
    }
    .await;
    app.repo
        .finish_audit(sequence, if result.is_ok() { 200 } else { 500 })
        .await?;
    result
}
async fn sweep_uploads(
    app: &App,
    cutoff: i64,
    apply: bool,
) -> Result<Vec<Uuid>, Box<dyn std::error::Error>> {
    let root = app.root.join("jobs");
    if std::fs::canonicalize(&root)? != root {
        return Err("invalid jobs directory".into());
    }
    let cutoff = std::time::UNIX_EPOCH + std::time::Duration::from_secs(cutoff as u64);
    let mut reclaimed = vec![];
    for entry in std::fs::read_dir(&root)?.take(4096) {
        let entry = entry?;
        let Ok(id) = Uuid::parse_str(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        let result = async {
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if !metadata.is_dir() || metadata.modified()? > cutoff {
                return Ok(false);
            }
            let marker = entry.path().join("upload-writer.lock");
            if !std::fs::symlink_metadata(&marker)?.is_file() {
                return Ok(false);
            }
            let writer = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(marker)?;
            match writer.try_lock() {
                Ok(()) => {}
                Err(std::fs::TryLockError::WouldBlock) => return Ok(false),
                Err(error) => return Err(error.into()),
            }
            if app.repo.upload_has_registration(id).await?
                || std::fs::symlink_metadata(entry.path())?.modified()? > cutoff
            {
                return Ok(false);
            }
            if apply {
                match app.repo.claim_upload_retention(id).await {
                    Ok(()) => {}
                    Err(StorageError::Conflict) => return Ok(false),
                    Err(error) => return Err(error.into()),
                }
            }
            Ok::<_, Box<dyn std::error::Error>>(true)
        }
        .await;
        match result {
            Ok(true) => reclaimed.push(id),
            Ok(false) => {}
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error),
        }
        if reclaimed.len() == 128 {
            break;
        }
    }
    Ok(reclaimed)
}

fn sweep_color_stages(
    root: &std::path::Path,
    apply: bool,
) -> Result<u64, Box<dyn std::error::Error>> {
    let directory = root.join("color-staging");
    if !directory.try_exists()? {
        return Ok(0);
    }
    if std::fs::canonicalize(&directory)? != directory {
        return Err("invalid color staging root".into());
    }
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let mut count = 0;
    for entry in std::fs::read_dir(&directory)?.take(128) {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("color-stage-")
            || !metadata.is_dir()
            || metadata.modified()? > cutoff
        {
            continue;
        }
        let path = entry.path().join("writer.lock");
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() {
            continue;
        }
        let writer = std::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .open(path)?;
        match writer.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => continue,
            Err(error) => return Err(error.into()),
        }
        if std::fs::symlink_metadata(entry.path())?.modified()? > cutoff {
            continue;
        }
        if apply {
            std::fs::remove_dir_all(entry.path())?;
        }
        count += 1;
    }
    Ok(count)
}

fn sweep_snapshots(root: &std::path::Path, apply: bool) -> Result<u64, Box<dyn std::error::Error>> {
    let directory = root.join("artifact-snapshots");
    if !directory.try_exists()? {
        return Ok(0);
    }
    if std::fs::canonicalize(&directory)? != directory {
        return Err("invalid snapshot directory".into());
    }
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let mut count = 0;
    for entry in std::fs::read_dir(&directory)?.take(128) {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("artifact-snapshot-") || !name.ends_with(".part") {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if !metadata.is_file() || metadata.modified()? > cutoff {
            continue;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(entry.path())?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => continue,
            Err(error) => return Err(error.into()),
        }
        // Recheck metadata after reservation; a live snapshot always owns this lock.
        if file.metadata()?.modified()? > cutoff {
            continue;
        }
        if apply {
            std::fs::remove_file(entry.path())?;
        }
        count += 1;
    }
    Ok(count)
}
