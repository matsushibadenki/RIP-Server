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
        let rows =
            sqlx::query("SELECT * FROM audit_records WHERE sequence>? ORDER BY sequence LIMIT ?")
                .bind(after)
                .bind(limit)
                .fetch_all(&mut *tx)
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
        let archive =
            json!({"digestProfile":rip_digest::PROFILE,"digest":digest,"payload":payload});
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
}
