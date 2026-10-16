//! Central snapshot admission. Held reservations never expire automatically.
use super::*;
use serde_json::{Value, json};
pub const SLOT: u64 = 16 * 1024 * 1024;
pub const DEFAULT_CAPACITY: u64 = 1024 * 1024 * 1024;
impl Repository {
    pub async fn configure_snapshot_budget(&self, capacity: u64) -> Result<(), StorageError> {
        if !(SLOT..=1024 * DEFAULT_CAPACITY).contains(&capacity) || !capacity.is_multiple_of(SLOT) {
            return Err(StorageError::Conflict);
        }
        sqlx::query("INSERT OR IGNORE INTO snapshot_budget(id,capacity) VALUES(1,?)")
            .bind(capacity as i64)
            .execute(&self.pool)
            .await?;
        let stored: i64 = sqlx::query_scalar("SELECT capacity FROM snapshot_budget WHERE id=1")
            .fetch_one(&self.pool)
            .await?;
        if stored != capacity as i64 {
            return Err(StorageError::Conflict);
        }
        Ok(())
    }
    pub async fn reserve_snapshot(
        &self,
        id: Uuid,
        owner: &str,
        bytes: u64,
    ) -> Result<Value, StorageError> {
        if !(1..=DEFAULT_CAPACITY).contains(&bytes) || owner.is_empty() || owner.len() > 256 {
            return Err(StorageError::Conflict);
        }
        let requested = bytes;
        let bytes = bytes.div_ceil(SLOT) * SLOT;
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE snapshot_budget SET capacity=capacity WHERE id=1")
            .execute(&mut *tx)
            .await?;
        if let Some(payload) =
            sqlx::query_scalar::<_, String>("SELECT payload FROM snapshot_reservations WHERE id=?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?
        {
            let value: Value = serde_json::from_str(&payload)?;
            if value["owner"] != owner
                || value["bytes"] != bytes
                || value["requestedBytes"] != requested
            {
                return Err(StorageError::Conflict);
            }
            return Ok(value);
        }
        let capacity: i64 = sqlx::query_scalar("SELECT capacity FROM snapshot_budget WHERE id=1")
            .fetch_one(&mut *tx)
            .await?;
        let held: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(bytes),0) FROM snapshot_reservations WHERE state='HELD'",
        )
        .fetch_one(&mut *tx)
        .await?;
        if held + bytes as i64 > capacity {
            return Err(StorageError::BudgetExhausted);
        }
        let payload = json!({"schemaVersion":1,"id":id,"owner":owner,"bytes":bytes,"requestedBytes":requested,"state":"HELD","createdAt":now(),"releasedAt":null});
        sqlx::query("INSERT INTO snapshot_reservations(id,owner,bytes,state,payload) VALUES(?,?,?,'HELD',?)")
            .bind(id.to_string()).bind(owner).bind(bytes as i64).bind(payload.to_string()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(payload)
    }
    pub async fn release_snapshot(
        &self,
        id: Uuid,
        owner: &str,
        expected_bytes: u64,
    ) -> Result<Value, StorageError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE snapshot_budget SET capacity=capacity WHERE id=1")
            .execute(&mut *tx)
            .await?;
        let payload: String =
            sqlx::query_scalar("SELECT payload FROM snapshot_reservations WHERE id=?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(StorageError::NotFound)?;
        let mut value: Value = serde_json::from_str(&payload)?;
        if value["owner"] != owner || value["bytes"] != expected_bytes {
            return Err(StorageError::Conflict);
        }
        if value["state"] == "RELEASED" {
            return Ok(value);
        }
        value["state"] = json!("RELEASED");
        value["releasedAt"] = json!(now());
        sqlx::query("UPDATE snapshot_reservations SET state='RELEASED',payload=? WHERE id=?")
            .bind(value.to_string())
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(value)
    }
    pub async fn snapshot_budget_status(
        &self,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Value, StorageError> {
        if !(1..=100).contains(&limit) {
            return Err(StorageError::Conflict);
        }
        let mut tx = self.pool.begin().await?;
        let capacity: i64 = sqlx::query_scalar("SELECT capacity FROM snapshot_budget WHERE id=1")
            .fetch_one(&mut *tx)
            .await?;
        let held: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(bytes),0) FROM snapshot_reservations WHERE state='HELD'",
        )
        .fetch_one(&mut *tx)
        .await?;
        let rows: Vec<String> = sqlx::query_scalar("SELECT payload FROM snapshot_reservations WHERE state='HELD' AND id>? ORDER BY id LIMIT ?")
            .bind(after.map(|id| id.to_string()).unwrap_or_default()).bind(i64::from(limit)+1).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        let more = rows.len() > limit as usize;
        let reservations = rows
            .into_iter()
            .take(limit as usize)
            .map(|s| serde_json::from_str::<Value>(&s))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(
            json!({"schemaVersion":1,"capacityBytes":capacity,"heldBytes":held,"reservations":reservations,"hasMore":more,"nextAfter":reservations.last().map(|v|v["id"].clone()),"automaticExpiry":false}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn concurrent_admission_replay_restart_and_explicit_release_never_overcommit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let repo = Repository::open(&path).await.unwrap();
        repo.configure_snapshot_budget(2 * SLOT).await.unwrap();
        assert!(repo.configure_snapshot_budget(3 * SLOT).await.is_err());
        let mut tasks = Vec::new();
        for _ in 0..12 {
            let repo = repo.clone();
            tasks.push(tokio::spawn(async move {
                repo.reserve_snapshot(Uuid::new_v4(), "worker-a", 1).await
            }));
        }
        let mut accepted = Vec::new();
        for task in tasks {
            match task.await.unwrap() {
                Ok(value) => accepted.push(value),
                Err(StorageError::BudgetExhausted) => {}
                Err(error) => panic!("{error}"),
            }
        }
        assert_eq!(accepted.len(), 2);
        let value = accepted[0].clone();
        let id = value["id"].as_str().unwrap().parse().unwrap();
        assert_eq!(
            repo.reserve_snapshot(id, "worker-a", 1).await.unwrap(),
            value
        );
        assert!(repo.reserve_snapshot(id, "worker-b", 1).await.is_err());
        assert!(repo.reserve_snapshot(id, "worker-a", 2).await.is_err());
        assert!(repo.release_snapshot(id, "worker-b", SLOT).await.is_err());
        assert!(
            repo.release_snapshot(id, "worker-a", SLOT + 1)
                .await
                .is_err()
        );
        drop(repo);
        let repo = Repository::open(&path).await.unwrap();
        repo.configure_snapshot_budget(2 * SLOT).await.unwrap();
        assert!(matches!(
            repo.reserve_snapshot(Uuid::new_v4(), "worker-b", 1).await,
            Err(StorageError::BudgetExhausted)
        ));
        let status = repo.snapshot_budget_status(None, 1).await.unwrap();
        assert_eq!(status["heldBytes"], 2 * SLOT);
        assert_eq!(status["automaticExpiry"], false);
        assert_eq!(status["hasMore"], true);
        assert_eq!(
            repo.snapshot_budget_status(
                Some(status["nextAfter"].as_str().unwrap().parse().unwrap()),
                1
            )
            .await
            .unwrap()["reservations"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let released = repo.release_snapshot(id, "worker-a", SLOT).await.unwrap();
        assert_eq!(released["state"], "RELEASED");
        assert_eq!(
            repo.release_snapshot(id, "worker-a", SLOT).await.unwrap(),
            released
        );
        assert_eq!(
            repo.reserve_snapshot(id, "worker-a", 1).await.unwrap(),
            released
        );
        assert_eq!(
            repo.snapshot_budget_status(None, 100).await.unwrap()["heldBytes"],
            SLOT
        );
        repo.reserve_snapshot(Uuid::new_v4(), "worker-b", SLOT)
            .await
            .unwrap();
        for size in [0, u64::MAX, DEFAULT_CAPACITY + 1] {
            assert!(
                repo.reserve_snapshot(Uuid::new_v4(), "worker", size)
                    .await
                    .is_err()
            );
        }
    }
}
