//! Administrator-configured filesystem vault; no remote-storage durability claim.
use super::*;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
type Error = Box<dyn std::error::Error + Send + Sync>;

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn matches_file(path: &Path, bytes: &[u8]) -> Result<bool, Error> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file() || metadata.len() != bytes.len() as u64 {
        return Ok(false);
    }
    let mut actual = Vec::new();
    std::fs::File::open(path)?
        .take(bytes.len() as u64 + 1)
        .read_to_end(&mut actual)?;
    Ok(actual == bytes)
}

fn publish(root: &Path, id: Uuid, bytes: &[u8]) -> Result<(), Error> {
    let destination = root.join(format!("{id}.json"));
    if std::fs::symlink_metadata(&destination)
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        let staging = Staging(root.join(format!(".jrip-audit-{id}-{}.tmp", Uuid::new_v4())));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&staging.0)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        match std::fs::hard_link(&staging.0, &destination) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        drop(staging);
    }
    if !matches_file(&destination, bytes)? {
        return Err("vault archive conflicts with stored snapshot".into());
    }
    // Also sync on replay: recover publication whose first directory sync failed.
    std::fs::File::open(&destination)?.sync_all()?;
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}

impl Repository {
    /// Publish exact stored bytes; record acknowledgement only after filesystem verification/sync.
    pub async fn store_audit_archive(&self, id: Uuid, directory: &Path) -> Result<Value, Error> {
        let archive = self.audit_archive(id).await?;
        if !audit_archive::verify(&archive) || archive["payload"]["id"] != id.to_string() {
            return Err("stored archive integrity invalid".into());
        }
        let bytes = serde_json::to_vec(&archive)?;
        if bytes.len() > audit_archive::MAX_BYTES {
            return Err("stored archive exceeds size limit".into());
        }
        let root = std::fs::canonicalize(directory)?;
        if !root.is_dir() {
            return Err("existing administrator vault directory required".into());
        }
        let target = root.to_str().ok_or("vault path must be UTF-8")?.to_owned();
        let file_digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        let root_copy = root.clone();
        let bytes_copy = bytes.clone();
        tokio::task::spawn_blocking(move || publish(&root_copy, id, &bytes_copy)).await??;
        let time = now();
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query("INSERT OR IGNORE INTO audit_archive_exports(archive_id,target,digest,bytes,verified_at) VALUES(?,?,?,?,?)")
            .bind(id.to_string()).bind(&target).bind(&file_digest).bind(bytes.len() as i64).bind(time)
            .execute(&mut *tx).await?.rows_affected() != 0;
        if inserted {
            sqlx::query("INSERT INTO audit_records(correlation_id,identity,operation,target,started_at,finished_at,status) VALUES(?,?,?,?,?,?,200)")
                .bind(Uuid::new_v4().to_string()).bind("local-audit-vault").bind("audit-vault-store")
                .bind(id.to_string()).bind(time).bind(time).execute(&mut *tx).await?;
        }
        let recorded: i64 = sqlx::query_scalar(
            "SELECT verified_at FROM audit_archive_exports WHERE archive_id=? AND target=?",
        )
        .bind(id.to_string())
        .bind(&target)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(
            json!({"archiveId":id,"target":target,"fileDigest":file_digest,"bytes":bytes.len(),
            "verifiedAt":recorded,"replayed":!inserted,"storageMeaning":"filesystem-sync","worm":false}),
        )
    }

    pub async fn audit_vault_status(&self, id: Uuid, directory: &Path) -> Result<Value, Error> {
        let archive = self.audit_archive(id).await?;
        if !audit_archive::verify(&archive) || archive["payload"]["id"] != id.to_string() {
            return Err("stored archive integrity invalid".into());
        }
        let bytes = serde_json::to_vec(&archive)?;
        if bytes.len() > audit_archive::MAX_BYTES {
            return Err("stored archive exceeds size limit".into());
        }
        let root = std::fs::canonicalize(directory)?;
        let target = root.to_str().ok_or("vault path must be UTF-8")?.to_owned();
        let recorded: Option<i64> = sqlx::query_scalar(
            "SELECT verified_at FROM audit_archive_exports WHERE archive_id=? AND target=?",
        )
        .bind(id.to_string())
        .bind(&target)
        .fetch_optional(&self.pool)
        .await?;
        let verified = tokio::task::spawn_blocking(move || {
            matches_file(&root.join(format!("{id}.json")), &bytes)
        })
        .await??;
        Ok(
            json!({"archiveId":id,"target":target,"acknowledged":recorded.is_some(),"verifiedAt":recorded,
            "contentMatches":verified,"storageMeaning":"filesystem-sync","worm":false}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn replay_recovery_and_tampering_preserve_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db");
        let root = temp.path().join("vault");
        std::fs::create_dir(&root).unwrap();
        let repo = Repository::open_with_receipt_signer(&db, None)
            .await
            .unwrap();
        let seq = repo
            .begin_audit(Uuid::new_v4(), "operator", "test", None)
            .await
            .unwrap();
        repo.finish_audit(seq, 200).await.unwrap();
        let archive = repo.archive_audit(0, now() + 1, 100).await.unwrap();
        let id = Uuid::parse_str(archive["payload"]["id"].as_str().unwrap()).unwrap();
        // A file published before a crash/DB acknowledgement is recoverable.
        let path = root.join(format!("{id}.json"));
        std::fs::write(&path, serde_json::to_vec(&archive).unwrap()).unwrap();
        assert_eq!(
            repo.audit_vault_status(id, &root).await.unwrap()["acknowledged"],
            false
        );
        let first = repo.store_audit_archive(id, &root).await.unwrap();
        assert_eq!(first["replayed"], false);
        let reopened = Repository::open_with_receipt_signer(&db, None)
            .await
            .unwrap();
        let replay = reopened.store_audit_archive(id, &root).await.unwrap();
        assert_eq!(replay["replayed"], true);
        assert_eq!(first["verifiedAt"], replay["verifiedAt"]);
        assert_eq!(
            reopened.audit_vault_status(id, &root).await.unwrap()["contentMatches"],
            true
        );
        std::fs::write(&path, b"tampered").unwrap();
        assert!(reopened.store_audit_archive(id, &root).await.is_err());
        let status = reopened.audit_vault_status(id, &root).await.unwrap();
        assert_eq!(status["acknowledged"], true);
        assert_eq!(status["contentMatches"], false);
        assert_eq!(std::fs::read(&path).unwrap(), b"tampered");
        assert_eq!(reopened.audit_archive(id).await.unwrap(), archive);
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            reopened.audit_vault_status(id, &root).await.unwrap()["contentMatches"],
            false
        );
        assert_eq!(
            reopened.store_audit_archive(id, &root).await.unwrap()["replayed"],
            true
        );
        assert_eq!(
            reopened.audit_vault_status(id, &root).await.unwrap()["contentMatches"],
            true
        );
        let records = reopened.audit_records(0, 100).await.unwrap();
        assert_eq!(
            records["records"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["operation"] == "audit-vault-store")
                .count(),
            1
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn conflicting_symlink_never_acknowledges_or_overwrites() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open_with_receipt_signer(&temp.path().join("db"), None)
            .await
            .unwrap();
        let sequence = repo
            .begin_audit(Uuid::new_v4(), "operator", "test", None)
            .await
            .unwrap();
        repo.finish_audit(sequence, 200).await.unwrap();
        let archive = repo.archive_audit(0, now() + 1, 100).await.unwrap();
        let id = Uuid::parse_str(archive["payload"]["id"].as_str().unwrap()).unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let original = temp.path().join("original.json");
        std::fs::write(&original, serde_json::to_vec(&archive).unwrap()).unwrap();
        std::os::unix::fs::symlink(&original, vault.join(format!("{id}.json"))).unwrap();
        assert!(repo.store_audit_archive(id, &vault).await.is_err());
        let status = repo.audit_vault_status(id, &vault).await.unwrap();
        assert_eq!(status["acknowledged"], false);
        assert_eq!(status["contentMatches"], false);
        assert_eq!(
            std::fs::read(&original).unwrap(),
            serde_json::to_vec(&archive).unwrap()
        );
    }
}
