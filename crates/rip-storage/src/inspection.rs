//! Bounded, owner-bound asynchronous PDF inspection; immutable reports remain separate.
use super::*;
use serde_json::{Value, json};
async fn event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    value: &Value,
) -> Result<(), StorageError> {
    let payload = json!({"schemaVersion":1,"inspectionId":value["inspectionId"],"revision":value["revision"],
        "state":value["state"],"reasonCode":value["reasonCode"],"createdAt":now(),
        "automaticRetry":false,"reportAvailable":value["state"] == "COMPLETED"});
    sqlx::query("INSERT INTO inspection_events(inspection_id,revision,payload) VALUES(?,?,?)")
        .bind(
            value["inspectionId"]
                .as_str()
                .ok_or(StorageError::Conflict)?,
        )
        .bind(value["revision"].as_i64().ok_or(StorageError::Conflict)?)
        .bind(serde_json::to_string(&payload)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
impl Repository {
    pub async fn inspection_events(
        &self,
        id: Uuid,
        actor: &str,
        after: i64,
        limit: i64,
    ) -> Result<Value, StorageError> {
        if after < 0 || !(1..=100).contains(&limit) {
            return Err(StorageError::Conflict);
        }
        let _ = self.inspection_request(id, actor).await?;
        let rows = sqlx::query("SELECT sequence,payload FROM inspection_events WHERE inspection_id=? AND sequence>? ORDER BY sequence LIMIT ?")
            .bind(id.to_string()).bind(after).bind(limit+1).fetch_all(&self.pool).await?;
        let has_more = rows.len() > limit as usize;
        let mut cursor = after;
        let mut events = vec![];
        for row in rows.into_iter().take(limit as usize) {
            cursor = row.get("sequence");
            let mut value: Value = serde_json::from_str(row.get("payload"))?;
            value["sequence"] = json!(cursor);
            events.push(value);
        }
        Ok(json!({"schemaVersion":1,"events":events,"nextAfter":cursor,"hasMore":has_more}))
    }
    pub async fn enqueue_inspection(
        &self,
        id: Uuid,
        actor: &str,
        hash: &str,
        bytes: u64,
        directory: Uuid,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        // Acquire the SQLite writer before reading idempotency/quota state.
        sqlx::query("UPDATE inspection_requests SET revision=revision WHERE 0")
            .execute(&mut *tx)
            .await?;
        if let Some(row) =
            sqlx::query("SELECT actor,source_hash FROM inspection_requests WHERE id=?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?
        {
            if row.get::<String, _>("actor") != actor || row.get::<String, _>("source_hash") != hash
            {
                return Err(StorageError::Conflict);
            }
            return Ok(false);
        }
        let recorded: i64 =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM document_inspections WHERE id=?)")
                .bind(id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        if recorded == 1 {
            return Err(StorageError::Conflict);
        }
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM inspection_requests WHERE cleanup=1")
                .fetch_one(&mut *tx)
                .await?;
        let total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(source_bytes),0) FROM inspection_requests WHERE cleanup=1",
        )
        .fetch_one(&mut *tx)
        .await?;
        if count >= 32
            || bytes == 0
            || bytes > 32 * 1024 * 1024
            || total as u64 + bytes > 256 * 1024 * 1024
        {
            return Err(StorageError::Conflict);
        }
        let payload = json!({"schemaVersion":1,"inspectionId":id,"sourceSha256":hash,"sourceBytes":bytes,"state":"QUEUED","revision":0,"createdAt":now(),"reasonCode":null,"automaticRetry":false});
        sqlx::query("INSERT INTO inspection_requests(id,actor,source_hash,source_bytes,directory,payload,state,revision) VALUES(?,?,?,?,?,?,'QUEUED',0)")
            .bind(id.to_string()).bind(actor).bind(hash).bind(bytes as i64).bind(directory.to_string()).bind(serde_json::to_string(&payload)?).execute(&mut *tx).await?;
        event(&mut tx, &payload).await?;
        tx.commit().await?;
        Ok(true)
    }
    pub async fn inspection_request(&self, id: Uuid, actor: &str) -> Result<Value, StorageError> {
        let text: String =
            sqlx::query_scalar("SELECT payload FROM inspection_requests WHERE id=? AND actor=?")
                .bind(id.to_string())
                .bind(actor)
                .fetch_optional(&self.pool)
                .await?
                .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(&text)?)
    }
    pub async fn inspection_work(&self) -> Result<Vec<(Uuid, String, Uuid, Value)>, StorageError> {
        let rows=sqlx::query("SELECT id,actor,directory,payload FROM inspection_requests WHERE cleanup=1 ORDER BY rowid LIMIT 32").fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    Uuid::parse_str(row.get("id")).map_err(|_| StorageError::Conflict)?,
                    row.get("actor"),
                    Uuid::parse_str(row.get("directory")).map_err(|_| StorageError::Conflict)?,
                    serde_json::from_str(row.get("payload"))?,
                ))
            })
            .collect()
    }
    pub async fn set_inspection_state(
        &self,
        id: Uuid,
        revision: i64,
        state: &str,
        reason: Option<&str>,
        report: Option<Value>,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE inspection_requests SET revision=revision WHERE 0")
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query(
            "SELECT actor,payload,state FROM inspection_requests WHERE id=? AND revision=?",
        )
        .bind(id.to_string())
        .bind(revision)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StorageError::Conflict)?;
        let old: String = row.get("state");
        if !matches!(
            (old.as_str(), state),
            ("QUEUED", "RUNNING" | "FAILED" | "CANCELLED") | ("RUNNING", "COMPLETED" | "FAILED")
        ) {
            return Err(StorageError::Conflict);
        }
        let mut value: Value = serde_json::from_str(row.get("payload"))?;
        if let Some(report) = report {
            if state != "COMPLETED"
                || report["inspectionId"] != id.to_string()
                || report["sourceSha256"] != value["sourceSha256"]
                || report["sourceBytes"] != value["sourceBytes"]
            {
                return Err(StorageError::Conflict);
            }
            sqlx::query("INSERT INTO document_inspections(id,actor,payload) VALUES(?,?,?)")
                .bind(id.to_string())
                .bind(row.get::<String, _>("actor"))
                .bind(serde_json::to_string(&report)?)
                .execute(&mut *tx)
                .await?;
        } else if state == "COMPLETED" {
            return Err(StorageError::Conflict);
        }
        value["state"] = json!(state);
        value["revision"] = json!(revision + 1);
        value["reasonCode"] = json!(reason);
        sqlx::query("UPDATE inspection_requests SET state=?,revision=revision+1,payload=? WHERE id=? AND revision=?")
            .bind(state).bind(serde_json::to_string(&value)?).bind(id.to_string()).bind(revision).execute(&mut *tx).await?;
        event(&mut tx, &value).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn finish_inspection_cleanup(&self, id: Uuid) -> Result<(), StorageError> {
        sqlx::query("UPDATE inspection_requests SET cleanup=0 WHERE id=? AND state IN ('COMPLETED','FAILED','CANCELLED')").bind(id.to_string()).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn inspection_directory_registered(
        &self,
        directory: Uuid,
    ) -> Result<bool, StorageError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM inspection_requests WHERE directory=?)",
        )
        .bind(directory.to_string())
        .fetch_one(&self.pool)
        .await?;
        Ok(n == 1)
    }
}
