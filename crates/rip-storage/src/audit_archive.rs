//! Bounded, immutable audit snapshots. Live records are never pruned here.
use super::*;
use serde_json::{Value, json};

pub const PROFILE: &str = "org.jrip.audit-archive-v1";
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

impl Repository {
    /// Archive a contiguous completed prefix; never skip an unfinished operation.
    pub async fn archive_audit(
        &self,
        after: i64,
        before: i64,
        limit: i64,
    ) -> Result<Value, StorageError> {
        if after < 0 || before < 0 || !(1..=1000).contains(&limit) {
            return Err(StorageError::Conflict);
        }
        let mut tx = self.pool.begin().await?;
        let mut archive = prepare_snapshot(&mut tx, after, before, limit).await?;
        let id = archive["payload"]["id"]
            .as_str()
            .ok_or(StorageError::Conflict)?
            .to_string();
        if let Some(signer) = &self.receipt_signer {
            let key_id = format!("sha256:{:x}", Sha256::digest(signer.public_key()));
            let revoked: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM receipt_key_revocations WHERE key_id=?)",
            )
            .bind(key_id)
            .fetch_one(&mut *tx)
            .await?;
            if revoked != 0 {
                return Err(StorageError::ReceiptKeyRevoked);
            }
            archive["proof"] = json!({"signed":false});
            signer.sign_profile(&mut archive, receipts::AUDIT_PROFILE)?;
        }
        let bytes = serde_json::to_vec(&archive)?;
        if bytes.len() > MAX_BYTES || !verify(&archive) {
            return Err(StorageError::Conflict);
        }
        sqlx::query("INSERT INTO audit_archives(id,payload) VALUES(?,?)")
            .bind(id.to_string())
            .bind(String::from_utf8(bytes).map_err(|_| StorageError::Conflict)?)
            .execute(&mut *tx)
            .await?;
        let time = now();
        sqlx::query("INSERT INTO audit_records(correlation_id,identity,operation,target,started_at,finished_at,status) VALUES(?,?,?,?,?,?,200)")
            .bind(Uuid::new_v4().to_string()).bind("local-audit-archive").bind("audit-archive-create")
            .bind(id.to_string()).bind(time).bind(time).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(archive)
    }

    /// Metadata only, ordered by canonical UUID. Export acknowledgement is historical.
    pub async fn audit_archives(
        &self,
        after: &str,
        before: i64,
        limit: i64,
    ) -> Result<Value, StorageError> {
        if !(1..=100).contains(&limit)
            || before < 0
            || (!after.is_empty()
                && !Uuid::parse_str(after).is_ok_and(|id| id.to_string() == after))
        {
            return Err(StorageError::Conflict);
        }
        let rows = sqlx::query("SELECT id,json_extract(payload,'$.payload.createdAt') AS created_at,json_extract(payload,'$.payload.after') AS start_sequence,json_extract(payload,'$.payload.through') AS through_sequence,json_array_length(json_extract(payload,'$.payload.records')) AS record_count,length(CAST(payload AS BLOB)) AS bytes,json_extract(payload,'$.digest') AS digest,json_extract(payload,'$.signature.profile') AS signature_profile,(SELECT COUNT(*) FROM audit_archive_exports e WHERE e.archive_id=a.id) AS exports FROM audit_archives a WHERE id>? AND json_extract(payload,'$.payload.createdAt')<? ORDER BY id LIMIT ?")
            .bind(after).bind(before).bind(limit+1).fetch_all(&self.pool).await?;
        let has_more = rows.len() > limit as usize;
        let mut cursor = after.to_owned();
        let archives: Vec<Value> = rows.into_iter().take(limit as usize).map(|row| {
            cursor = row.get::<String,_>("id");
            json!({"archiveId":cursor,"createdAt":row.get::<i64,_>("created_at"),
                "after":row.get::<i64,_>("start_sequence"),"through":row.get::<i64,_>("through_sequence"),
                "recordCount":row.get::<i64,_>("record_count"),"bytes":row.get::<i64,_>("bytes"),
                "digest":row.get::<String,_>("digest"),"signatureProfile":row.get::<Option<String>,_>("signature_profile"),
                "acknowledgedVaultCount":row.get::<i64,_>("exports")})
        }).collect();
        Ok(
            json!({"archives":archives,"nextAfter":cursor,"hasMore":has_more,"before":before,
            "vaultAcknowledgementsAreHistorical":true}),
        )
    }

    pub async fn audit_archive(&self, id: Uuid) -> Result<Value, StorageError> {
        let payload: Option<String> =
            sqlx::query_scalar("SELECT payload FROM audit_archives WHERE id=?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;
        Ok(serde_json::from_str(
            &payload.ok_or(StorageError::NotFound)?,
        )?)
    }
}

/// Integrity only: authenticity requires an independently pinned digest.
pub fn verify(archive: &Value) -> bool {
    if archive["digestProfile"] != rip_digest::PROFILE || archive["payload"]["profile"] != PROFILE {
        return false;
    }
    let p = &archive["payload"];
    let Some(records) = p["records"].as_array() else {
        return false;
    };
    let (Some(mut sequence), Some(through), Some(before)) = (
        p["after"].as_i64(),
        p["through"].as_i64(),
        p["before"].as_i64(),
    ) else {
        return false;
    };
    if sequence < 0
        || before < 0
        || records.is_empty()
        || records.len() > 1000
        || p["id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .is_none()
    {
        return false;
    }
    for record in records {
        let (Some(next), Some(finished), Some(status)) = (
            record["sequence"].as_i64(),
            record["finishedAt"].as_i64(),
            record["status"].as_i64(),
        ) else {
            return false;
        };
        if next <= sequence || finished >= before || !(100..=599).contains(&status) {
            return false;
        }
        sequence = next;
    }
    sequence == through && rip_digest::digest(p).is_ok_and(|digest| archive["digest"] == digest)
}

/// Verify the archive contract and its distinct signature domain using a pinned key.
pub fn verify_signature(archive: &Value, trusted_key: &[u8]) -> bool {
    verify(archive)
        && matches!(
            archive["signature"]["profile"].as_str(),
            Some(receipts::AUDIT_PROFILE | receipts::KMS_AUDIT_PROFILE)
        )
        && receipts::verify_signed_record(archive, trusted_key)
}

impl Repository {
    pub async fn verify_audit_archive_current(
        &self,
        archive: &Value,
        trusted_key: &[u8],
    ) -> Result<bool, StorageError> {
        if !verify_signature(archive, trusted_key) {
            return Ok(false);
        }
        let key_id = format!("sha256:{:x}", Sha256::digest(trusted_key));
        let valid: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM receipt_public_keys k WHERE k.key_id=? AND NOT EXISTS(SELECT 1 FROM receipt_key_revocations r WHERE r.key_id=k.key_id))")
            .bind(key_id).fetch_one(&self.pool).await?;
        Ok(valid != 0)
    }
}

pub(crate) async fn prepare_snapshot(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    after: i64,
    before: i64,
    limit: i64,
) -> Result<Value, StorageError> {
    if after < 0 || before < 0 || !(1..=1000).contains(&limit) {
        return Err(StorageError::Conflict);
    }
    let rows =
        sqlx::query("SELECT * FROM audit_records WHERE sequence>? ORDER BY sequence LIMIT ?")
            .bind(after)
            .bind(limit)
            .fetch_all(&mut **tx)
            .await?;
    let mut cursor = after;
    let mut records = Vec::new();
    for row in rows {
        let finished: Option<i64> = row.get("finished_at");
        if !finished.is_some_and(|time| time < before) {
            break;
        }
        cursor = row.get("sequence");
        records.push(json!({"sequence":cursor,"correlationId":row.get::<String,_>("correlation_id"),
                "identity":row.get::<String,_>("identity"),"operation":row.get::<String,_>("operation"),
                "target":row.get::<Option<String>,_>("target"),"startedAt":row.get::<i64,_>("started_at"),
                "finishedAt":finished,"status":row.get::<Option<i64>,_>("status")}));
    }
    if records.is_empty() {
        return Err(StorageError::NotFound);
    }
    let id = Uuid::new_v4();
    let payload = json!({"profile":PROFILE,"id":id,"createdAt":now(),"after":after,
            "through":cursor,"before":before,"records":records});
    let digest = rip_digest::digest(&payload).map_err(|_| StorageError::Conflict)?;
    let archive = json!({"digestProfile":rip_digest::PROFILE,"digest":digest,"payload":payload});
    Ok(archive)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn archive_is_durable_bounded_and_does_not_skip_incomplete_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.db");
        let repo = Repository::open_with_receipt_signer(&path, None)
            .await
            .unwrap();
        let first = repo
            .begin_audit(Uuid::new_v4(), "operator", "submit", None)
            .await
            .unwrap();
        repo.finish_audit(first, 201).await.unwrap();
        let blocked = repo
            .begin_audit(Uuid::new_v4(), "operator", "pending", None)
            .await
            .unwrap();
        let last = repo
            .begin_audit(Uuid::new_v4(), "operator", "cancel", None)
            .await
            .unwrap();
        repo.finish_audit(last, 200).await.unwrap();
        assert!(repo.archive_audit(0, 0, 10).await.is_err());
        let archive = repo.archive_audit(0, now() + 1, 10).await.unwrap();
        assert!(verify(&archive));
        assert_eq!(archive["payload"]["through"], first);
        assert!(matches!(
            repo.archive_audit(first, now() + 1, 10).await,
            Err(StorageError::NotFound)
        ));
        let mut tampered = archive.clone();
        tampered["payload"]["records"][0]["identity"] = json!("changed");
        assert!(!verify(&tampered));
        let reopened = Repository::open_with_receipt_signer(&path, None)
            .await
            .unwrap();
        let id = Uuid::parse_str(archive["payload"]["id"].as_str().unwrap()).unwrap();
        assert_eq!(reopened.audit_archive(id).await.unwrap(), archive);
        reopened.finish_audit(blocked, 500).await.unwrap();
        let next = reopened.archive_audit(first, now() + 1, 1).await.unwrap();
        assert_eq!(next["payload"]["through"], blocked);
        assert!(
            reopened.audit_records(0, 100).await.unwrap()["records"]
                .as_array()
                .unwrap()
                .len()
                >= 5
        );
        assert!(reopened.archive_audit(0, now() + 1, 1001).await.is_err());
    }
    #[tokio::test]
    async fn oversized_archive_rolls_back_without_publishing_or_pruning() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::open_with_receipt_signer(&dir.path().join("db"), None)
            .await
            .unwrap();
        let seq = repo
            .begin_audit(Uuid::new_v4(), "operator", &"x".repeat(MAX_BYTES), None)
            .await
            .unwrap();
        repo.finish_audit(seq, 200).await.unwrap();
        assert!(matches!(
            repo.archive_audit(0, now() + 1, 1).await,
            Err(StorageError::Conflict)
        ));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_archives")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_records")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
    #[tokio::test]
    async fn signature_is_durable_domain_bound_and_revocation_blocks_publication() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let key = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap();
        let signer = receipts::ReceiptSigner::from_pkcs8(key.as_ref()).unwrap();
        let public = signer.public_key().to_vec();
        let repo = Repository::open_with_receipt_signer(&path, Some(signer))
            .await
            .unwrap();
        let seq = repo
            .begin_audit(Uuid::new_v4(), "operator", "submit", None)
            .await
            .unwrap();
        repo.finish_audit(seq, 201).await.unwrap();
        let archive = repo.archive_audit(0, now() + 1, 100).await.unwrap();
        assert!(verify_signature(&archive, &public));
        assert!(!receipts::verify(&archive, &public));
        assert!(
            repo.verify_audit_archive_current(&archive, &public)
                .await
                .unwrap()
        );
        let mut changed = archive.clone();
        changed["payload"]["records"][0]["identity"] = json!("tampered");
        changed["digest"] = json!(rip_digest::digest(&changed["payload"]).unwrap());
        assert!(verify(&changed));
        assert!(!verify_signature(&changed, &public));
        changed = archive.clone();
        changed["signature"]["profile"] = json!(receipts::PROFILE);
        assert!(!verify_signature(&changed, &public));
        let reopened = Repository::open_with_receipt_signer(&path, None)
            .await
            .unwrap();
        let id = Uuid::parse_str(archive["payload"]["id"].as_str().unwrap()).unwrap();
        assert_eq!(reopened.audit_archive(id).await.unwrap(), archive);
        let key_id = archive["signature"]["keyId"].as_str().unwrap();
        reopened
            .revoke_receipt_key(key_id, "compromised", "admin")
            .await
            .unwrap();
        assert!(
            !reopened
                .verify_audit_archive_current(&archive, &public)
                .await
                .unwrap()
        );
        assert!(verify_signature(&archive, &public)); // Offline cryptography does not check revocation.
        assert!(matches!(
            repo.archive_audit(0, now() + 1, 100).await,
            Err(StorageError::ReceiptKeyRevoked)
        ));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_archives")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(reopened.audit_archive(id).await.unwrap(), archive);
    }
    #[tokio::test]
    async fn catalog_pages_legacy_snapshots_filters_age_and_counts_acknowledgements() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db");
        let repo = Repository::open_with_receipt_signer(&db, None)
            .await
            .unwrap();
        let seq = repo
            .begin_audit(Uuid::new_v4(), "operator", "submit", None)
            .await
            .unwrap();
        repo.finish_audit(seq, 200).await.unwrap();
        let a = repo.archive_audit(0, now() + 1, 100).await.unwrap();
        let b = repo.archive_audit(0, now() + 1, 100).await.unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let id = Uuid::parse_str(a["payload"]["id"].as_str().unwrap()).unwrap();
        repo.store_audit_archive(id, &vault).await.unwrap();
        let reopened = Repository::open_with_receipt_signer(&db, None)
            .await
            .unwrap();
        let first = reopened.audit_archives("", now() + 1, 1).await.unwrap();
        assert_eq!(first["hasMore"], true);
        let cursor = first["nextAfter"].as_str().unwrap();
        let second = reopened.audit_archives(cursor, now() + 1, 1).await.unwrap();
        assert_eq!(second["hasMore"], false);
        assert_ne!(
            first["archives"][0]["archiveId"],
            second["archives"][0]["archiveId"]
        );
        let all = reopened.audit_archives("", now() + 1, 100).await.unwrap();
        let rows = all["archives"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let exported = rows
            .iter()
            .find(|r| r["archiveId"] == id.to_string())
            .unwrap();
        assert_eq!(exported["acknowledgedVaultCount"], 1);
        assert_eq!(
            exported["recordCount"],
            a["payload"]["records"].as_array().unwrap().len()
        );
        assert!(exported.get("records").is_none());
        assert_eq!(
            reopened.audit_archives("", 0, 100).await.unwrap()["archives"],
            json!([])
        );
        assert!(
            reopened
                .audit_archives("invalid", now() + 1, 100)
                .await
                .is_err()
        );
        assert!(reopened.audit_archives("", now() + 1, 101).await.is_err());
        assert!(reopened.audit_archives("", -1, 1).await.is_err());
        let b_id = Uuid::parse_str(b["payload"]["id"].as_str().unwrap()).unwrap();
        assert_eq!(reopened.audit_archive(b_id).await.unwrap(), b);
    }
}
