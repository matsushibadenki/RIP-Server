//! Immutable JCS-addressed settings; changing a setting creates a new revision.
use crate::*;
use serde_json::{Value, json};
pub fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Repository {
    pub async fn save_color_policy(
        &self,
        definition: Value,
        actor: &str,
        reason: &str,
    ) -> Result<Value, StorageError> {
        if actor.is_empty()
            || actor.len() > 256
            || reason.trim().is_empty()
            || reason.len() > 1024
            || serde_json::to_vec(&definition)?.len() > 8192
        {
            return Err(StorageError::Conflict);
        }
        let digest = rip_digest::digest(&definition).map_err(|_| StorageError::Conflict)?;
        let digest = digest
            .strip_prefix("sha256:")
            .ok_or(StorageError::Conflict)?;
        let payload = json!({"schemaVersion":1,"profile":"org.jrip.color-policy-v1","digestProfile":rip_digest::PROFILE,"digest":digest,"definition":definition,"createdAt":now(),"createdBy":actor,"reason":reason});
        let mut tx = self.pool.begin().await?;
        let result =
            sqlx::query("INSERT OR IGNORE INTO color_policies(digest,payload) VALUES(?,?)")
                .bind(digest)
                .bind(payload.to_string())
                .execute(&mut *tx)
                .await?;
        if result.rows_affected() == 1 {
            sqlx::query("INSERT INTO audit_records(correlation_id,identity,operation,target,started_at,finished_at,status) VALUES(?,?,?,?,?,?,201)").bind(Uuid::new_v4().to_string()).bind(actor).bind("color-policy-create").bind(digest).bind(now()).bind(now()).execute(&mut *tx).await?;
        }
        let stored: String =
            sqlx::query_scalar("SELECT payload FROM color_policies WHERE digest=?")
                .bind(digest)
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(serde_json::from_str(&stored)?)
    }
    pub async fn color_policy(&self, digest: &str) -> Result<Value, StorageError> {
        if !valid_digest(digest) {
            return Err(StorageError::NotFound);
        }
        let payload: String =
            sqlx::query_scalar("SELECT payload FROM color_policies WHERE digest=?")
                .bind(digest)
                .fetch_optional(&self.pool)
                .await?
                .ok_or(StorageError::NotFound)?;
        let value: Value = serde_json::from_str(&payload)?;
        if value["digest"] != digest
            || value["profile"] != "org.jrip.color-policy-v1"
            || value["digestProfile"] != rip_digest::PROFILE
            || rip_digest::digest(&value["definition"]).map_err(|_| StorageError::Conflict)?
                != format!("sha256:{digest}")
        {
            return Err(StorageError::Conflict);
        }
        Ok(value)
    }
    pub async fn color_policies(&self, after: &str, limit: i64) -> Result<Value, StorageError> {
        if (!after.is_empty() && !valid_digest(after)) || !(1..=100).contains(&limit) {
            return Err(StorageError::Conflict);
        }
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT payload FROM color_policies WHERE digest>? ORDER BY digest LIMIT ?",
        )
        .bind(after)
        .bind(limit + 1)
        .fetch_all(&self.pool)
        .await?;
        let more = rows.len() > limit as usize;
        let entries: Vec<Value> = rows
            .into_iter()
            .take(limit as usize)
            .map(|s| serde_json::from_str(&s))
            .collect::<Result<_, _>>()?;
        let next = if more {
            entries.last().and_then(|v| v["digest"].as_str())
        } else {
            None
        };
        Ok(json!({"policies":entries,"nextAfter":next}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn audit_failure_rolls_back_and_duplicate_registration_preserves_first_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::open_with_receipt_signer(&dir.path().join("db"), None)
            .await
            .unwrap();
        let definition = json!({"policy":"test"});
        sqlx::query("CREATE TRIGGER reject_policy_audit BEFORE INSERT ON audit_records BEGIN SELECT RAISE(ABORT,'audit unavailable'); END").execute(&repo.pool).await.unwrap();
        assert!(
            repo.save_color_policy(definition.clone(), "operator", "reason")
                .await
                .is_err()
        );
        assert!(
            repo.color_policies("", 100).await.unwrap()["policies"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        sqlx::query("DROP TRIGGER reject_policy_audit")
            .execute(&repo.pool)
            .await
            .unwrap();
        let first = repo
            .save_color_policy(definition.clone(), "operator", "first reason")
            .await
            .unwrap();
        assert_eq!(
            repo.save_color_policy(definition, "other", "second reason")
                .await
                .unwrap(),
            first
        );
        let audits = repo.audit_records(0, 100).await.unwrap();
        assert_eq!(audits["records"].as_array().unwrap().len(), 1);
        assert_eq!(audits["records"][0]["identity"], "operator");
        assert_eq!(audits["records"][0]["target"], first["digest"]);
        sqlx::query(
            "UPDATE color_policies SET payload=json_set(payload,'$.definition.policy','corrupt')",
        )
        .execute(&repo.pool)
        .await
        .unwrap();
        assert!(
            repo.color_policy(first["digest"].as_str().unwrap())
                .await
                .is_err()
        );
    }
}
