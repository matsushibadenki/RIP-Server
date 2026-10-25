pub mod approvals;
pub mod audit;
pub mod audit_archive;
pub mod audit_remote;
pub mod audit_vault;
pub mod budget;
pub mod budget_client;
pub mod color;
pub mod color_policy;
pub mod delivery;
pub mod delivery_receipt;
pub mod inspection;
pub mod kms;
pub mod maintenance;
pub mod pipeline;
pub mod progress;
pub mod receipts;
pub mod s3;
mod s3_resolution;
pub mod source_validation;
pub mod unified;
use rip_core::{Artifact, Job, JobState};
use sha2::{Digest, Sha256};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{path::Path, time::Duration};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("snapshot_budget_exhausted")]
    BudgetExhausted,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("receipt_signing_key_revoked")]
    ReceiptKeyRevoked,
    #[error("invalid_receipt_signing_key")]
    ReceiptKey,
    #[error("job_not_found")]
    NotFound,
    #[error("state_conflict")]
    Conflict,
}
#[derive(Clone)]
pub struct Repository {
    pool: SqlitePool,
    receipt_signer: Option<std::sync::Arc<receipts::ReceiptSigner>>,
}
impl Repository {
    pub async fn open(path: &Path) -> Result<Self, StorageError> {
        let signer = match std::env::var_os("JRIP_RECEIPT_SIGNING_KEY") {
            Some(path) => Some(receipts::ReceiptSigner::from_pkcs8(
                &std::fs::read(path).map_err(|_| StorageError::ReceiptKey)?,
            )?),
            None => None,
        };
        Self::open_with_receipt_signer(path, signer).await
    }
    pub async fn open_with_receipt_signer(
        path: &Path,
        signer: Option<receipts::ReceiptSigner>,
    ) -> Result<Self, StorageError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .busy_timeout(Duration::from_secs(5)),
            )
            .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS snapshot_budget(id INTEGER PRIMARY KEY CHECK(id=1),capacity INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS snapshot_reservations(id TEXT PRIMARY KEY,owner TEXT NOT NULL,bytes INTEGER NOT NULL,state TEXT NOT NULL,payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS snapshot_held ON snapshot_reservations(state,id)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS color_policies(digest TEXT PRIMARY KEY,payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS source_validations(job_id TEXT PRIMARY KEY,payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, payload TEXT NOT NULL, priority INTEGER NOT NULL, created_at INTEGER NOT NULL, revision INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS job_events (sequence INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL, state TEXT NOT NULL, created_at INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS job_events_cursor ON job_events(job_id,sequence)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS ipp_jobs (ipp_id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL UNIQUE)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS worker_leases (job_id TEXT PRIMARY KEY, run_id TEXT NOT NULL, expires_at INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS print_preflights (id TEXT PRIMARY KEY, payload TEXT NOT NULL, expires_at INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS print_commits (preflight_id TEXT PRIMARY KEY, job_id TEXT NOT NULL UNIQUE)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS print_receipts (job_id TEXT PRIMARY KEY, payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS print_result_receipts (job_id TEXT PRIMARY KEY, payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS job_artifacts (job_id TEXT PRIMARY KEY, payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS delivery_result_receipts(delivery_id TEXT PRIMARY KEY,payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS deliveries (id TEXT PRIMARY KEY,payload TEXT NOT NULL,state TEXT NOT NULL,revision INTEGER NOT NULL,lease_until INTEGER,created_at INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS delivery_queue ON deliveries(state,created_at,id)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS delivery_events (sequence INTEGER PRIMARY KEY AUTOINCREMENT,delivery_id TEXT NOT NULL,state TEXT NOT NULL,created_at INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS job_event_details (sequence INTEGER PRIMARY KEY,payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS audit_remote_prunes(archive_id TEXT NOT NULL,target TEXT NOT NULL,state TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(archive_id,target))").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS audit_remote_exports(archive_id TEXT NOT NULL,target TEXT NOT NULL,digest TEXT NOT NULL,bytes INTEGER NOT NULL,verified_at INTEGER NOT NULL,PRIMARY KEY(archive_id,target))").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS audit_archive_prunes(archive_id TEXT PRIMARY KEY,payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS audit_archive_exports(archive_id TEXT NOT NULL,target TEXT NOT NULL,digest TEXT NOT NULL,bytes INTEGER NOT NULL,verified_at INTEGER NOT NULL,PRIMARY KEY(archive_id,target))").execute(&pool).await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS audit_archives(id TEXT PRIMARY KEY,payload TEXT NOT NULL)",
        )
        .execute(&pool)
        .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS audit_records (sequence INTEGER PRIMARY KEY AUTOINCREMENT,correlation_id TEXT NOT NULL,identity TEXT NOT NULL,operation TEXT NOT NULL,target TEXT,started_at INTEGER NOT NULL,finished_at INTEGER,status INTEGER)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS execution_approvals(id TEXT PRIMARY KEY,payload TEXT NOT NULL,expires_at INTEGER NOT NULL,revoked INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS retention_tombstones(job_id TEXT PRIMARY KEY,created_at INTEGER NOT NULL,finished_at INTEGER)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS color_attempts(id TEXT PRIMARY KEY,source_job TEXT NOT NULL,ticket TEXT NOT NULL,payload TEXT NOT NULL,state TEXT NOT NULL,revision INTEGER NOT NULL,lease_until INTEGER,created_at INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS color_events(sequence INTEGER PRIMARY KEY AUTOINCREMENT,attempt_id TEXT NOT NULL,payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS color_queue ON color_attempts(state,created_at,id)",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS color_event_cursor ON color_events(attempt_id,sequence)",
        )
        .execute(&pool)
        .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS color_cleanup(attempt_id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS job_tickets(job_id TEXT PRIMARY KEY,payload TEXT NOT NULL)",
        )
        .execute(&pool)
        .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS delivery_contracts(delivery_id TEXT PRIMARY KEY,payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS inspection_requests(id TEXT PRIMARY KEY,actor TEXT NOT NULL,source_hash TEXT NOT NULL,source_bytes INTEGER NOT NULL,directory TEXT NOT NULL UNIQUE,payload TEXT NOT NULL,state TEXT NOT NULL,revision INTEGER NOT NULL,cleanup INTEGER NOT NULL DEFAULT 1)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS inspection_events(sequence INTEGER PRIMARY KEY AUTOINCREMENT,inspection_id TEXT NOT NULL,revision INTEGER NOT NULL,payload TEXT NOT NULL,UNIQUE(inspection_id,revision))").execute(&pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS inspection_event_cursor ON inspection_events(inspection_id,sequence)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS document_inspections(id TEXT PRIMARY KEY,actor TEXT NOT NULL,payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS print_pipelines(id TEXT PRIMARY KEY,source_job TEXT NOT NULL,ticket TEXT NOT NULL,payload TEXT NOT NULL,state TEXT NOT NULL,revision INTEGER NOT NULL,created_at INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS pipeline_source ON print_pipelines(source_job,state)",
        )
        .execute(&pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS pipeline_order ON print_pipelines(created_at,id)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS pipeline_events(sequence INTEGER PRIMARY KEY AUTOINCREMENT,pipeline_id TEXT NOT NULL,payload TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS pipeline_event_cursor ON pipeline_events(pipeline_id,sequence)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS delivery_event_details(sequence INTEGER PRIMARY KEY,payload TEXT NOT NULL)").execute(&pool).await?;
        receipts::install(&pool, signer.as_ref()).await?;
        unified::install(&pool).await?;
        Ok(Self {
            pool,
            receipt_signer: signer.map(std::sync::Arc::new),
        })
    }
    pub async fn events(
        &self,
        id: Uuid,
        after: i64,
        limit: i64,
    ) -> Result<serde_json::Value, StorageError> {
        if after < 0 || !(1..=100).contains(&limit) {
            return Err(StorageError::Conflict);
        }
        let exists: i64 =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM job_events WHERE job_id=?)")
                .bind(id.to_string())
                .fetch_one(&self.pool)
                .await?;
        if exists == 0 {
            return Err(StorageError::NotFound);
        }
        let rows = sqlx::query("SELECT e.sequence,e.state,e.created_at,d.payload AS details FROM job_events e LEFT JOIN job_event_details d ON d.sequence=e.sequence WHERE e.job_id=? AND e.sequence>? ORDER BY e.sequence LIMIT ?")
            .bind(id.to_string()).bind(after).bind(limit + 1).fetch_all(&self.pool).await?;
        let has_more = rows.len() > limit as usize;
        let mut cursor = after;
        let mut events = Vec::new();
        for row in rows.into_iter().take(limit as usize) {
            cursor = row.get("sequence");
            let state: JobState = serde_json::from_str(row.get("state"))?;
            let mut event = serde_json::json!({"sequence":cursor,"state":state,"createdAt":row.get::<i64,_>("created_at")});
            if let Some(details) = row.get::<Option<String>, _>("details") {
                let details: serde_json::Value = serde_json::from_str(&details)?;
                event
                    .as_object_mut()
                    .unwrap()
                    .extend(details.as_object().ok_or(StorageError::Conflict)?.clone());
            }
            events.push(event);
        }
        Ok(serde_json::json!({"jobId":id,"events":events,"nextCursor":cursor,"hasMore":has_more}))
    }
    pub async fn save_document_inspection(
        &self,
        id: Uuid,
        actor: &str,
        payload: &serde_json::Value,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO document_inspections(id,actor,payload) VALUES(?,?,?)")
            .bind(id.to_string())
            .bind(actor)
            .bind(serde_json::to_string(payload)?)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn document_inspection(
        &self,
        id: Uuid,
        actor: &str,
    ) -> Result<serde_json::Value, StorageError> {
        let value: String =
            sqlx::query_scalar("SELECT payload FROM document_inspections WHERE id=? AND actor=?")
                .bind(id.to_string())
                .bind(actor)
                .fetch_optional(&self.pool)
                .await?
                .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(&value)?)
    }
    pub async fn save_preflight(
        &self,
        id: Uuid,
        payload: &serde_json::Value,
        expires_at: i64,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO print_preflights(id,payload,expires_at) VALUES(?,?,?)")
            .bind(id.to_string())
            .bind(serde_json::to_string(payload)?)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn get_preflight(&self, id: Uuid) -> Result<serde_json::Value, StorageError> {
        let row = sqlx::query("SELECT payload FROM print_preflights WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(row.get("payload"))?)
    }
    pub async fn retry_preflight(
        &self,
        id: Uuid,
        source: Uuid,
        revision: i64,
        payload: &serde_json::Value,
    ) -> Result<serde_json::Value, StorageError> {
        sqlx::query("INSERT OR IGNORE INTO print_preflights(id,payload,expires_at) SELECT ?,?,? FROM jobs WHERE id=? AND revision=? AND json_extract(payload,'$.state') IN ('FAILED','CANCELLED')")
            .bind(id.to_string()).bind(serde_json::to_string(payload)?).bind(i64::MAX)
            .bind(source.to_string()).bind(revision).execute(&self.pool).await?;
        let stored = self.get_preflight(id).await.map_err(|e| match e {
            StorageError::NotFound => StorageError::Conflict,
            other => other,
        })?;
        if stored["retrySource"] != source.to_string() || stored["retryRevision"] != revision {
            return Err(StorageError::Conflict);
        }
        Ok(stored)
    }
    pub async fn insert(&self, job: &Job) -> Result<(), StorageError> {
        self.insert_with_preflight(job, None).await
    }
    pub async fn committed_job(&self, id: Uuid) -> Result<Option<Uuid>, StorageError> {
        let row = sqlx::query("SELECT job_id FROM print_commits WHERE preflight_id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| Uuid::parse_str(r.get("job_id")).map_err(|_| StorageError::Conflict))
            .transpose()
    }
    pub async fn get_print_receipt(&self, job_id: Uuid) -> Result<serde_json::Value, StorageError> {
        let row = sqlx::query("SELECT payload FROM print_receipts WHERE job_id=?")
            .bind(job_id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(row.get("payload"))?)
    }
    pub async fn get_result_receipt(
        &self,
        job_id: Uuid,
    ) -> Result<serde_json::Value, StorageError> {
        let row = sqlx::query("SELECT payload FROM print_result_receipts WHERE job_id=?")
            .bind(job_id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(row.get("payload"))?)
    }
    pub async fn insert_with_preflight(
        &self,
        job: &Job,
        preflight: Option<Uuid>,
    ) -> Result<(), StorageError> {
        self.insert_with_ticket(job, preflight, None).await
    }
    pub async fn job_ticket(&self, id: Uuid) -> Result<rip_core::print::JobTicket, StorageError> {
        let payload: String = sqlx::query_scalar("SELECT payload FROM job_tickets WHERE job_id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(&payload)?)
    }
    pub async fn insert_with_ticket(
        &self,
        job: &Job,
        preflight: Option<Uuid>,
        ticket: Option<&rip_core::print::JobTicket>,
    ) -> Result<(), StorageError> {
        self.insert_with_pipeline(job, preflight, ticket, None)
            .await
    }
    pub async fn insert_with_pipeline(
        &self,
        job: &Job,
        preflight: Option<Uuid>,
        ticket: Option<&rip_core::print::JobTicket>,
        pipeline: Option<(Uuid, serde_json::Value)>,
    ) -> Result<(), StorageError> {
        if pipeline.is_some() && ticket.is_none() {
            return Err(StorageError::Conflict);
        }
        if let Some(ticket) = ticket {
            ticket
                .validate_raster()
                .map_err(|_| StorageError::Conflict)?;
            if ticket.job_id != Some(job.id)
                || ticket.document.sha256 != job.source_sha256
                || ticket.manifest != job.manifest
                || !ticket.document.integrity_verified
            {
                return Err(StorageError::Conflict);
            }
        }
        let mut tx = self.pool.begin().await?;
        if let Some(id) = preflight {
            let result = sqlx::query("INSERT OR IGNORE INTO print_commits(preflight_id,job_id) SELECT id,? FROM print_preflights WHERE id=? AND expires_at>?")
                .bind(job.id.to_string()).bind(id.to_string()).bind(now()).execute(&mut *tx).await?;
            if result.rows_affected() != 1 {
                return Err(StorageError::Conflict);
            }
        }
        sqlx::query("INSERT INTO jobs VALUES (?, ?, ?, ?, ?)")
            .bind(job.id.to_string())
            .bind(serde_json::to_string(job)?)
            .bind(job.manifest.priority as i64)
            .bind(job.created_at)
            .bind(job.revision)
            .execute(&mut *tx)
            .await?;
        if let Some(ticket) = ticket {
            sqlx::query("INSERT INTO job_tickets(job_id,payload) VALUES(?,?)")
                .bind(job.id.to_string())
                .bind(serde_json::to_string(ticket)?)
                .execute(&mut *tx)
                .await?;
        }
        store_job_event(&mut tx, job).await?;
        if let Some((id, ticket)) = pipeline {
            pipeline::insert_new(&mut tx, id, job, ticket).await?;
        }
        if let Some(id) = preflight {
            let row = sqlx::query("SELECT payload, strftime('%Y-%m-%dT%H:%M:%SZ', ?, 'unixepoch') AS issued_at FROM print_preflights WHERE id=?")
                .bind(job.created_at).bind(id.to_string()).fetch_one(&mut *tx).await?;
            let pf: serde_json::Value = serde_json::from_str(row.get("payload"))?;
            let mut receipt = serde_json::json!({
                "acx":"0.1", "receiptId":job.id.to_string(),
                "capability":pf["capability"], "provider":"urn:acx:provider:jrip-local",
                "status":"accepted", "issuedAt":row.get::<String,_>("issued_at"),
                "requestHash":pf["requestHash"],
                "effects":if matches!(pf["profile"].as_str(),Some("org.jrip.pipeline-preflight-v1"|"org.jrip.pipeline-preflight-v2")) {serde_json::json!(["job-queued","source-stored","workflow-queued"])} else {serde_json::json!(["job-queued","source-stored"])},
                "recovery":{"reversible":false,"cancelUntil":"terminal","note":"Cancellation can discard output; consumed compute cannot be undone."},
                "proof":{"type":"plain-provider-record","profile":"org.jrip.acceptance-receipt-v1",
                    "jobId":job.id,"preflightId":id,"preflightDigest":pf["preflightDigest"],
                    "sourceSha256":job.source_sha256,"completionMeaning":"raster_export",
                    "requestHashDigestProfile":if pf.get("retrySource").is_some() {serde_json::json!("org.jrip.serde-json-v1")}else{pf["digestProfile"].clone()},"preflightDigestProfile":pf["digestProfile"],
                    "authorization":pf["authorizationEvidence"]["mechanism"].as_str().unwrap_or(if pf["actor"].is_string() {"actor-bound-bearer"}else{"shared-bearer"}),"actor":pf["actor"],"authorizationEvidence":pf["authorizationEvidence"],"signed":false,
                    "workflowId":pf["workflowRequest"]["requestId"],
                    "workflowCompletionMeaning":if matches!(pf["profile"].as_str(),Some("org.jrip.pipeline-preflight-v1"|"org.jrip.pipeline-preflight-v2")) {Some("device-confirmed")}else{None}}
            });
            if let Some(signer) = &self.receipt_signer {
                receipts::sign_stored(&mut tx, signer, &mut receipt, true).await?;
            }
            sqlx::query("INSERT INTO print_receipts(job_id,payload) VALUES(?,?)")
                .bind(job.id.to_string())
                .bind(serde_json::to_string(&receipt)?)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn get(&self, id: Uuid) -> Result<Job, StorageError> {
        let row = sqlx::query("SELECT payload FROM jobs WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(row.get("payload"))?)
    }
    pub async fn register_ipp_job(&self, id: Uuid) -> Result<i32, StorageError> {
        let result = sqlx::query("INSERT INTO ipp_jobs(job_id) VALUES(?)")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        i32::try_from(result.last_insert_rowid()).map_err(|_| StorageError::Conflict)
    }
    pub async fn get_ipp_job(&self, ipp_id: i32) -> Result<Job, StorageError> {
        let row = sqlx::query("SELECT job_id FROM ipp_jobs WHERE ipp_id=?")
            .bind(ipp_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StorageError::NotFound)?;
        let id = Uuid::parse_str(row.get("job_id")).map_err(|_| StorageError::Conflict)?;
        self.get(id).await
    }
    pub async fn list_ipp_jobs(&self) -> Result<Vec<(i32, Job)>, StorageError> {
        let rows = sqlx::query(
            "SELECT ipp_jobs.ipp_id, jobs.payload FROM ipp_jobs JOIN jobs ON jobs.id = ipp_jobs.job_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| Ok((row.get("ipp_id"), serde_json::from_str(row.get("payload"))?)))
            .collect()
    }
    pub async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Job>, StorageError> {
        let rows = sqlx::query("SELECT payload FROM jobs ORDER BY priority DESC, created_at ASC, id ASC LIMIT ? OFFSET ?").bind(limit.clamp(1,100)).bind(offset.max(0)).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|r| serde_json::from_str(r.get("payload")).map_err(StorageError::from))
            .collect()
    }
    pub async fn next_queued(&self) -> Result<Option<Job>, StorageError> {
        let row = sqlx::query("SELECT payload FROM jobs WHERE json_extract(payload, '$.state') = 'QUEUED' ORDER BY priority DESC, created_at ASC, id ASC LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| serde_json::from_str(row.get("payload")).map_err(StorageError::from))
            .transpose()
    }
    pub async fn claim_queued(
        &self,
        mut job: Job,
        run_id: Uuid,
        lease_until: i64,
    ) -> Result<Job, StorageError> {
        job.state
            .transition(JobState::Ripping)
            .map_err(|_| StorageError::Conflict)?;
        let previous = job.revision;
        job.state = JobState::Ripping;
        job.worker_run_id = Some(run_id);
        job.revision += 1;
        job.updated_at = now();
        job.error_code = None;
        let mut tx = self.pool.begin().await?;
        let changed =
            sqlx::query("UPDATE jobs SET payload=?, revision=? WHERE id=? AND revision=?")
                .bind(serde_json::to_string(&job)?)
                .bind(job.revision)
                .bind(job.id.to_string())
                .bind(previous)
                .execute(&mut *tx)
                .await?;
        if changed.rows_affected() != 1 {
            return Err(StorageError::Conflict);
        }
        store_job_event(&mut tx, &job).await?;
        sqlx::query("INSERT INTO worker_leases(job_id,run_id,expires_at) VALUES(?,?,?)")
            .bind(job.id.to_string())
            .bind(run_id.to_string())
            .bind(lease_until)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(job)
    }
    pub async fn renew_lease(
        &self,
        job_id: Uuid,
        run_id: Uuid,
        lease_until: i64,
    ) -> Result<(), StorageError> {
        let changed =
            sqlx::query("UPDATE worker_leases SET expires_at=? WHERE job_id=? AND run_id=?")
                .bind(lease_until)
                .bind(job_id.to_string())
                .bind(run_id.to_string())
                .execute(&self.pool)
                .await?;
        if changed.rows_affected() != 1 {
            return Err(StorageError::Conflict);
        }
        Ok(())
    }
    pub async fn clear_lease(&self, job_id: Uuid, run_id: Uuid) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM worker_leases WHERE job_id=? AND run_id=?")
            .bind(job_id.to_string())
            .bind(run_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn recover_expired_leases(&self, cutoff: i64) -> Result<Vec<Uuid>, StorageError> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query("SELECT worker_leases.job_id, worker_leases.run_id, jobs.payload FROM worker_leases JOIN jobs ON jobs.id=worker_leases.job_id WHERE worker_leases.expires_at<=?")
            .bind(cutoff)
            .fetch_all(&mut *tx)
            .await?;
        let mut recovered = Vec::new();
        for row in rows {
            let mut job: Job = serde_json::from_str(row.get("payload"))?;
            let run_id = Uuid::parse_str(row.get("run_id")).map_err(|_| StorageError::Conflict)?;
            if job.worker_run_id == Some(run_id) && !job.state.terminal() {
                job.state
                    .transition(JobState::Failed)
                    .map_err(|_| StorageError::Conflict)?;
                let previous = job.revision;
                job.state = JobState::Failed;
                job.error_code = Some("LEASE_EXPIRED".into());
                job.revision += 1;
                job.updated_at = now();
                let changed =
                    sqlx::query("UPDATE jobs SET payload=?, revision=? WHERE id=? AND revision=?")
                        .bind(serde_json::to_string(&job)?)
                        .bind(job.revision)
                        .bind(job.id.to_string())
                        .bind(previous)
                        .execute(&mut *tx)
                        .await?;
                if changed.rows_affected() == 1 {
                    store_job_event(&mut tx, &job).await?;
                    recovered.push(job.id);
                    store_result_receipt(&mut tx, &job, self.receipt_signer.as_deref()).await?;
                }
            }
            sqlx::query("DELETE FROM worker_leases WHERE job_id=? AND run_id=? AND expires_at<=?")
                .bind(job.id.to_string())
                .bind(run_id.to_string())
                .bind(cutoff)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(recovered)
    }
    pub async fn transition(
        &self,
        job: Job,
        next: JobState,
        error: Option<String>,
    ) -> Result<Job, StorageError> {
        self.transition_recorded(job, next, error, None).await
    }
    pub async fn complete_with_artifacts(
        &self,
        job: Job,
        artifacts: &[Artifact],
    ) -> Result<Job, StorageError> {
        if artifacts.is_empty()
            || artifacts.len() > 100
            || artifacts.iter().enumerate().any(|(i, a)| {
                a.page != i as u32 + 1
                    || a.name != format!("page-{:06}.tiff", a.page)
                    || a.media_type != "image/tiff"
                    || a.bytes == 0
                    || a.sha256.len() != 64
                    || !a
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return Err(StorageError::Conflict);
        }
        self.transition_recorded(job, JobState::Completed, None, Some(artifacts))
            .await
    }
    pub async fn artifacts(&self, id: Uuid) -> Result<serde_json::Value, StorageError> {
        let row = sqlx::query("SELECT payload FROM job_artifacts WHERE job_id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(row.get("payload"))?)
    }
    async fn transition_recorded(
        &self,
        mut job: Job,
        next: JobState,
        error: Option<String>,
        artifacts: Option<&[Artifact]>,
    ) -> Result<Job, StorageError> {
        job.state
            .transition(next)
            .map_err(|_| StorageError::Conflict)?;
        let previous = job.revision;
        job.state = next;
        job.revision += 1;
        job.updated_at = now();
        job.error_code = error;
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query("UPDATE jobs SET payload=?, revision=? WHERE id=? AND revision=?")
            .bind(serde_json::to_string(&job)?)
            .bind(job.revision)
            .bind(job.id.to_string())
            .bind(previous)
            .execute(&mut *tx)
            .await?;
        if result.rows_affected() != 1 {
            return Err(StorageError::Conflict);
        }
        store_job_event(&mut tx, &job).await?;
        if let Some(artifacts) = artifacts {
            sqlx::query("INSERT INTO job_artifacts(job_id,payload) VALUES(?,?)")
                .bind(job.id.to_string())
                .bind(serde_json::to_string(artifacts)?)
                .execute(&mut *tx)
                .await?;
        }
        store_result_receipt(&mut tx, &job, self.receipt_signer.as_deref()).await?;
        tx.commit().await?;
        Ok(job)
    }
    /// Remove metadata only after a terminal state; original files are retained for explicit retention maintenance.
    pub async fn delete(&self, job: Job) -> Result<(), StorageError> {
        if !job.state.terminal() {
            return Err(StorageError::Conflict);
        }
        let result = sqlx::query("DELETE FROM jobs WHERE id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM print_pipelines p WHERE p.source_job=jobs.id AND p.state NOT IN ('COMPLETED','FAILED','CANCELLED')) AND NOT EXISTS(SELECT 1 FROM color_attempts c WHERE c.source_job=jobs.id AND (c.state IN ('QUEUED','RUNNING') OR EXISTS(SELECT 1 FROM color_cleanup cc WHERE cc.attempt_id=c.id)))")
            .bind(job.id.to_string())
            .bind(job.revision)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() != 1 {
            return Err(StorageError::Conflict);
        }
        Ok(())
    }
}
async fn store_result_receipt(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    job: &Job,
    signer: Option<&receipts::ReceiptSigner>,
) -> Result<(), StorageError> {
    if !job.state.terminal() {
        return Ok(());
    }
    let Some(row) = sqlx::query("SELECT payload, strftime('%Y-%m-%dT%H:%M:%SZ', ?, 'unixepoch') AS issued_at FROM print_receipts WHERE job_id=?")
        .bind(job.updated_at).bind(job.id.to_string()).fetch_optional(&mut **tx).await? else { return Ok(()); };
    let accepted: serde_json::Value = serde_json::from_str(row.get("payload"))?;
    let status = match job.state {
        JobState::Completed => "succeeded",
        JobState::Cancelled => "cancelled",
        _ => "failed",
    };
    let artifact_row = sqlx::query("SELECT payload FROM job_artifacts WHERE job_id=?")
        .bind(job.id.to_string())
        .fetch_optional(&mut **tx)
        .await?;
    let artifacts: Option<serde_json::Value> = artifact_row
        .map(|r| serde_json::from_str(r.get("payload")))
        .transpose()?;
    let validation: Option<String> =
        sqlx::query_scalar("SELECT payload FROM source_validations WHERE job_id=?")
            .bind(job.id.to_string())
            .fetch_optional(&mut **tx)
            .await?;
    let validation: Option<serde_json::Value> = validation
        .map(|value| serde_json::from_str(&value))
        .transpose()?;
    let result = serde_json::json!({"jobId":job.id,"state":job.state,"revision":job.revision,
        "errorCode":job.error_code,"selectedEngine":job.selected_engine,"sourceSha256":job.source_sha256,
        "completionMeaning":"raster_export","sourceValidation":validation,"artifactHashesVerified":artifacts.is_some(),"pageCount":artifacts.as_ref().and_then(|v| v.as_array()).map(Vec::len),"artifacts":artifacts});
    let mut receipt = serde_json::json!({"acx":"0.1","receiptId":format!("{}:result",job.id),
        "provider":accepted["provider"],"capability":accepted["capability"],"requestHash":accepted["requestHash"],
        "status":status,"issuedAt":row.get::<String,_>("issued_at"),
        "resultHash":format!("sha256:{:x}",Sha256::digest(serde_json::to_vec(&result)?)),
        "effects":if job.state == JobState::Completed { vec!["raster-export-completed"] } else { vec!["job-terminated"] },
        "proof":{"type":"plain-provider-record","signed":false,"profile":"org.jrip.result-receipt-v1",
        "digestProfile":"org.jrip.serde-json-v1","requestHashDigestProfile":accepted["proof"]["requestHashDigestProfile"],"acceptanceReceiptId":accepted["receiptId"],"result":result}});
    if let Some(signer) = signer {
        receipts::sign_stored(tx, signer, &mut receipt, false).await?;
    }
    sqlx::query("INSERT INTO print_result_receipts(job_id,payload) VALUES(?,?)")
        .bind(job.id.to_string())
        .bind(serde_json::to_string(&receipt)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

async fn store_job_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    job: &Job,
) -> Result<(), StorageError> {
    let result = sqlx::query("INSERT INTO job_events(job_id,state,created_at) VALUES(?,?,?)")
        .bind(job.id.to_string())
        .bind(serde_json::to_string(&job.state)?)
        .bind(job.updated_at)
        .execute(&mut **tx)
        .await?;
    let stage = match job.state {
        JobState::Received | JobState::Validating | JobState::Preflight => "VALIDATING",
        JobState::Queued | JobState::Held => "QUEUED_FOR_RIP",
        JobState::Ripping => "RIPPING",
        JobState::ColorProcessing => "COLOR_MANAGING",
        JobState::Halftoning => "HALFTONING",
        JobState::Spooling => "EXPORTING",
        JobState::Printing => "PRINTING",
        JobState::Completed => "COMPLETED",
        JobState::Cancelled => "CANCELED",
        JobState::Failed => "FAILED",
    };
    let reason = job.error_code.as_deref().unwrap_or(match job.state {
        JobState::Held => "HELD_BY_USER",
        JobState::Cancelled => "CANCELED_BY_USER",
        JobState::Completed => "TIFF_EXPORT_COMPLETED",
        JobState::Failed => "UNSPECIFIED_FAILURE",
        _ => "NONE",
    });
    let metadata = serde_json::json!({"schemaVersion":1,"stage":stage,"revision":job.revision,
        "reasonCode":reason,"retryable":matches!(reason,"LEASE_EXPIRED"|"WORKER_EXIT"|"TIMEOUT"),
        "progressPercent":if job.state==JobState::Completed {Some(100)} else {None},
        "completionKind":if job.state==JobState::Completed {Some("tiff-export")} else {None}});
    sqlx::query("INSERT INTO job_event_details(sequence,payload) VALUES(?,?)")
        .bind(result.last_insert_rowid())
        .bind(metadata.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rip_core::{ColorMode, DocumentFormat, Manifest};
    #[tokio::test]
    async fn signed_receipts_survive_restart_and_key_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.db");
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap();
        let signer = receipts::ReceiptSigner::from_pkcs8(doc.as_ref()).unwrap();
        let key = signer.public_key().to_vec();
        let repo = Repository::open_with_receipt_signer(&path, Some(signer))
            .await
            .unwrap();
        let j = Job {
            id: Uuid::new_v4(),
            manifest: Manifest {
                name: "日本語.ps".into(),
                format: DocumentFormat::Postscript,
                dpi: 600,
                color_mode: ColorMode::Cmyk,
                priority: 50,
                engine: Default::default(),
            },
            state: JobState::Queued,
            source_sha256: "hash".into(),
            created_at: now(),
            updated_at: now(),
            revision: 0,
            error_code: None,
            selected_engine: None,
            worker_run_id: None,
        };
        let pf = Uuid::new_v4();
        repo.save_preflight(
            pf,
            &serde_json::json!({"capability":"print","requestHash":"sha256:test"}),
            now() + 60,
        )
        .await
        .unwrap();
        repo.insert_with_preflight(&j, Some(pf)).await.unwrap();
        let accepted = repo.get_print_receipt(j.id).await.unwrap();
        assert!(receipts::verify(&accepted, &key));
        let rotated =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .unwrap();
        let signer = receipts::ReceiptSigner::from_pkcs8(rotated.as_ref()).unwrap();
        let rotated_key = signer.public_key().to_vec();
        let repo = Repository::open_with_receipt_signer(&path, Some(signer))
            .await
            .unwrap();
        assert_eq!(repo.get_print_receipt(j.id).await.unwrap(), accepted);
        repo.transition(j.clone(), JobState::Cancelled, None)
            .await
            .unwrap();
        let result = repo.get_result_receipt(j.id).await.unwrap();
        assert!(receipts::verify(&result, &rotated_key));
        assert!(!receipts::verify(&result, &key));
        let reopened = Repository::open_with_receipt_signer(&path, None)
            .await
            .unwrap();
        assert_eq!(reopened.get_result_receipt(j.id).await.unwrap(), result);
        let keys = reopened.receipt_keys("", 1).await.unwrap();
        assert_eq!(keys["hasMore"], true);
        assert_eq!(keys["trustEstablished"], false);
        assert_eq!(keys["signingEnabledForThisProcess"], false);
        let second = reopened
            .receipt_keys(keys["nextAfter"].as_str().unwrap(), 1)
            .await
            .unwrap();
        assert_eq!(second["hasMore"], false);
        assert_ne!(keys["keys"][0]["keyId"], second["keys"][0]["keyId"]);
        assert_eq!(
            repo.receipt_keys("", 100).await.unwrap()["keys"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|k| k["activeForThisProcess"] == true)
                .count(),
            1
        );
        assert!(reopened.receipt_keys("", 101).await.is_err());
        assert!(
            reopened
                .verify_receipt_current(&result, &rotated_key)
                .await
                .unwrap()
        );
        let revoked_id = result["signature"]["keyId"].as_str().unwrap();
        let record = reopened
            .revoke_receipt_key(revoked_id, "compromised", "admin")
            .await
            .unwrap();
        assert_eq!(record["replayed"], false);
        let repeat = reopened
            .revoke_receipt_key(revoked_id, "compromised", "other-admin")
            .await
            .unwrap();
        assert_eq!(repeat["replayed"], true);
        assert_eq!(repeat["actor"], "admin");
        assert!(matches!(
            reopened
                .revoke_receipt_key(revoked_id, "retired", "admin")
                .await,
            Err(StorageError::Conflict)
        ));
        let audit = reopened.audit_records(0, 100).await.unwrap();
        assert_eq!(audit["records"].as_array().unwrap().len(), 1);
        assert_eq!(audit["records"][0]["target"], revoked_id);
        let catalog = repo.receipt_keys("", 100).await.unwrap();
        assert_eq!(catalog["signingEnabledForThisProcess"], false);
        assert_eq!(catalog["signingBlockedReason"], "KEY_REVOKED");
        assert_eq!(reopened.get_result_receipt(j.id).await.unwrap(), result);
        assert!(
            !reopened
                .verify_receipt_current(&result, &rotated_key)
                .await
                .unwrap()
        );
        assert!(receipts::verify(&result, &rotated_key)); // Cryptography is separate from current trust.
        let mut pending = j.clone();
        pending.id = Uuid::new_v4();
        let pf2 = Uuid::new_v4();
        repo.save_preflight(
            pf2,
            &serde_json::json!({"capability":"print","requestHash":"sha256:test"}),
            now() + 60,
        )
        .await
        .unwrap();
        assert!(matches!(
            repo.insert_with_preflight(&pending, Some(pf2)).await,
            Err(StorageError::ReceiptKeyRevoked)
        ));
        assert!(matches!(
            repo.get(pending.id).await,
            Err(StorageError::NotFound)
        ));
        assert_eq!(repo.committed_job(pf2).await.unwrap(), None);
        // A process opened before revocation sees DB state without restart.
        let accepted_before = receipts::ReceiptSigner::from_pkcs8(doc.as_ref()).unwrap();
        let old_repo = Repository::open_with_receipt_signer(&path, Some(accepted_before))
            .await
            .unwrap();
        old_repo
            .insert_with_preflight(&pending, Some(pf2))
            .await
            .unwrap();
        repo.transition(pending.clone(), JobState::Cancelled, None)
            .await
            .unwrap();
        let unsigned = repo.get_result_receipt(pending.id).await.unwrap();
        assert_eq!(unsigned["proof"]["signed"], false);
        assert_eq!(unsigned["proof"]["signingBlocked"]["code"], "KEY_REVOKED");
        assert!(unsigned.get("signature").is_none());
        let restarted = Repository::open_with_receipt_signer(
            &path,
            Some(receipts::ReceiptSigner::from_pkcs8(rotated.as_ref()).unwrap()),
        )
        .await
        .unwrap();
        assert_eq!(
            restarted.receipt_keys("", 100).await.unwrap()["signingEnabledForThisProcess"],
            false
        );
    }
    #[tokio::test]
    async fn durable_and_optimistic_transitions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.db");
        let repo = Repository::open(&path).await.unwrap();
        let j = Job {
            id: Uuid::new_v4(),
            manifest: Manifest {
                name: "日本語.ps".into(),
                format: DocumentFormat::Postscript,
                dpi: 600,
                color_mode: ColorMode::Cmyk,
                priority: 50,
                engine: Default::default(),
            },
            state: JobState::Queued,
            source_sha256: "hash".into(),
            created_at: now(),
            updated_at: now(),
            revision: 0,
            error_code: None,
            selected_engine: None,
            worker_run_id: None,
        };
        let mut legacy = serde_json::to_value(&j).unwrap();
        legacy.as_object_mut().unwrap().remove("selected_engine");
        legacy.as_object_mut().unwrap().remove("worker_run_id");
        legacy["manifest"].as_object_mut().unwrap().remove("engine");
        let old_job: Job = serde_json::from_value(legacy).unwrap();
        assert_eq!(old_job.manifest.engine, rip_core::EnginePreference::Auto);
        assert_eq!(old_job.selected_engine, None);
        assert_eq!(old_job.worker_run_id, None);
        repo.insert(&old_job).await.unwrap();
        let held = repo
            .transition(j.clone(), JobState::Held, None)
            .await
            .unwrap();
        assert!(matches!(
            repo.transition(j.clone(), JobState::Cancelled, None).await,
            Err(StorageError::Conflict)
        ));
        assert!(repo.delete(held.clone()).await.is_err());
        let reopened = Repository::open(&path).await.unwrap();
        assert_eq!(reopened.get(j.id).await.unwrap().state, JobState::Held);
        let cancelled = repo
            .transition(held, JobState::Cancelled, None)
            .await
            .unwrap();
        repo.delete(cancelled).await.unwrap();
        assert!(matches!(repo.get(j.id).await, Err(StorageError::NotFound)));
    }
    #[tokio::test]
    async fn next_queued_skips_held_and_prefers_priority() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::open(&dir.path().join("jobs.db")).await.unwrap();
        let make = |priority, state| Job {
            id: Uuid::new_v4(),
            manifest: Manifest {
                name: "queued.pdf".into(),
                format: DocumentFormat::Pdf,
                dpi: 600,
                color_mode: ColorMode::Cmyk,
                priority,
                engine: Default::default(),
            },
            state,
            source_sha256: "hash".into(),
            created_at: now(),
            updated_at: now(),
            revision: 0,
            error_code: None,
            selected_engine: None,
            worker_run_id: None,
        };
        let low = make(10, JobState::Queued);
        let high = make(90, JobState::Queued);
        let held = make(100, JobState::Held);
        for job in [&low, &high, &held] {
            repo.insert(job).await.unwrap();
        }
        assert_eq!(repo.next_queued().await.unwrap().unwrap().id, high.id);
        repo.transition(high, JobState::Cancelled, None)
            .await
            .unwrap();
        assert_eq!(repo.next_queued().await.unwrap().unwrap().id, low.id);
    }
    #[tokio::test]
    async fn lease_heartbeat_prevents_recovery_until_expired() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::open(&dir.path().join("jobs.db")).await.unwrap();
        let run_id = Uuid::new_v4();
        let job = Job {
            id: Uuid::new_v4(),
            manifest: Manifest {
                name: "leased.pdf".into(),
                format: DocumentFormat::Pdf,
                dpi: 600,
                color_mode: ColorMode::Cmyk,
                priority: 50,
                engine: Default::default(),
            },
            state: JobState::Queued,
            source_sha256: "hash".into(),
            created_at: now(),
            updated_at: now(),
            revision: 0,
            error_code: None,
            selected_engine: None,
            worker_run_id: None,
        };
        repo.insert(&job).await.unwrap();
        let claimed = repo.claim_queued(job, run_id, 100).await.unwrap();
        assert_eq!(claimed.state, JobState::Ripping);
        assert!(repo.recover_expired_leases(99).await.unwrap().is_empty());
        repo.renew_lease(claimed.id, run_id, 200).await.unwrap();
        assert!(repo.recover_expired_leases(150).await.unwrap().is_empty());
        assert_eq!(
            repo.recover_expired_leases(200).await.unwrap(),
            vec![claimed.id]
        );
        let recovered = repo.get(claimed.id).await.unwrap();
        assert_eq!(recovered.state, JobState::Failed);
        assert_eq!(recovered.error_code.as_deref(), Some("LEASE_EXPIRED"));
        assert!(matches!(
            repo.renew_lease(claimed.id, run_id, 300).await,
            Err(StorageError::Conflict)
        ));
    }
}
