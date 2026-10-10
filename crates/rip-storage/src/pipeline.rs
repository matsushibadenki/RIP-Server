//! Durable orchestration records; child attempt IDs equal the pipeline UUID.
use super::*;
use serde_json::{Value, json};
async fn record(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    value: &Value,
) -> Result<(), StorageError> {
    let state = value["state"].as_str().ok_or(StorageError::Conflict)?;
    let stage = match state {
        "COLOR" => "color_management",
        "RIP" => "rip",
        "DELIVERY" | "UNKNOWN" => "printer",
        "WAITING_APPROVAL" => "approval",
        "COMPLETED" => "completed",
        _ => "orchestration",
    };
    let event = json!({"pipelineId":value["id"],"sourceJob":value["sourceJob"],"revision":value["revision"],"state":state,"stage":stage,"reason":value["reason"],"createdAt":now(),"retryable":matches!(state,"BLOCKED"|"WAITING_APPROVAL"),"automaticRetry":false,"completionMeaning":if state=="COMPLETED"{Some("device-confirmed")}else{None},"progress":null});
    sqlx::query("INSERT INTO pipeline_events(pipeline_id,payload) VALUES(?,?)")
        .bind(value["id"].as_str().ok_or(StorageError::Conflict)?)
        .bind(serde_json::to_string(&event)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
impl Repository {
    /// One SQLite read snapshot joins immutable contracts and observed attempt states.
    /// Missing historical evidence remains null; it is never inferred from job metadata.
    pub async fn pipeline_ticket(&self, id: Uuid) -> Result<Value, StorageError> {
        let mut tx = self.pool.begin().await?;
        let workflow: String = sqlx::query_scalar("SELECT payload FROM print_pipelines WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StorageError::NotFound)?;
        let workflow: Value = serde_json::from_str(&workflow)?;
        let source = workflow["sourceJob"]
            .as_str()
            .ok_or(StorageError::Conflict)?;
        let raster: Option<String> =
            sqlx::query_scalar("SELECT payload FROM job_tickets WHERE job_id=?")
                .bind(source)
                .fetch_optional(&mut *tx)
                .await?;
        let color: Option<String> =
            sqlx::query_scalar("SELECT payload FROM color_attempts WHERE id=?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        let delivery: Option<String> =
            sqlx::query_scalar("SELECT payload FROM deliveries WHERE id=?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        let contract: Option<String> =
            sqlx::query_scalar("SELECT payload FROM delivery_contracts WHERE delivery_id=?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        tx.commit().await?;
        fn parse(text: Option<String>) -> Result<Value, StorageError> {
            Ok(text
                .map(|text| serde_json::from_str(&text))
                .transpose()?
                .unwrap_or(Value::Null))
        }
        let raster = parse(raster)?;
        let color = parse(color)?;
        let delivery = parse(delivery)?;
        let contract = parse(contract)?;
        if (!color.is_null() && color["sourceJob"] != source)
            || (!delivery.is_null()
                && (delivery["ticket"]["sourceJob"] != source
                    || delivery["ticket"]["printerId"]
                        != workflow["ticket"]["request"]["printerId"]
                    || delivery["ticket"]["media"] != workflow["ticket"]["request"]["media"]))
        {
            return Err(StorageError::Conflict);
        }
        if !raster.is_null()
            && (raster["jobId"] != source
                || raster["document"]["sha256"] != workflow["ticket"]["sourceSha256"])
        {
            return Err(StorageError::Conflict);
        }
        if !contract.is_null()
            && (contract["ticket"]["jobId"] != source
                || contract["ticket"]["attemptId"] != id.to_string()
                || contract["ticket"]["sourceSha256"] != workflow["ticket"]["sourceSha256"]
                || contract["ticket"]["printerConfigSha256"]
                    != workflow["ticket"]["printerConfigSha256"])
        {
            return Err(StorageError::Conflict);
        }
        let ticket = json!({"schemaVersion":1,"version":"0.1","profile":"org.jrip.print.unified-ticket-v1",
            "pipelineId":id,"sourceJob":source,"raster":raster,"registration":workflow["ticket"],
            "color":color["ticket"],"physical":contract,"completionMeaning":"device-confirmed"});
        let digest = format!("{:x}", sha2::Sha256::digest(serde_json::to_vec(&ticket)?));
        let portable = match rip_digest::digest(&ticket) {
            Ok(hash) => json!({"profile":rip_digest::PROFILE,"sha256":hash,"available":true}),
            Err(_) => {
                json!({"profile":rip_digest::PROFILE,"sha256":null,"available":false,"reason":"CANONICAL_INPUT_UNSUPPORTED"})
            }
        };
        Ok(
            json!({"ticket":ticket,"portableDigest":portable,"ticketSha256":digest,"digestProfile":"org.jrip.serde-json-v1",
            "contractFinalized":!contract.is_null(),"rasterEvidenceAvailable":!raster.is_null(),
            "execution":{"pipelineState":workflow["state"],"pipelineRevision":workflow["revision"],
                "colorState":color["state"],"colorRevision":color["revision"],
                "deliveryState":delivery["state"],"deliveryRevision":delivery["revision"],
                "deviceJobId":delivery["deviceJobId"],
                "completed":workflow["state"] == "COMPLETED" && delivery["state"] == "COMPLETED" && !contract.is_null()},
            "authorizationGranted":false}),
        )
    }
    pub async fn create_pipeline(
        &self,
        id: Uuid,
        job: Uuid,
        revision: i64,
        ticket: Value,
    ) -> Result<Value, StorageError> {
        let mut tx = self.pool.begin().await?;
        if let Some(existing) =
            sqlx::query_scalar::<_, String>("SELECT payload FROM print_pipelines WHERE id=?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?
        {
            let stored: Value = serde_json::from_str(&existing)?;
            if stored["ticket"] != ticket || stored["sourceJob"] != job.to_string() {
                return Err(StorageError::Conflict);
            }
            tx.commit().await?;
            return Ok(stored);
        }
        let valid: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=? AND revision=? AND json_extract(payload,'$.state') NOT IN ('FAILED','CANCELLED'))")
            .bind(job.to_string()).bind(revision).fetch_one(&mut *tx).await?;
        if valid != 1 {
            return Err(StorageError::Conflict);
        }
        if ticket.get("sourceSha256").is_some() {
            let source: String = sqlx::query_scalar("SELECT payload FROM jobs WHERE id=?")
                .bind(job.to_string())
                .fetch_one(&mut *tx)
                .await?;
            let source: Job = serde_json::from_str(&source)?;
            if ticket["sourceSha256"] != source.source_sha256
                || ticket["sourceManifest"] != serde_json::to_value(&source.manifest)?
            {
                return Err(StorageError::Conflict);
            }
        }
        let payload = json!({"id":id,"sourceJob":job,"ticket":ticket,"state":"QUEUED","revision":0,"reason":null,"approvalId":null});
        let inserted=sqlx::query("INSERT OR IGNORE INTO print_pipelines(id,source_job,ticket,payload,state,revision,created_at) VALUES(?,?,?,?,'QUEUED',0,?)")
            .bind(id.to_string()).bind(job.to_string()).bind(serde_json::to_string(&ticket)?)
            .bind(serde_json::to_string(&payload)?).bind(now()).execute(&mut *tx).await?;
        if inserted.rows_affected() == 1 {
            record(&mut tx, &payload).await?;
        }
        let stored: String = sqlx::query_scalar("SELECT payload FROM print_pipelines WHERE id=?")
            .bind(id.to_string())
            .fetch_one(&mut *tx)
            .await?;
        let stored: Value = serde_json::from_str(&stored)?;
        if stored["ticket"] != ticket || stored["sourceJob"] != job.to_string() {
            return Err(StorageError::Conflict);
        }
        tx.commit().await?;
        Ok(stored)
    }
    pub async fn pipeline(&self, id: Uuid) -> Result<Value, StorageError> {
        let text: String = sqlx::query_scalar("SELECT payload FROM print_pipelines WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(&text)?)
    }
    pub async fn active_pipelines(&self) -> Result<Vec<Uuid>, StorageError> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT id FROM print_pipelines WHERE state NOT IN ('COMPLETED','FAILED','CANCELLED','BLOCKED','WAITING_APPROVAL') ORDER BY created_at,id LIMIT 100").fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|id| Uuid::parse_str(&id).map_err(|_| StorageError::Conflict))
            .collect()
    }
    pub async fn update_pipeline(
        &self,
        id: Uuid,
        revision: i64,
        state: &str,
        reason: Option<&str>,
        approval: Option<Uuid>,
    ) -> Result<Value, StorageError> {
        if !matches!(
            state,
            "CANCELLED"
                | "QUEUED"
                | "RIP"
                | "COLOR"
                | "DELIVERY"
                | "WAITING_APPROVAL"
                | "BLOCKED"
                | "UNKNOWN"
                | "COMPLETED"
                | "FAILED"
        ) {
            return Err(StorageError::Conflict);
        }
        let mut value = self.pipeline(id).await?;
        if value["revision"] != revision {
            return Err(StorageError::Conflict);
        }
        value["state"] = json!(state);
        value["reason"] = json!(reason);
        value["approvalId"] = json!(approval);
        value["revision"] = json!(revision.checked_add(1).ok_or(StorageError::Conflict)?);
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query("UPDATE print_pipelines SET payload=?,state=?,revision=revision+1 WHERE id=? AND revision=?")
            .bind(serde_json::to_string(&value)?).bind(state).bind(id.to_string()).bind(revision).execute(&mut *tx).await?;
        if result.rows_affected() != 1 {
            return Err(StorageError::Conflict);
        }
        record(&mut tx, &value).await?;
        tx.commit().await?;
        Ok(value)
    }
    pub async fn pipeline_events(
        &self,
        id: Uuid,
        after: i64,
        limit: i64,
    ) -> Result<Value, StorageError> {
        if after < 0 || !(1..=100).contains(&limit) {
            return Err(StorageError::Conflict);
        }
        let _ = self.pipeline(id).await?;
        let rows=sqlx::query("SELECT sequence,payload FROM pipeline_events WHERE pipeline_id=? AND sequence>? ORDER BY sequence LIMIT ?")
            .bind(id.to_string()).bind(after).bind(limit).fetch_all(&self.pool).await?;
        let mut events = vec![];
        let mut cursor = after;
        for row in rows {
            cursor = row.get("sequence");
            let mut value: Value = serde_json::from_str(row.get("payload"))?;
            value["sequence"] = json!(cursor);
            events.push(value);
        }
        Ok(json!({"schemaVersion":1,"events":events,"nextAfter":cursor}))
    }
}

pub(super) async fn insert_new(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: Uuid,
    job: &Job,
    ticket: Value,
) -> Result<(), StorageError> {
    if job.state != JobState::Queued
        || job.revision != 0
        || ticket["request"]["requestId"] != id.to_string()
        || ticket["request"]["expectedRevision"] != 0
        || ticket["sourceSha256"] != job.source_sha256
        || ticket["sourceManifest"] != serde_json::to_value(&job.manifest)?
    {
        return Err(StorageError::Conflict);
    }
    let payload = json!({"id":id,"sourceJob":job.id,"ticket":ticket,"state":"QUEUED","revision":0,"reason":null,"approvalId":null});
    let changed = sqlx::query("INSERT OR IGNORE INTO print_pipelines(id,source_job,ticket,payload,state,revision,created_at) VALUES(?,?,?,?,'QUEUED',0,?)")
        .bind(id.to_string()).bind(job.id.to_string()).bind(serde_json::to_string(&ticket)?)
        .bind(serde_json::to_string(&payload)?).bind(now()).execute(&mut **tx).await?;
    if changed.rows_affected() != 1 {
        return Err(StorageError::Conflict);
    }
    record(tx, &payload).await
}
