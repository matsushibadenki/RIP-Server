//! Color attempts never mutate the original raster or automatically retry.
use super::*;
use serde_json::{Value, json};
async fn event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: Uuid,
    payload: &Value,
) -> Result<(), StorageError> {
    let total = payload["ticket"]["sourceArtifacts"]
        .as_array()
        .map_or(0, Vec::len);
    let completed = payload["completedPages"].as_u64().unwrap_or(0);
    let progress = if payload["state"] == "COMPLETED" {
        100
    } else if total > 0 {
        (completed * 100 / total as u64).min(99)
    } else {
        0
    };
    let payload = json!({"schemaVersion":1,"attemptId":id,"sourceJob":payload["sourceJob"],"state":payload["state"],"stage":"COLOR_MANAGING","revision":payload["revision"],"completedPages":completed,"pageCount":total,"progressPercent":progress,"reasonCode":payload["reasonCode"],"createdAt":payload["updatedAt"],"retryable":payload["reasonCode"]=="COLOR_WORKER_TIMEOUT" || payload["reasonCode"]=="COLOR_WORKER_LEASE_EXPIRED","completionKind":if payload["state"]=="COMPLETED" {Some("color-managed-tiff-export")}else{None}});
    sqlx::query("INSERT INTO color_events(attempt_id,payload) VALUES(?,?)")
        .bind(id.to_string())
        .bind(serde_json::to_string(&payload)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
impl Repository {
    pub async fn create_color(
        &self,
        id: Uuid,
        source: Uuid,
        revision: i64,
        ticket: Value,
    ) -> Result<Value, StorageError> {
        let mut tx = self.pool.begin().await?;
        if let Some(row) = sqlx::query("SELECT ticket,payload FROM color_attempts WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?
        {
            if serde_json::from_str::<Value>(row.get("ticket"))? != ticket {
                return Err(StorageError::Conflict);
            }
            return Ok(serde_json::from_str(row.get("payload"))?);
        }
        let eligible: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=? AND revision=? AND json_extract(payload,'$.state')='COMPLETED')")
            .bind(source.to_string()).bind(revision).fetch_one(&mut *tx).await?;
        if eligible != 1 {
            return Err(StorageError::Conflict);
        }
        let payload = json!({"schemaVersion":1,"id":id,"sourceJob":source,"ticket":ticket,"state":"QUEUED","stage":"COLOR_MANAGING","revision":0,"completedPages":0,"createdAt":now(),"updatedAt":now(),"reasonCode":null,"result":null});
        sqlx::query("INSERT INTO color_attempts(id,source_job,ticket,payload,state,revision,lease_until,created_at) VALUES(?,?,?,?,'QUEUED',0,NULL,?)")
            .bind(id.to_string()).bind(source.to_string()).bind(serde_json::to_string(&ticket)?).bind(serde_json::to_string(&payload)?).bind(now()).execute(&mut *tx).await?;
        event(&mut tx, id, &payload).await?;
        tx.commit().await?;
        Ok(payload)
    }
    pub async fn color_attempt(&self, id: Uuid) -> Result<Value, StorageError> {
        let payload: String = sqlx::query_scalar("SELECT payload FROM color_attempts WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(&payload)?)
    }
    pub async fn next_color(&self) -> Result<Option<Uuid>, StorageError> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM color_attempts WHERE state='QUEUED' ORDER BY created_at,id LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        id.map(|id| Uuid::parse_str(&id).map_err(|_| StorageError::Conflict))
            .transpose()
    }
    pub async fn update_color(
        &self,
        id: Uuid,
        revision: i64,
        next: &str,
        pages: u32,
        result: Option<Value>,
        reason: Option<&str>,
    ) -> Result<Value, StorageError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT payload,state,revision FROM color_attempts WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StorageError::NotFound)?;
        let state: &str = row.get("state");
        if row.get::<i64, _>("revision") != revision
            || !matches!(
                (state, next),
                ("QUEUED", "RUNNING")
                    | ("QUEUED", "CANCELLED")
                    | ("RUNNING", "RUNNING")
                    | ("RUNNING", "COMPLETED")
                    | ("RUNNING", "FAILED")
            )
        {
            return Err(StorageError::Conflict);
        }
        let mut payload: Value = serde_json::from_str(row.get("payload"))?;
        if u64::from(pages) < payload["completedPages"].as_u64().unwrap_or(0) {
            return Err(StorageError::Conflict);
        }
        payload["state"] = json!(next);
        payload["revision"] = json!(revision + 1);
        payload["completedPages"] = json!(pages);
        payload["updatedAt"] = json!(now());
        payload["reasonCode"] = json!(reason);
        payload["result"] = json!(result);
        let lease = (next == "RUNNING").then(|| now() + 600);
        let changed = sqlx::query("UPDATE color_attempts SET payload=?,state=?,revision=revision+1,lease_until=? WHERE id=? AND revision=?")
            .bind(serde_json::to_string(&payload)?).bind(next).bind(lease).bind(id.to_string()).bind(revision).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(StorageError::Conflict);
        }
        if reason == Some("COLOR_CLEANUP_FAILED") {
            sqlx::query("INSERT OR IGNORE INTO color_cleanup(attempt_id) VALUES(?)")
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        event(&mut tx, id, &payload).await?;
        tx.commit().await?;
        Ok(payload)
    }
    pub async fn pending_color_cleanup(&self) -> Result<Vec<Uuid>, StorageError> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT attempt_id FROM color_cleanup ORDER BY attempt_id LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;
        ids.into_iter()
            .map(|id| Uuid::parse_str(&id).map_err(|_| StorageError::Conflict))
            .collect()
    }
    pub async fn finish_color_cleanup(&self, id: Uuid) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM color_cleanup WHERE attempt_id=?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn expired_colors(&self) -> Result<Vec<Uuid>, StorageError> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM color_attempts WHERE state='RUNNING' AND lease_until<=? ORDER BY created_at LIMIT 100").bind(now()).fetch_all(&self.pool).await?;
        ids.into_iter()
            .map(|id| Uuid::parse_str(&id).map_err(|_| StorageError::Conflict))
            .collect()
    }
    pub async fn color_events(
        &self,
        id: Uuid,
        after: i64,
        limit: i64,
    ) -> Result<Value, StorageError> {
        if after < 0 || !(1..=100).contains(&limit) {
            return Err(StorageError::Conflict);
        }
        self.color_attempt(id).await?;
        let rows = sqlx::query("SELECT sequence,payload FROM color_events WHERE attempt_id=? AND sequence>? ORDER BY sequence LIMIT ?").bind(id.to_string()).bind(after).bind(limit+1).fetch_all(&self.pool).await?;
        let more = rows.len() > limit as usize;
        let mut cursor = after;
        let mut events = Vec::new();
        for row in rows.into_iter().take(limit as usize) {
            cursor = row.get("sequence");
            let mut value: Value = serde_json::from_str(row.get("payload"))?;
            value["sequence"] = json!(cursor);
            events.push(value);
        }
        Ok(json!({"attemptId":id,"events":events,"nextCursor":cursor,"hasMore":more}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn expired_attempts_remain_failed_and_stale_updates_cannot_publish() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("jobs.sqlite3");
        let repository = Repository::open(&path).await.unwrap();
        let job = Job {
            id: Uuid::new_v4(),
            manifest: serde_json::from_value(
                json!({"name":"x.pdf","format":"application/pdf","dpi":300}),
            )
            .unwrap(),
            state: JobState::Completed,
            source_sha256: "a".repeat(64),
            created_at: now(),
            updated_at: now(),
            revision: 0,
            error_code: None,
            selected_engine: None,
            worker_run_id: None,
        };
        repository.insert(&job).await.unwrap();
        let id = Uuid::new_v4();
        let ticket = json!({"sourceArtifacts":[{}],"sourceJob":job.id});
        repository
            .create_color(id, job.id, 0, ticket)
            .await
            .unwrap();
        repository
            .update_color(id, 0, "RUNNING", 0, None, None)
            .await
            .unwrap();
        assert!(
            repository
                .update_color(id, 0, "RUNNING", 0, None, None)
                .await
                .is_err()
        );
        sqlx::query("UPDATE color_attempts SET lease_until=? WHERE id=?")
            .bind(now() - 1)
            .bind(id.to_string())
            .execute(&repository.pool)
            .await
            .unwrap();
        drop(repository);
        let reopened = Repository::open(&path).await.unwrap();
        assert_eq!(reopened.expired_colors().await.unwrap(), vec![id]);
        reopened
            .update_color(id, 1, "FAILED", 0, None, Some("COLOR_WORKER_LEASE_EXPIRED"))
            .await
            .unwrap();
        assert!(
            reopened
                .update_color(id, 1, "COMPLETED", 1, Some(json!({})), None)
                .await
                .is_err()
        );
        assert!(reopened.next_color().await.unwrap().is_none());
        assert!(reopened.expired_colors().await.unwrap().is_empty());
        let history = reopened.color_events(id, 0, 100).await.unwrap();
        assert_eq!(history["events"].as_array().unwrap().len(), 3);
        assert!(history["events"][2]["ticket"].is_null());
        assert_eq!(history["events"][2]["retryable"], true);
        let cleanup_id = Uuid::new_v4();
        reopened
            .create_color(
                cleanup_id,
                job.id,
                0,
                json!({"sourceJob":job.id,"sourceArtifacts":[{}]}),
            )
            .await
            .unwrap();
        reopened
            .update_color(cleanup_id, 0, "RUNNING", 0, None, None)
            .await
            .unwrap();
        reopened
            .update_color(
                cleanup_id,
                1,
                "FAILED",
                0,
                None,
                Some("COLOR_CLEANUP_FAILED"),
            )
            .await
            .unwrap();
        assert_eq!(
            reopened.pending_color_cleanup().await.unwrap(),
            vec![cleanup_id]
        );
        assert!(reopened.delete(job.clone()).await.is_err());
        assert!(reopened.claim_retention(job.id, now() + 1).await.is_err());
        drop(reopened);
        let reopened = Repository::open(&path).await.unwrap();
        assert_eq!(
            reopened.pending_color_cleanup().await.unwrap(),
            vec![cleanup_id]
        );
        reopened.finish_color_cleanup(cleanup_id).await.unwrap();
        reopened.delete(job).await.unwrap();
    }
}
