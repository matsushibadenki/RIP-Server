//! One-use admission, serialized against revocation. Failed actions remain consumed.
use super::*;
use serde_json::{Value, json};
pub(crate) async fn install(pool: &SqlitePool) -> Result<(), StorageError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS operation_approvals(id TEXT PRIMARY KEY,payload TEXT NOT NULL,binding_digest TEXT NOT NULL,expires_at INTEGER NOT NULL,revoked INTEGER NOT NULL DEFAULT 0,consumed_at INTEGER)").execute(pool).await?;
    Ok(())
}
async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor: &str,
    operation: &str,
    id: Uuid,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO audit_records(correlation_id,identity,operation,target,started_at,finished_at,status) VALUES(?,?,?,?,?,?,200)")
        .bind(Uuid::new_v4().to_string()).bind(actor).bind(operation).bind(id.to_string()).bind(now()).bind(now()).execute(&mut **tx).await?;
    Ok(())
}
impl Repository {
    pub async fn issue_operation_approval(
        &self,
        binding: &Value,
        issuer: &str,
        reason: &str,
    ) -> Result<Value, StorageError> {
        let identity = binding["identity"].as_str().ok_or(StorageError::Conflict)?;
        if issuer.is_empty()
            || identity.is_empty()
            || issuer == identity
            || reason.trim().is_empty()
            || reason.len() > 1024
        {
            return Err(StorageError::Conflict);
        }
        let digest = rip_digest::digest(binding).map_err(|_| StorageError::Conflict)?;
        let id = Uuid::new_v4();
        let expires = now() + 300;
        let payload = json!({"profile":"org.jrip.operation-approval-v1","id":id,"binding":binding,"bindingDigest":digest,"issuer":issuer,"reason":reason,"expiresAt":expires,"singleUse":true});
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO operation_approvals(id,payload,binding_digest,expires_at) VALUES(?,?,?,?)",
        )
        .bind(id.to_string())
        .bind(payload.to_string())
        .bind(digest)
        .bind(expires)
        .execute(&mut *tx)
        .await?;
        audit(&mut tx, issuer, "operation-approval-issue", id).await?;
        tx.commit().await?;
        Ok(payload)
    }
    pub async fn consume_operation_approval(
        &self,
        id: Uuid,
        binding: &Value,
        actor: &str,
    ) -> Result<bool, StorageError> {
        if binding["identity"] != actor {
            return Ok(false);
        }
        let digest = rip_digest::digest(binding).map_err(|_| StorageError::Conflict)?;
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query("UPDATE operation_approvals SET consumed_at=? WHERE id=? AND binding_digest=? AND expires_at>? AND revoked=0 AND consumed_at IS NULL")
            .bind(now()).bind(id.to_string()).bind(digest).bind(now()).execute(&mut *tx).await?.rows_affected();
        if changed == 1 {
            audit(&mut tx, actor, "operation-approval-consume", id).await?;
        }
        tx.commit().await?;
        Ok(changed == 1)
    }
    pub async fn revoke_operation_approval(
        &self,
        id: Uuid,
        actor: &str,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query("UPDATE operation_approvals SET revoked=1 WHERE id=? AND consumed_at IS NULL AND revoked=0").bind(id.to_string()).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(StorageError::Conflict);
        }
        audit(&mut tx, actor, "operation-approval-revoke", id).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn receipt_key_approval_state(&self, id: &str) -> Result<Value, StorageError> {
        let row = sqlx::query("SELECT k.key_id,r.revoked_at,r.reason FROM receipt_public_keys k LEFT JOIN receipt_key_revocations r ON r.key_id=k.key_id WHERE k.key_id=?")
            .bind(id).fetch_optional(&self.pool).await?.ok_or(StorageError::NotFound)?;
        Ok(
            json!({"keyId":id,"revokedAt":row.get::<Option<i64>,_>("revoked_at"),"reason":row.get::<Option<String>,_>("reason")}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn repository() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::open_with_receipt_signer(&dir.path().join("db"), None)
            .await
            .unwrap();
        (dir, repo)
    }
    fn binding() -> Value {
        json!({"identity":"agent","operation":"job-delete","effect":{"revision":3}})
    }
    fn id(value: &Value) -> Uuid {
        Uuid::parse_str(value["id"].as_str().unwrap()).unwrap()
    }
    #[tokio::test]
    async fn binding_identity_expiry_revocation_and_restart_are_enforced() {
        let (dir, repo) = repository().await;
        let binding = binding();
        assert!(
            repo.issue_operation_approval(&binding, "agent", "self")
                .await
                .is_err()
        );
        let issued = repo
            .issue_operation_approval(&binding, "human", "reviewed")
            .await
            .unwrap();
        let approval = id(&issued);
        assert!(
            !repo
                .consume_operation_approval(approval, &binding, "other")
                .await
                .unwrap()
        );
        let mut changed = binding.clone();
        changed["effect"]["revision"] = json!(4);
        assert!(
            !repo
                .consume_operation_approval(approval, &changed, "agent")
                .await
                .unwrap()
        );
        let reopened = Repository::open_with_receipt_signer(&dir.path().join("db"), None)
            .await
            .unwrap();
        assert!(
            reopened
                .consume_operation_approval(approval, &binding, "agent")
                .await
                .unwrap()
        );
        assert!(
            !repo
                .consume_operation_approval(approval, &binding, "agent")
                .await
                .unwrap()
        );
        assert!(
            repo.revoke_operation_approval(approval, "human")
                .await
                .is_err()
        );
        let revoked = id(&repo
            .issue_operation_approval(&binding, "human", "reviewed")
            .await
            .unwrap());
        repo.revoke_operation_approval(revoked, "human")
            .await
            .unwrap();
        assert!(
            !repo
                .consume_operation_approval(revoked, &binding, "agent")
                .await
                .unwrap()
        );
        let expired = id(&repo
            .issue_operation_approval(&binding, "human", "reviewed")
            .await
            .unwrap());
        sqlx::query("UPDATE operation_approvals SET expires_at=? WHERE id=?")
            .bind(now())
            .bind(expired.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        assert!(
            !repo
                .consume_operation_approval(expired, &binding, "agent")
                .await
                .unwrap()
        );
        let records = repo.audit_records(0, 100).await.unwrap();
        assert_eq!(records["records"].as_array().unwrap().len(), 5);
    }
    #[tokio::test]
    async fn concurrent_admission_has_one_winner_and_revoke_serializes() {
        let (_dir, repo) = repository().await;
        let binding = binding();
        let approval = id(&repo
            .issue_operation_approval(&binding, "human", "reviewed")
            .await
            .unwrap());
        let (a, b) = tokio::join!(
            repo.consume_operation_approval(approval, &binding, "agent"),
            repo.consume_operation_approval(approval, &binding, "agent")
        );
        assert_ne!(a.unwrap(), b.unwrap());
        let approval = id(&repo
            .issue_operation_approval(&binding, "human", "reviewed")
            .await
            .unwrap());
        let (consume, revoke) = tokio::join!(
            repo.consume_operation_approval(approval, &binding, "agent"),
            repo.revoke_operation_approval(approval, "human")
        );
        assert_ne!(consume.unwrap(), revoke.is_ok());
    }
    #[tokio::test]
    async fn audit_failure_rolls_back_issue_consume_and_revoke() {
        let (_dir, repo) = repository().await;
        let binding = binding();
        let approval = id(&repo
            .issue_operation_approval(&binding, "human", "reviewed")
            .await
            .unwrap());
        sqlx::query("CREATE TRIGGER reject_operation_audit BEFORE INSERT ON audit_records BEGIN SELECT RAISE(ABORT,'audit unavailable'); END").execute(&repo.pool).await.unwrap();
        assert!(
            repo.issue_operation_approval(&binding, "human", "reviewed")
                .await
                .is_err()
        );
        assert!(
            repo.consume_operation_approval(approval, &binding, "agent")
                .await
                .is_err()
        );
        assert!(
            repo.revoke_operation_approval(approval, "human")
                .await
                .is_err()
        );
        sqlx::query("DROP TRIGGER reject_operation_audit")
            .execute(&repo.pool)
            .await
            .unwrap();
        assert!(
            repo.consume_operation_approval(approval, &binding, "agent")
                .await
                .unwrap()
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM operation_approvals")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
}
