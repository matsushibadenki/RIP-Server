//! Durable delivery attempts. Expired sends become UNKNOWN, never QUEUED.
use super::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliveryTicket {
    pub source_job: Uuid,
    #[serde(default)]
    pub color_attempt_id: Option<Uuid>,
    pub printer_id: String,
    pub artifact_sha256: String,
    pub artifact_bytes: u64,
    pub dpi: u32,
    pub document_type: String,
    pub media: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeliveryState {
    Queued,
    Sending,
    Accepted,
    Completed,
    Cancelled,
    Aborted,
    Rejected,
    Unknown,
}
impl DeliveryState {
    fn label(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Sending => "SENDING",
            Self::Accepted => "ACCEPTED",
            Self::Completed => "COMPLETED",
            Self::Cancelled => "CANCELLED",
            Self::Aborted => "ABORTED",
            Self::Rejected => "REJECTED",
            Self::Unknown => "UNKNOWN",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub id: Uuid,
    pub ticket: DeliveryTicket,
    pub state: DeliveryState,
    pub revision: i64,
    pub device_job_id: Option<i32>,
    pub run_id: Option<Uuid>,
    pub lease_until: Option<i64>,
    pub reason: Option<String>,
}
async fn event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    delivery: &Delivery,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO delivery_events(delivery_id,state,created_at) VALUES(?,?,?)")
        .bind(delivery.id.to_string())
        .bind(delivery.state.label())
        .bind(now())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
impl Repository {
    pub async fn enqueue_delivery(
        &self,
        id: Uuid,
        ticket: DeliveryTicket,
    ) -> Result<Delivery, StorageError> {
        self.enqueue_approved_delivery(id, ticket, None).await
    }
    pub async fn enqueue_approved_delivery(
        &self,
        id: Uuid,
        ticket: DeliveryTicket,
        approval: Option<Uuid>,
    ) -> Result<Delivery, StorageError> {
        if ticket.printer_id.is_empty()
            || ticket.printer_id.len() > 128
            || !ticket
                .printer_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || ticket.artifact_sha256.len() != 64
            || !ticket
                .artifact_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || ticket.artifact_bytes == 0
            || ticket.artifact_bytes > 32 * 1024 * 1024
            || !(72..=2400).contains(&ticket.dpi)
            || !matches!(
                ticket.document_type.as_str(),
                "rgb_8" | "cmyk_8" | "black_8"
            )
            || ticket.media.is_empty()
            || ticket.media.len() > 128
            || !ticket
                .media
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(StorageError::Conflict);
        }
        let delivery = Delivery {
            id,
            ticket,
            state: DeliveryState::Queued,
            revision: 0,
            device_job_id: None,
            run_id: None,
            lease_until: None,
            reason: None,
        };
        let mut tx = self.pool.begin().await?;
        if let Some(color) = delivery.ticket.color_attempt_id {
            let valid:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM color_attempts WHERE id=? AND source_job=? AND state='COMPLETED')").bind(color.to_string()).bind(delivery.ticket.source_job.to_string()).fetch_one(&mut *tx).await?;
            if valid != 1 {
                return Err(StorageError::Conflict);
            }
        }
        if let Some(approval) = approval {
            let allowed:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM execution_approvals WHERE id=? AND revoked=0 AND expires_at>? AND json_extract(payload,'$.requestId')=? AND json_extract(payload,'$.jobId')=? AND json_extract(payload,'$.printerId')=? AND json_extract(payload,'$.media')=? AND json_extract(payload,'$.colorAttemptId') IS ?)")
                .bind(approval.to_string()).bind(now()).bind(id.to_string()).bind(delivery.ticket.source_job.to_string())
                .bind(&delivery.ticket.printer_id).bind(&delivery.ticket.media).bind(delivery.ticket.color_attempt_id.map(|id|id.to_string())).fetch_one(&mut *tx).await?;
            if allowed != 1 {
                return Err(StorageError::Conflict);
            }
        }
        let result=sqlx::query("INSERT OR IGNORE INTO deliveries(id,payload,state,revision,lease_until,created_at) SELECT ?,?,'QUEUED',0,NULL,? FROM jobs WHERE id=? AND json_extract(payload,'$.state')='COMPLETED'")
            .bind(id.to_string()).bind(serde_json::to_string(&delivery)?).bind(now()).bind(delivery.ticket.source_job.to_string()).execute(&mut *tx).await?;
        if result.rows_affected() == 1 {
            event(&mut tx, &delivery).await?;
        }
        let row = sqlx::query("SELECT payload FROM deliveries WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StorageError::Conflict)?;
        let stored: Delivery = serde_json::from_str(row.get("payload"))?;
        if stored.ticket != delivery.ticket {
            return Err(StorageError::Conflict);
        }
        tx.commit().await?;
        Ok(stored)
    }
    pub async fn delivery(&self, id: Uuid) -> Result<Delivery, StorageError> {
        let row = sqlx::query("SELECT payload FROM deliveries WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(row.get("payload"))?)
    }
    pub async fn accepted_deliveries(&self) -> Result<Vec<Delivery>, StorageError> {
        let rows=sqlx::query("SELECT payload FROM deliveries WHERE state='ACCEPTED' ORDER BY created_at,id LIMIT 100").fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| serde_json::from_str(row.get("payload")).map_err(StorageError::from))
            .collect()
    }
    pub async fn next_delivery(&self) -> Result<Option<Delivery>, StorageError> {
        let row = sqlx::query(
            "SELECT payload FROM deliveries WHERE state='QUEUED' ORDER BY created_at,id LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| serde_json::from_str(row.get("payload")).map_err(StorageError::from))
            .transpose()
    }
    pub async fn claim_delivery(
        &self,
        id: Uuid,
        revision: i64,
        run: Uuid,
    ) -> Result<Delivery, StorageError> {
        let mut delivery = self.delivery(id).await?;
        if delivery.state != DeliveryState::Queued || delivery.revision != revision {
            return Err(StorageError::Conflict);
        }
        delivery.state = DeliveryState::Sending;
        delivery.run_id = Some(run);
        delivery.lease_until = Some(now().checked_add(60).ok_or(StorageError::Conflict)?);
        self.update_delivery(delivery, DeliveryState::Queued, revision, None)
            .await
    }
    /// Only the live sender can record acceptance/rejection/uncertainty. Never send again on UNKNOWN.
    pub async fn finish_delivery(
        &self,
        id: Uuid,
        run: Uuid,
        next: DeliveryState,
        device_job_id: Option<i32>,
        reason: Option<String>,
    ) -> Result<Delivery, StorageError> {
        let mut delivery = self.delivery(id).await?;
        if delivery.state != DeliveryState::Sending
            || delivery.run_id != Some(run)
            || delivery.lease_until.is_none_or(|until| until <= now())
            || !matches!(
                next,
                DeliveryState::Accepted | DeliveryState::Rejected | DeliveryState::Unknown
            )
            || (next == DeliveryState::Accepted && device_job_id.is_none_or(|id| id <= 0))
            || (next != DeliveryState::Accepted && device_job_id.is_some())
            || reason.as_ref().is_some_and(|s| s.len() > 256)
        {
            return Err(StorageError::Conflict);
        }
        let revision = delivery.revision;
        delivery.state = next;
        delivery.device_job_id = device_job_id;
        delivery.reason = reason;
        self.update_delivery(delivery, DeliveryState::Sending, revision, Some(run))
            .await
    }
    /// Persist cancellation before any device operation; compete atomically with send claims.
    pub async fn request_delivery_cancel(
        &self,
        id: Uuid,
        revision: i64,
    ) -> Result<Delivery, StorageError> {
        let mut d = self.delivery(id).await?;
        if d.revision != revision {
            return Err(StorageError::Conflict);
        }
        match d.state {
            DeliveryState::Cancelled => Ok(d),
            DeliveryState::Queued => {
                d.state = DeliveryState::Cancelled;
                d.reason = Some("CANCELLED_BEFORE_SEND".into());
                self.update_delivery(d, DeliveryState::Queued, revision, None)
                    .await
            }
            DeliveryState::Accepted => {
                if d.reason
                    .as_deref()
                    .is_some_and(|r| r.starts_with("CANCEL_"))
                {
                    return Ok(d);
                }
                d.reason = Some("CANCEL_REQUESTED".into());
                self.update_delivery(d, DeliveryState::Accepted, revision, None)
                    .await
            }
            _ => Err(StorageError::Conflict),
        }
    }
    /// A durable claim prevents duplicate Cancel-Job calls across workers and restarts.
    pub async fn claim_delivery_cancel(
        &self,
        id: Uuid,
        revision: i64,
    ) -> Result<Delivery, StorageError> {
        let mut d = self.delivery(id).await?;
        if d.state != DeliveryState::Accepted
            || d.revision != revision
            || d.reason.as_deref() != Some("CANCEL_REQUESTED")
        {
            return Err(StorageError::Conflict);
        }
        d.reason = Some("CANCEL_DISPATCHING".into());
        self.update_delivery(d, DeliveryState::Accepted, revision, None)
            .await
    }
    pub async fn observe_delivery(
        &self,
        id: Uuid,
        revision: i64,
        next: DeliveryState,
    ) -> Result<Delivery, StorageError> {
        let mut delivery = self.delivery(id).await?;
        if delivery.state != DeliveryState::Accepted
            || delivery.revision != revision
            || !matches!(
                next,
                DeliveryState::Completed | DeliveryState::Cancelled | DeliveryState::Aborted
            )
        {
            return Err(StorageError::Conflict);
        }
        delivery.state = next;
        self.update_delivery(delivery, DeliveryState::Accepted, revision, None)
            .await
    }
    /// Administrator reconciliation binds a verified remote job; never makes UNKNOWN sendable.
    pub async fn reconcile_delivery(
        &self,
        id: Uuid,
        revision: i64,
        device_job_id: i32,
        next: DeliveryState,
    ) -> Result<Delivery, StorageError> {
        let mut delivery = self.delivery(id).await?;
        if delivery.state != DeliveryState::Unknown
            || delivery.revision != revision
            || device_job_id <= 0
            || !matches!(
                next,
                DeliveryState::Accepted
                    | DeliveryState::Completed
                    | DeliveryState::Cancelled
                    | DeliveryState::Aborted
            )
        {
            return Err(StorageError::Conflict);
        }
        delivery.device_job_id = Some(device_job_id);
        delivery.state = next;
        delivery.reason = Some("REMOTE_JOB_RECONCILED".into());
        delivery.lease_until = None;
        self.update_delivery(delivery, DeliveryState::Unknown, revision, None)
            .await
    }
    async fn update_delivery(
        &self,
        mut delivery: Delivery,
        previous: DeliveryState,
        revision: i64,
        run: Option<Uuid>,
    ) -> Result<Delivery, StorageError> {
        delivery.revision = revision.checked_add(1).ok_or(StorageError::Conflict)?;
        let mut tx = self.pool.begin().await?;
        let result=sqlx::query("UPDATE deliveries SET payload=?,state=?,revision=?,lease_until=? WHERE id=? AND state=? AND revision=? AND (? IS NULL OR (json_extract(payload,'$.runId')=? AND lease_until>?))")
            .bind(serde_json::to_string(&delivery)?).bind(delivery.state.label()).bind(delivery.revision).bind(delivery.lease_until)
            .bind(delivery.id.to_string()).bind(previous.label()).bind(revision).bind(run.map(|id|id.to_string())).bind(run.map(|id|id.to_string())).bind(now()).execute(&mut *tx).await?;
        if result.rows_affected() != 1 {
            return Err(StorageError::Conflict);
        }
        event(&mut tx, &delivery).await?;
        tx.commit().await?;
        Ok(delivery)
    }
    pub async fn recover_delivery_sends(&self) -> Result<Vec<Uuid>, StorageError> {
        let rows =
            sqlx::query("SELECT payload FROM deliveries WHERE state='SENDING' AND lease_until<=?")
                .bind(now())
                .fetch_all(&self.pool)
                .await?;
        let mut recovered = Vec::new();
        for row in rows {
            let mut delivery: Delivery = serde_json::from_str(row.get("payload"))?;
            let revision = delivery.revision;
            delivery.state = DeliveryState::Unknown;
            delivery.reason = Some("SENDER_LEASE_EXPIRED".into());
            match self
                .update_delivery(delivery.clone(), DeliveryState::Sending, revision, None)
                .await
            {
                Ok(_) => recovered.push(delivery.id),
                Err(StorageError::Conflict) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(recovered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rip_core::{ColorMode, DocumentFormat, Manifest};
    async fn fixture() -> (tempfile::TempDir, Repository, DeliveryTicket) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::open(&dir.path().join("queue.db"))
            .await
            .unwrap();
        let source = Job {
            id: Uuid::new_v4(),
            manifest: Manifest {
                name: "proof.pdf".into(),
                format: DocumentFormat::Pdf,
                dpi: 300,
                color_mode: ColorMode::Rgb,
                priority: 50,
                engine: Default::default(),
            },
            state: JobState::Completed,
            source_sha256: "a".repeat(64),
            created_at: now(),
            updated_at: now(),
            revision: 0,
            error_code: None,
            selected_engine: None,
            worker_run_id: None,
        };
        repo.insert(&source).await.unwrap();
        let ticket = DeliveryTicket {
            source_job: source.id,
            color_attempt_id: None,
            printer_id: "proof-printer".into(),
            artifact_sha256: "b".repeat(64),
            artifact_bytes: 1024,
            dpi: 300,
            document_type: "rgb_8".into(),
            media: "iso_a4_210x297mm".into(),
        };
        (dir, repo, ticket)
    }
    #[tokio::test]
    async fn persistent_idempotency_single_sender_and_completion_are_separate() {
        let (dir, repo, ticket) = fixture().await;
        let id = Uuid::new_v4();
        let (a, b) = tokio::join!(
            repo.enqueue_delivery(id, ticket.clone()),
            repo.enqueue_delivery(id, ticket.clone())
        );
        assert_eq!(a.unwrap().id, b.unwrap().id);
        let mut changed = ticket.clone();
        changed.dpi = 600;
        assert!(matches!(
            repo.enqueue_delivery(id, changed).await,
            Err(StorageError::Conflict)
        ));
        let run = Uuid::new_v4();
        let (a, b) = tokio::join!(
            repo.claim_delivery(id, 0, run),
            repo.claim_delivery(id, 0, Uuid::new_v4())
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        let sender = repo.delivery(id).await.unwrap().run_id.unwrap();
        assert!(
            repo.finish_delivery(id, Uuid::new_v4(), DeliveryState::Accepted, Some(42), None)
                .await
                .is_err()
        );
        let accepted = repo
            .finish_delivery(id, sender, DeliveryState::Accepted, Some(42), None)
            .await
            .unwrap();
        assert_eq!(accepted.state, DeliveryState::Accepted);
        assert!(
            repo.observe_delivery(id, 0, DeliveryState::Completed)
                .await
                .is_err()
        );
        drop(repo);
        let repo = Repository::open(&dir.path().join("queue.db"))
            .await
            .unwrap();
        assert_eq!(
            repo.enqueue_delivery(id, ticket)
                .await
                .unwrap()
                .device_job_id,
            Some(42)
        );
        assert!(repo.next_delivery().await.unwrap().is_none());
        assert_eq!(
            repo.observe_delivery(id, accepted.revision, DeliveryState::Completed)
                .await
                .unwrap()
                .state,
            DeliveryState::Completed
        );
        assert!(
            repo.observe_delivery(id, accepted.revision, DeliveryState::Cancelled)
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn expired_sends_become_unknown_and_never_reenter_queue() {
        let (dir, repo, ticket) = fixture().await;
        let id = Uuid::new_v4();
        repo.enqueue_delivery(id, ticket).await.unwrap();
        let run = Uuid::new_v4();
        let mut sending = repo.claim_delivery(id, 0, run).await.unwrap();
        sending.lease_until = Some(0);
        sqlx::query("UPDATE deliveries SET lease_until=0,payload=? WHERE id=?")
            .bind(serde_json::to_string(&sending).unwrap())
            .bind(id.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        drop(repo);
        let repo = Repository::open(&dir.path().join("queue.db"))
            .await
            .unwrap();
        assert_eq!(repo.recover_delivery_sends().await.unwrap(), vec![id]);
        assert_eq!(
            repo.delivery(id).await.unwrap().state,
            DeliveryState::Unknown
        );
        assert!(repo.next_delivery().await.unwrap().is_none());
        assert!(repo.claim_delivery(id, 2, run).await.is_err());
        assert!(
            repo.finish_delivery(id, run, DeliveryState::Accepted, Some(42), None)
                .await
                .is_err()
        );
        assert!(repo.recover_delivery_sends().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn cancellation_competes_with_sending_and_claims_only_once() {
        let (_dir, repo, ticket) = fixture().await;
        let id = Uuid::new_v4();
        repo.enqueue_delivery(id, ticket).await.unwrap();
        let (cancel, send) = tokio::join!(
            repo.request_delivery_cancel(id, 0),
            repo.claim_delivery(id, 0, Uuid::new_v4())
        );
        assert_eq!(usize::from(cancel.is_ok()) + usize::from(send.is_ok()), 1);
        let d = repo.delivery(id).await.unwrap();
        if d.state == DeliveryState::Sending {
            assert!(matches!(
                repo.request_delivery_cancel(id, d.revision).await,
                Err(StorageError::Conflict)
            ));
            repo.finish_delivery(
                id,
                d.run_id.unwrap(),
                DeliveryState::Accepted,
                Some(42),
                None,
            )
            .await
            .unwrap();
            let d = repo.delivery(id).await.unwrap();
            let requested = repo.request_delivery_cancel(id, d.revision).await.unwrap();
            let (a, b) = tokio::join!(
                repo.claim_delivery_cancel(id, requested.revision),
                repo.claim_delivery_cancel(id, requested.revision)
            );
            assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
            let d = repo.delivery(id).await.unwrap();
            assert!(repo.claim_delivery_cancel(id, d.revision).await.is_err());
        }
    }
}
