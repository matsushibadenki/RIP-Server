//! Durable delivery attempts. Expired sends become UNKNOWN, never QUEUED.
use super::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliveryTicket {
    pub source_job: Uuid,
    #[serde(
        default = "rip_core::print::default_copies",
        skip_serializing_if = "rip_core::print::single_copy"
    )]
    pub copies: u32,
    #[serde(default, skip_serializing_if = "rip_core::print::Sides::is_simplex")]
    pub sides: rip_core::print::Sides,
    #[serde(default, skip_serializing_if = "rip_core::print::SheetBack::is_normal")]
    pub sheet_back: rip_core::print::SheetBack,
    #[serde(default, skip_serializing_if = "rip_core::pages::PageRanges::is_all")]
    pub page_ranges: rip_core::pages::PageRanges,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceObservation {
    pub profile: String,
    /// IPP job-state: 3 pending through 9 completed.
    pub state: i32,
    pub reasons: Vec<String>,
    /// When this changed state/reason/counter set was observed, not a heartbeat.
    pub observed_at: i64,
    #[serde(
        default,
        skip_serializing_if = "rip_core::print::DeviceProgress::is_empty"
    )]
    pub progress: rip_core::print::DeviceProgress,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeviceQueryFailure {
    Unavailable,
    InvalidResponse,
    JobNotFound,
    RegistrationUnavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceQuery {
    pub failure: Option<DeviceQueryFailure>,
    /// Time this query outcome changed, not a per-poll heartbeat.
    pub changed_at: i64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_observation: Option<DeviceObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_query: Option<DeviceQuery>,
}
async fn event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    delivery: &Delivery,
) -> Result<(), StorageError> {
    let result =
        sqlx::query("INSERT INTO delivery_events(delivery_id,state,created_at) VALUES(?,?,?)")
            .bind(delivery.id.to_string())
            .bind(delivery.state.label())
            .bind(now())
            .execute(&mut **tx)
            .await?;
    let mut details = serde_json::json!({"revision":delivery.revision,"reasonCode":delivery.reason,"deviceJobId":delivery.device_job_id,"retryable":false});
    if let Some(observation) = &delivery.device_observation {
        details["deviceObservation"] = serde_json::to_value(observation)?;
    }
    if let Some(query) = &delivery.device_query {
        details["deviceQuery"] = serde_json::to_value(query)?;
    }
    sqlx::query("INSERT INTO delivery_event_details(sequence,payload) VALUES(?,?)")
        .bind(result.last_insert_rowid())
        .bind(serde_json::to_string(&details)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
impl Repository {
    pub async fn source_deliveries(
        &self,
        source: Uuid,
        limit: u32,
        before: Option<&str>,
    ) -> Result<serde_json::Value, StorageError> {
        if !(1..=100).contains(&limit) {
            return Err(StorageError::Conflict);
        }
        let cursor = before
            .map(|s| {
                if s.len() > 64 {
                    return Err(StorageError::Conflict);
                }
                let (time, id) = s.split_once(':').ok_or(StorageError::Conflict)?;
                let time = time
                    .parse::<i64>()
                    .ok()
                    .filter(|n| *n >= 0)
                    .ok_or(StorageError::Conflict)?;
                Ok((
                    time,
                    Uuid::parse_str(id)
                        .map_err(|_| StorageError::Conflict)?
                        .to_string(),
                ))
            })
            .transpose()?;
        let mut sql="SELECT payload,created_at FROM deliveries WHERE json_extract(payload,'$.ticket.sourceJob')=?".to_string();
        if cursor.is_some() {
            sql.push_str(" AND (created_at,id)<(?,?)");
        }
        sql.push_str(" ORDER BY created_at DESC,id DESC LIMIT ?");
        let mut query = sqlx::query(&sql).bind(source.to_string());
        if let Some((time, id)) = cursor {
            query = query.bind(time).bind(id);
        }
        let rows = query.bind(limit + 1).fetch_all(&self.pool).await?;
        let more = rows.len() > limit as usize;
        let mut items = Vec::new();
        let mut next = None;
        for row in rows.into_iter().take(limit as usize) {
            let mut value: serde_json::Value = serde_json::from_str(row.get("payload"))?;
            let created: i64 = row.get("created_at");
            value["createdAt"] = serde_json::json!(created);
            next = Some(format!(
                "{created}:{}",
                value["id"].as_str().ok_or(StorageError::Conflict)?
            ));
            items.push(value);
        }
        Ok(
            serde_json::json!({"schemaVersion":1,"deliveries":items,"limit":limit,"hasMore":more,"nextBefore":if more{next}else{None}}),
        )
    }
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
        self.enqueue_contract_delivery(id, ticket, approval, None)
            .await
    }
    pub async fn delivery_contract(&self, id: Uuid) -> Result<serde_json::Value, StorageError> {
        let payload: String =
            sqlx::query_scalar("SELECT payload FROM delivery_contracts WHERE delivery_id=?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?
                .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(&payload)?)
    }
    pub async fn enqueue_contract_delivery(
        &self,
        id: Uuid,
        ticket: DeliveryTicket,
        approval: Option<Uuid>,
        contract: Option<serde_json::Value>,
    ) -> Result<Delivery, StorageError> {
        if (ticket.sides.is_simplex() && !ticket.sheet_back.is_normal())
            || !(1..=rip_core::print::MAX_DELIVERY_COPIES).contains(&ticket.copies)
            || ticket.printer_id.is_empty()
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
            device_observation: None,
            device_query: None,
        };
        let mut tx = self.pool.begin().await?;
        if let Some(contract) = &contract {
            let hash = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&contract["ticket"])?)
            );
            if contract["ticketSha256"] != hash || contract["ticket"]["schemaVersion"] != 1 {
                return Err(StorageError::Conflict);
            }
            let revision = contract["ticket"]["sourceRevision"]
                .as_i64()
                .ok_or(StorageError::Conflict)?;
            if contract["ticket"]["sides"].as_str().unwrap_or("one-sided")
                != delivery.ticket.sides.as_str()
                || contract["ticket"]["copies"] != delivery.ticket.copies
                || contract["ticket"]["jobId"] != delivery.ticket.source_job.to_string()
                || contract["ticket"]["attemptId"] != id.to_string()
                || contract["ticket"]["delivery"] != serde_json::to_value(&delivery.ticket)?
            {
                return Err(StorageError::Conflict);
            }
            if !delivery.ticket.sides.is_simplex()
                && contract["ticket"]["sheetBack"] != delivery.ticket.sheet_back.as_str()
            {
                return Err(StorageError::Conflict);
            }
            if contract["ticket"]["pageSelection"] != delivery.ticket.page_ranges.label() {
                return Err(StorageError::Conflict);
            }
            if !delivery.ticket.page_ranges.is_all() {
                let total = contract["ticket"]["sourcePageCount"]
                    .as_u64()
                    .filter(|n| *n <= 100)
                    .ok_or(StorageError::Conflict)?;
                let indices = delivery
                    .ticket
                    .page_ranges
                    .indices(total as u32)
                    .map_err(|_| StorageError::Conflict)?;
                if contract["ticket"]["selectedSourcePages"]
                    != serde_json::json!(indices.iter().map(|i| i + 1).collect::<Vec<_>>())
                    || contract["ticket"]["selectedPageCount"] != indices.len()
                {
                    return Err(StorageError::Conflict);
                }
            }
            let valid: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=? AND json_extract(payload,'$.state')='COMPLETED' AND json_extract(payload,'$.revision')=?)")
                .bind(delivery.ticket.source_job.to_string()).bind(revision).fetch_one(&mut *tx).await?;
            if valid != 1 {
                return Err(StorageError::Conflict);
            }
        }
        if let Some(color) = delivery.ticket.color_attempt_id {
            let valid:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM color_attempts WHERE id=? AND source_job=? AND state='COMPLETED')").bind(color.to_string()).bind(delivery.ticket.source_job.to_string()).fetch_one(&mut *tx).await?;
            if valid != 1 {
                return Err(StorageError::Conflict);
            }
        }
        if let Some(approval) = approval {
            let allowed:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM execution_approvals WHERE id=? AND revoked=0 AND expires_at>? AND json_extract(payload,'$.requestId')=? AND json_extract(payload,'$.jobId')=? AND json_extract(payload,'$.printerId')=? AND json_extract(payload,'$.media')=? AND json_extract(payload,'$.colorAttemptId') IS ? AND coalesce(json_extract(payload,'$.sides'),'one-sided')=? AND coalesce(json_extract(payload,'$.copies'),1)=? AND json(coalesce(json_extract(payload,'$.pageRanges'),'[]'))=json(?))")
                .bind(approval.to_string()).bind(now()).bind(id.to_string()).bind(delivery.ticket.source_job.to_string())
                .bind(&delivery.ticket.printer_id).bind(&delivery.ticket.media).bind(delivery.ticket.color_attempt_id.map(|id|id.to_string())).bind(delivery.ticket.sides.as_str()).bind(delivery.ticket.copies).bind(serde_json::to_string(&delivery.ticket.page_ranges)?).fetch_one(&mut *tx).await?;
            if allowed != 1 {
                return Err(StorageError::Conflict);
            }
        }
        let result=sqlx::query("INSERT OR IGNORE INTO deliveries(id,payload,state,revision,lease_until,created_at) SELECT ?,?,'QUEUED',0,NULL,? FROM jobs WHERE id=? AND json_extract(payload,'$.state')='COMPLETED'")
            .bind(id.to_string()).bind(serde_json::to_string(&delivery)?).bind(now()).bind(delivery.ticket.source_job.to_string()).execute(&mut *tx).await?;
        if result.rows_affected() == 1 {
            if let Some(contract) = &contract {
                sqlx::query("INSERT INTO delivery_contracts(delivery_id,payload) VALUES(?,?)")
                    .bind(id.to_string())
                    .bind(serde_json::to_string(contract)?)
                    .execute(&mut *tx)
                    .await?;
            }
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
        if let Some(contract) = contract {
            let payload: String =
                sqlx::query_scalar("SELECT payload FROM delivery_contracts WHERE delivery_id=?")
                    .bind(id.to_string())
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or(StorageError::Conflict)?;
            if serde_json::from_str::<serde_json::Value>(&payload)? != contract {
                return Err(StorageError::Conflict);
            }
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
    /// Actual device samples never make a delivery sendable or infer page progress.
    pub async fn observe_delivery_device(
        &self,
        id: Uuid,
        revision: i64,
        device_job_id: i32,
        state: i32,
        reasons: &[String],
    ) -> Result<Delivery, StorageError> {
        self.observe_delivery_device_progress(
            id,
            revision,
            device_job_id,
            state,
            reasons,
            rip_core::print::DeviceProgress::default(),
        )
        .await
    }
    pub async fn observe_delivery_device_progress(
        &self,
        id: Uuid,
        revision: i64,
        device_job_id: i32,
        state: i32,
        reasons: &[String],
        progress: rip_core::print::DeviceProgress,
    ) -> Result<Delivery, StorageError> {
        let mut delivery = self.delivery(id).await?;
        if !progress.is_valid()
            || delivery.state != DeliveryState::Accepted
            || delivery.revision != revision
            || device_job_id <= 0
            || delivery.device_job_id != Some(device_job_id)
            || !(3..=9).contains(&state)
            || reasons.len() > 64
            || reasons
                .iter()
                .any(|r| r.is_empty() || r.len() > 255 || r.chars().any(char::is_control))
        {
            return Err(StorageError::Conflict);
        }
        let mut reasons = reasons.to_vec();
        reasons.sort();
        reasons.dedup();
        if delivery
            .device_query
            .as_ref()
            .is_some_and(|q| q.failure.is_none())
            && delivery
                .device_observation
                .as_ref()
                .is_some_and(|o| o.state == state && o.reasons == reasons && o.progress == progress)
        {
            return Ok(delivery);
        }
        if delivery
            .device_query
            .as_ref()
            .is_none_or(|q| q.failure.is_some())
        {
            delivery.device_query = Some(DeviceQuery {
                failure: None,
                changed_at: now(),
            });
        }
        delivery.device_observation = Some(DeviceObservation {
            profile: "org.jrip.device-job-observation-v1".into(),
            state,
            reasons,
            observed_at: now(),
            progress,
        });
        delivery.state = match state {
            7 => DeliveryState::Cancelled,
            8 => DeliveryState::Aborted,
            9 => DeliveryState::Completed,
            _ => DeliveryState::Accepted,
        };
        self.update_delivery(delivery, DeliveryState::Accepted, revision, None)
            .await
    }
    pub async fn fail_delivery_device_query(
        &self,
        id: Uuid,
        revision: i64,
        failure: DeviceQueryFailure,
    ) -> Result<Delivery, StorageError> {
        let mut delivery = self.delivery(id).await?;
        if delivery.state != DeliveryState::Accepted || delivery.revision != revision {
            return Err(StorageError::Conflict);
        }
        if delivery
            .device_query
            .as_ref()
            .is_some_and(|q| q.failure == Some(failure))
        {
            return Ok(delivery);
        }
        delivery.device_query = Some(DeviceQuery {
            failure: Some(failure),
            changed_at: now(),
        });
        self.update_delivery(delivery, DeliveryState::Accepted, revision, None)
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
        super::delivery_receipt::store(&mut tx, &delivery, self.receipt_signer.as_deref()).await?;
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
                page_ranges: Default::default(),
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
            page_ranges: Default::default(),
            sides: Default::default(),
            sheet_back: Default::default(),
            copies: 1,
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
    async fn device_observations_are_durable_deduplicated_and_bound_to_the_accepted_job() {
        use sha2::{Digest, Sha256};
        let (dir, repo, ticket) = fixture().await;
        let id = Uuid::new_v4();
        let run = Uuid::new_v4();
        let queued = repo.enqueue_delivery(id, ticket).await.unwrap();
        assert!(
            serde_json::to_value(&queued)
                .unwrap()
                .get("deviceObservation")
                .is_none()
        );
        assert!(
            repo.observe_delivery_device(id, 0, 42, 5, &[])
                .await
                .is_err()
        );
        repo.claim_delivery(id, 0, run).await.unwrap();
        let accepted = repo
            .finish_delivery(id, run, DeliveryState::Accepted, Some(42), None)
            .await
            .unwrap();
        let printing = repo
            .observe_delivery_device(
                id,
                accepted.revision,
                42,
                5,
                &[
                    "vendor-wait".into(),
                    "job-printing".into(),
                    "job-printing".into(),
                ],
            )
            .await
            .unwrap();
        assert_eq!(printing.state, DeliveryState::Accepted);
        assert_eq!(
            printing.device_observation.as_ref().unwrap().reasons,
            vec!["job-printing", "vendor-wait"]
        );
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery_events")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        let unchanged = repo
            .observe_delivery_device(
                id,
                printing.revision,
                42,
                5,
                &["job-printing".into(), "vendor-wait".into()],
            )
            .await
            .unwrap();
        assert_eq!(unchanged.revision, printing.revision);
        assert_eq!(unchanged.device_observation, printing.device_observation);
        assert_eq!(
            count,
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_events")
                .fetch_one(&repo.pool)
                .await
                .unwrap()
        );
        assert!(repo.delivery_result_receipt(id).await.is_err());
        assert!(
            repo.observe_delivery_device(id, accepted.revision, 42, 6, &[])
                .await
                .is_err()
        );
        assert!(
            repo.observe_delivery_device(id, printing.revision, 99, 6, &[])
                .await
                .is_err()
        );
        assert!(
            repo.observe_delivery_device(id, printing.revision, 42, 2, &[])
                .await
                .is_err()
        );
        assert!(
            repo.observe_delivery_device(id, printing.revision, 42, 6, &["x".repeat(256)])
                .await
                .is_err()
        );
        assert!(
            repo.observe_delivery_device(id, printing.revision, 42, 6, &["bad\nreason".into()])
                .await
                .is_err()
        );
        let stopped = repo
            .observe_delivery_device(id, printing.revision, 42, 6, &["media-needed".into()])
            .await
            .unwrap();
        let cancelled = repo
            .request_delivery_cancel(id, stopped.revision)
            .await
            .unwrap();
        let resumed = repo
            .observe_delivery_device(id, cancelled.revision, 42, 5, &["job-printing".into()])
            .await
            .unwrap();
        assert_eq!(resumed.reason.as_deref(), Some("CANCEL_REQUESTED"));
        let completed = repo
            .observe_delivery_device(
                id,
                resumed.revision,
                42,
                9,
                &["job-completed-successfully".into()],
            )
            .await
            .unwrap();
        assert_eq!(completed.state, DeliveryState::Completed);
        let receipt = repo.delivery_result_receipt(id).await.unwrap();
        assert_eq!(receipt["proof"]["result"]["deviceObservation"]["state"], 9);
        assert_eq!(
            receipt["resultHash"],
            format!(
                "sha256:{:x}",
                Sha256::digest(serde_json::to_vec(&receipt["proof"]["result"]).unwrap())
            )
        );
        let reopened = Repository::open(&dir.path().join("queue.db"))
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(reopened.delivery(id).await.unwrap()).unwrap(),
            serde_json::to_value(&completed).unwrap()
        );
        assert!(
            repo.observe_delivery_device(id, completed.revision, 42, 5, &[])
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn physical_counters_advance_without_state_change_and_bind_final_receipt() {
        use rip_core::print::DeviceProgress;
        let (dir, repo, ticket) = fixture().await;
        let id = Uuid::new_v4();
        let run = Uuid::new_v4();
        repo.enqueue_delivery(id, ticket).await.unwrap();
        repo.claim_delivery(id, 0, run).await.unwrap();
        let accepted = repo
            .finish_delivery(id, run, DeliveryState::Accepted, Some(42), None)
            .await
            .unwrap();
        let zero = DeviceProgress {
            impressions_completed: Some(0),
            media_sheets_completed: Some(0),
        };
        let first = repo
            .observe_delivery_device_progress(id, accepted.revision, 42, 5, &[], zero)
            .await
            .unwrap();
        let progress = DeviceProgress {
            impressions_completed: Some(8),
            media_sheets_completed: Some(4),
        };
        let advanced = repo
            .observe_delivery_device_progress(id, first.revision, 42, 5, &[], progress)
            .await
            .unwrap();
        assert_eq!(advanced.revision, first.revision + 1);
        assert_eq!(
            advanced.device_observation.as_ref().unwrap().progress,
            progress
        );
        let duplicate = repo
            .observe_delivery_device_progress(id, advanced.revision, 42, 5, &[], progress)
            .await
            .unwrap();
        assert_eq!(duplicate.revision, advanced.revision);
        assert!(
            repo.observe_delivery_device_progress(
                id,
                advanced.revision,
                42,
                5,
                &[],
                DeviceProgress {
                    impressions_completed: Some(u32::MAX),
                    media_sheets_completed: None
                }
            )
            .await
            .is_err()
        );
        let complete = repo
            .observe_delivery_device_progress(id, advanced.revision, 42, 9, &[], progress)
            .await
            .unwrap();
        let receipt = repo.delivery_result_receipt(id).await.unwrap();
        assert_eq!(
            receipt["proof"]["result"]["deviceObservation"]["progress"],
            serde_json::to_value(progress).unwrap()
        );
        let reopened = Repository::open(&dir.path().join("queue.db"))
            .await
            .unwrap();
        assert_eq!(
            reopened.delivery(id).await.unwrap().device_observation,
            complete.device_observation
        );
    }

    #[tokio::test]
    async fn query_failures_preserve_samples_cancel_requests_and_never_requeue() {
        let (dir, repo, ticket) = fixture().await;
        let id = Uuid::new_v4();
        let run = Uuid::new_v4();
        let queued = repo.enqueue_delivery(id, ticket).await.unwrap();
        assert!(
            serde_json::to_value(&queued)
                .unwrap()
                .get("deviceQuery")
                .is_none()
        );
        assert!(
            repo.fail_delivery_device_query(id, 0, DeviceQueryFailure::Unavailable)
                .await
                .is_err()
        );
        repo.claim_delivery(id, 0, run).await.unwrap();
        let accepted = repo
            .finish_delivery(id, run, DeliveryState::Accepted, Some(42), None)
            .await
            .unwrap();
        let failed = repo
            .fail_delivery_device_query(id, accepted.revision, DeviceQueryFailure::Unavailable)
            .await
            .unwrap();
        assert!(failed.device_observation.is_none());
        assert_eq!(failed.state, DeliveryState::Accepted);
        let first = repo
            .observe_delivery_device(id, failed.revision, 42, 5, &[])
            .await
            .unwrap();
        assert!(first.device_query.as_ref().unwrap().failure.is_none());
        let cancelled = repo
            .request_delivery_cancel(id, first.revision)
            .await
            .unwrap();
        let failure = repo
            .fail_delivery_device_query(id, cancelled.revision, DeviceQueryFailure::InvalidResponse)
            .await
            .unwrap();
        assert_eq!(failure.device_observation, first.device_observation);
        assert_eq!(failure.reason.as_deref(), Some("CANCEL_REQUESTED"));
        let duplicate = repo
            .fail_delivery_device_query(id, failure.revision, DeviceQueryFailure::InvalidResponse)
            .await
            .unwrap();
        assert_eq!(duplicate.revision, failure.revision);
        assert!(
            repo.fail_delivery_device_query(id, first.revision, DeviceQueryFailure::Unavailable)
                .await
                .is_err()
        );
        let reopened = Repository::open(&dir.path().join("queue.db"))
            .await
            .unwrap();
        assert_eq!(
            reopened.delivery(id).await.unwrap().device_query,
            failure.device_query
        );
        assert!(reopened.next_delivery().await.unwrap().is_none());
        assert!(reopened.delivery_result_receipt(id).await.is_err());
        let recovered = reopened
            .observe_delivery_device(id, failure.revision, 42, 5, &[])
            .await
            .unwrap();
        assert_eq!(recovered.revision, failure.revision + 1);
        assert_eq!(recovered.reason.as_deref(), Some("CANCEL_REQUESTED"));
        assert!(recovered.device_query.unwrap().failure.is_none());
        let completed = reopened
            .observe_delivery_device(id, recovered.revision, 42, 9, &[])
            .await
            .unwrap();
        assert!(
            reopened
                .fail_delivery_device_query(id, completed.revision, DeviceQueryFailure::JobNotFound)
                .await
                .is_err()
        );
        assert!(reopened.delivery_result_receipt(id).await.unwrap()["proof"]["result"]["deviceQuery"]["failure"].is_null());
    }

    #[tokio::test]
    async fn concurrent_device_observations_have_one_revision_winner() {
        let (_dir, repo, ticket) = fixture().await;
        let id = Uuid::new_v4();
        let run = Uuid::new_v4();
        repo.enqueue_delivery(id, ticket).await.unwrap();
        repo.claim_delivery(id, 0, run).await.unwrap();
        let accepted = repo
            .finish_delivery(id, run, DeliveryState::Accepted, Some(42), None)
            .await
            .unwrap();
        let (a, b) = tokio::join!(
            repo.observe_delivery_device(id, accepted.revision, 42, 5, &[]),
            repo.observe_delivery_device(id, accepted.revision, 42, 6, &[])
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        assert_eq!(
            repo.delivery(id).await.unwrap().revision,
            accepted.revision + 1
        );
    }
    #[tokio::test]
    async fn page_ranges_preserve_legacy_json_and_atomic_approval_binding() {
        let (_dir, repo, mut ticket) = fixture().await;
        let legacy = serde_json::to_value(&ticket).unwrap();
        assert!(legacy.get("pageRanges").is_none());
        assert!(
            serde_json::from_value::<DeliveryTicket>(legacy)
                .unwrap()
                .page_ranges
                .is_all()
        );
        let id = Uuid::new_v4();
        let approval = Uuid::new_v4();
        let payload = serde_json::json!({"requestId":id,"jobId":ticket.source_job,"printerId":ticket.printer_id,"media":ticket.media,"colorAttemptId":null});
        repo.save_approval(approval, &payload, now() + 300)
            .await
            .unwrap();
        ticket.page_ranges =
            serde_json::from_value(serde_json::json!([{"first":2,"last":3}])).unwrap();
        assert!(
            repo.enqueue_approved_delivery(id, ticket.clone(), Some(approval))
                .await
                .is_err()
        );
        assert!(repo.delivery(id).await.is_err());
        let approved = Uuid::new_v4();
        let mut exact = payload;
        exact["pageRanges"] = serde_json::to_value(&ticket.page_ranges).unwrap();
        repo.save_approval(approved, &exact, now() + 300)
            .await
            .unwrap();
        assert_eq!(
            repo.enqueue_approved_delivery(id, ticket.clone(), Some(approved))
                .await
                .unwrap()
                .ticket
                .page_ranges
                .label(),
            "2-3"
        );
        ticket.page_ranges = Default::default();
        assert!(repo.enqueue_delivery(id, ticket).await.is_err());
    }
    #[tokio::test]
    async fn sides_preserve_legacy_json_and_are_atomically_approval_bound() {
        let (_dir, repo, mut ticket) = fixture().await;
        let old = serde_json::to_value(&ticket).unwrap();
        assert!(old.get("sides").is_none());
        assert_eq!(
            serde_json::from_value::<DeliveryTicket>(old).unwrap().sides,
            rip_core::print::Sides::OneSided
        );
        let id = Uuid::new_v4();
        let approval = Uuid::new_v4();
        let mut payload = serde_json::json!({"requestId":id,"jobId":ticket.source_job,"printerId":ticket.printer_id,"media":ticket.media,"colorAttemptId":null});
        repo.save_approval(approval, &payload, now() + 300)
            .await
            .unwrap();
        ticket.sides = rip_core::print::Sides::TwoSidedLongEdge;
        assert!(
            repo.enqueue_approved_delivery(id, ticket.clone(), Some(approval))
                .await
                .is_err()
        );
        payload["sides"] = serde_json::json!(ticket.sides);
        let exact = Uuid::new_v4();
        repo.save_approval(exact, &payload, now() + 300)
            .await
            .unwrap();
        repo.enqueue_approved_delivery(id, ticket.clone(), Some(exact))
            .await
            .unwrap();
        ticket.sides = rip_core::print::Sides::TwoSidedShortEdge;
        assert!(
            repo.enqueue_approved_delivery(id, ticket, Some(exact))
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn copies_preserve_legacy_json_and_atomic_approval_and_replay_guards() {
        let (_dir, repo, mut ticket) = fixture().await;
        let legacy = serde_json::to_value(&ticket).unwrap();
        assert!(legacy.get("copies").is_none());
        assert_eq!(
            serde_json::from_value::<DeliveryTicket>(legacy.clone())
                .unwrap()
                .copies,
            1
        );
        let id = Uuid::new_v4();
        let approval = Uuid::new_v4();
        let old = serde_json::json!({"requestId":id,"jobId":ticket.source_job,"printerId":ticket.printer_id,"media":ticket.media,"colorAttemptId":null});
        repo.save_approval(approval, &old, now() + 300)
            .await
            .unwrap();
        ticket.copies = 2;
        assert!(
            repo.enqueue_approved_delivery(id, ticket.clone(), Some(approval))
                .await
                .is_err()
        );
        assert!(repo.delivery(id).await.is_err());
        let approved = Uuid::new_v4();
        let mut new = old;
        new["copies"] = serde_json::json!(2);
        repo.save_approval(approved, &new, now() + 300)
            .await
            .unwrap();
        assert_eq!(
            repo.enqueue_approved_delivery(id, ticket.clone(), Some(approved))
                .await
                .unwrap()
                .ticket
                .copies,
            2
        );
        ticket.copies = 1;
        assert!(repo.enqueue_delivery(id, ticket.clone()).await.is_err());
        ticket.copies = 0;
        assert!(repo.enqueue_delivery(Uuid::new_v4(), ticket).await.is_err());
        assert_eq!(repo.delivery(id).await.unwrap().ticket.copies, 2);
    }
    #[tokio::test]
    async fn terminal_receipts_are_signed_immutable_and_unknown_is_not_completion() {
        let (dir, _repo, ticket) = fixture().await;
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap();
        let signer = crate::receipts::ReceiptSigner::from_pkcs8(doc.as_ref()).unwrap();
        let public = signer.public_key().to_vec();
        let repo = Repository::open_with_receipt_signer(&dir.path().join("queue.db"), Some(signer))
            .await
            .unwrap();
        let id = Uuid::new_v4();
        repo.enqueue_delivery(id, ticket.clone()).await.unwrap();
        assert!(matches!(
            repo.delivery_result_receipt(id).await,
            Err(StorageError::NotFound)
        ));
        let run = Uuid::new_v4();
        repo.claim_delivery(id, 0, run).await.unwrap();
        let uncertain = repo
            .finish_delivery(
                id,
                run,
                DeliveryState::Unknown,
                None,
                Some("NETWORK_UNCERTAIN".into()),
            )
            .await
            .unwrap();
        assert!(matches!(
            repo.delivery_result_receipt(id).await,
            Err(StorageError::NotFound)
        ));
        repo.reconcile_delivery(id, uncertain.revision, 42, DeliveryState::Completed)
            .await
            .unwrap();
        let receipt = repo.delivery_result_receipt(id).await.unwrap();
        assert!(crate::receipts::verify(&receipt, &public));
        assert_eq!(receipt["status"], "succeeded");
        assert_eq!(
            receipt["proof"]["result"]["completionMeaning"],
            "device-confirmed"
        );
        assert_eq!(
            receipt["proof"]["result"]["confirmationSource"],
            "remote-job-reconciliation"
        );
        assert_eq!(
            receipt["proof"]["result"]["printedAppearanceVerified"],
            false
        );
        assert!(
            repo.reconcile_delivery(id, uncertain.revision, 42, DeliveryState::Cancelled)
                .await
                .is_err()
        );
        let reopened = Repository::open_with_receipt_signer(&dir.path().join("queue.db"), None)
            .await
            .unwrap();
        assert_eq!(reopened.delivery_result_receipt(id).await.unwrap(), receipt);
        let denied = Uuid::new_v4();
        repo.enqueue_delivery(denied, ticket.clone()).await.unwrap();
        let run = Uuid::new_v4();
        repo.claim_delivery(denied, 0, run).await.unwrap();
        repo.finish_delivery(
            denied,
            run,
            DeliveryState::Rejected,
            None,
            Some("REMOTE_REJECTED".into()),
        )
        .await
        .unwrap();
        assert_eq!(
            repo.delivery_result_receipt(denied).await.unwrap()["status"],
            "failed"
        );
        repo.revoke_receipt_key(
            receipt["signature"]["keyId"].as_str().unwrap(),
            "compromised",
            "admin",
        )
        .await
        .unwrap();
        let cancelled = Uuid::new_v4();
        repo.enqueue_delivery(cancelled, ticket).await.unwrap();
        repo.request_delivery_cancel(cancelled, 0).await.unwrap();
        let unsigned = repo.delivery_result_receipt(cancelled).await.unwrap();
        assert_eq!(unsigned["status"], "cancelled");
        assert_eq!(unsigned["proof"]["signed"], false);
        assert_eq!(unsigned["proof"]["signingBlocked"]["code"], "KEY_REVOKED");
        assert!(unsigned.get("signature").is_none());
        assert_eq!(repo.delivery_result_receipt(id).await.unwrap(), receipt);
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
