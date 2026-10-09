use super::*;
impl Repository {
    pub async fn retention_candidates(&self, cutoff: i64) -> Result<Vec<Uuid>, StorageError> {
        let ids:Vec<String>=sqlx::query_scalar("SELECT j.id FROM jobs j WHERE json_extract(j.payload,'$.state') IN ('COMPLETED','CANCELLED','FAILED') AND json_extract(j.payload,'$.updated_at')<=? AND NOT EXISTS(SELECT 1 FROM worker_leases l WHERE l.job_id=j.id) AND NOT EXISTS(SELECT 1 FROM color_attempts c WHERE c.source_job=j.id AND (c.state IN ('QUEUED','RUNNING') OR EXISTS(SELECT 1 FROM color_cleanup cc WHERE cc.attempt_id=c.id))) AND NOT EXISTS(SELECT 1 FROM deliveries d WHERE json_extract(d.payload,'$.ticket.sourceJob')=j.id AND d.state IN ('QUEUED','SENDING','ACCEPTED','UNKNOWN')) ORDER BY j.created_at,j.id LIMIT 100")
            .bind(cutoff).fetch_all(&self.pool).await?;
        ids.into_iter()
            .map(|id| Uuid::parse_str(&id).map_err(|_| StorageError::Conflict))
            .collect()
    }
    pub async fn orphan_retention_candidates(
        &self,
        cutoff: i64,
    ) -> Result<Vec<Uuid>, StorageError> {
        let ids:Vec<String>=sqlx::query_scalar("SELECT e.job_id FROM job_events e WHERE e.sequence=(SELECT MAX(last.sequence) FROM job_events last WHERE last.job_id=e.job_id) AND e.state IN ('\"COMPLETED\"','\"CANCELLED\"','\"FAILED\"') AND e.created_at<=? AND NOT EXISTS(SELECT 1 FROM jobs j WHERE j.id=e.job_id) AND NOT EXISTS(SELECT 1 FROM retention_tombstones t WHERE t.job_id=e.job_id) AND NOT EXISTS(SELECT 1 FROM worker_leases l WHERE l.job_id=e.job_id) AND NOT EXISTS(SELECT 1 FROM color_attempts c WHERE c.source_job=e.job_id AND (c.state IN ('QUEUED','RUNNING') OR EXISTS(SELECT 1 FROM color_cleanup cc WHERE cc.attempt_id=c.id))) AND NOT EXISTS(SELECT 1 FROM deliveries d WHERE json_extract(d.payload,'$.ticket.sourceJob')=e.job_id AND d.state IN ('QUEUED','SENDING','ACCEPTED','UNKNOWN')) ORDER BY e.sequence LIMIT 100")
            .bind(cutoff).fetch_all(&self.pool).await?;
        ids.into_iter()
            .map(|id| Uuid::parse_str(&id).map_err(|_| StorageError::Conflict))
            .collect()
    }
    pub async fn claim_retention(&self, id: Uuid, cutoff: i64) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        let result=sqlx::query("DELETE FROM jobs WHERE id=? AND json_extract(payload,'$.state') IN ('COMPLETED','CANCELLED','FAILED') AND json_extract(payload,'$.updated_at')<=? AND NOT EXISTS(SELECT 1 FROM worker_leases l WHERE l.job_id=jobs.id) AND NOT EXISTS(SELECT 1 FROM color_attempts c WHERE c.source_job=jobs.id AND (c.state IN ('QUEUED','RUNNING') OR EXISTS(SELECT 1 FROM color_cleanup cc WHERE cc.attempt_id=c.id))) AND NOT EXISTS(SELECT 1 FROM deliveries d WHERE json_extract(d.payload,'$.ticket.sourceJob')=jobs.id AND d.state IN ('QUEUED','SENDING','ACCEPTED','UNKNOWN'))")
            .bind(id.to_string()).bind(cutoff).execute(&mut *tx).await?;
        if result.rows_affected() != 1 {
            let orphan:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM job_events e WHERE e.job_id=? AND e.sequence=(SELECT MAX(last.sequence) FROM job_events last WHERE last.job_id=e.job_id) AND e.state IN ('\"COMPLETED\"','\"CANCELLED\"','\"FAILED\"') AND e.created_at<=? AND NOT EXISTS(SELECT 1 FROM jobs j WHERE j.id=e.job_id) AND NOT EXISTS(SELECT 1 FROM retention_tombstones t WHERE t.job_id=e.job_id) AND NOT EXISTS(SELECT 1 FROM worker_leases l WHERE l.job_id=e.job_id) AND NOT EXISTS(SELECT 1 FROM color_attempts c WHERE c.source_job=e.job_id AND (c.state IN ('QUEUED','RUNNING') OR EXISTS(SELECT 1 FROM color_cleanup cc WHERE cc.attempt_id=c.id))) AND NOT EXISTS(SELECT 1 FROM deliveries d WHERE json_extract(d.payload,'$.ticket.sourceJob')=e.job_id AND d.state IN ('QUEUED','SENDING','ACCEPTED','UNKNOWN')))")
                .bind(id.to_string()).bind(cutoff).fetch_one(&mut *tx).await?;
            if orphan != 1 {
                return Err(StorageError::Conflict);
            }
        }
        sqlx::query("DELETE FROM job_artifacts WHERE job_id=?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO retention_tombstones(job_id,created_at,finished_at) VALUES(?,?,NULL)",
        )
        .bind(id.to_string())
        .bind(now())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn terminal_deliveries_for_job(&self, job: Uuid) -> Result<Vec<Uuid>, StorageError> {
        let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM deliveries WHERE json_extract(payload,'$.ticket.sourceJob')=? AND state IN ('COMPLETED','CANCELLED','ABORTED','REJECTED')")
            .bind(job.to_string()).fetch_all(&self.pool).await?;
        ids.into_iter()
            .map(|id| Uuid::parse_str(&id).map_err(|_| StorageError::Conflict))
            .collect()
    }
    pub async fn pending_retention(&self) -> Result<Vec<Uuid>, StorageError> {
        let ids:Vec<String>=sqlx::query_scalar("SELECT job_id FROM retention_tombstones WHERE finished_at IS NULL ORDER BY created_at LIMIT 100").fetch_all(&self.pool).await?;
        ids.into_iter()
            .map(|id| Uuid::parse_str(&id).map_err(|_| StorageError::Conflict))
            .collect()
    }
    pub async fn finish_retention(&self, id: Uuid) -> Result<(), StorageError> {
        sqlx::query("UPDATE retention_tombstones SET finished_at=? WHERE job_id=?")
            .bind(now())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn prune_expired_admissions(
        &self,
        cutoff: i64,
        apply: bool,
    ) -> Result<serde_json::Value, StorageError> {
        let mut tx = self.pool.begin().await?;
        let preflights:i64=sqlx::query_scalar("SELECT COUNT(*) FROM print_preflights WHERE expires_at<=? AND NOT EXISTS(SELECT 1 FROM print_commits c WHERE c.preflight_id=print_preflights.id)").bind(cutoff).fetch_one(&mut *tx).await?;
        let approvals: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM execution_approvals WHERE expires_at<=?")
                .bind(cutoff)
                .fetch_one(&mut *tx)
                .await?;
        if apply {
            sqlx::query("DELETE FROM print_preflights WHERE expires_at<=? AND NOT EXISTS(SELECT 1 FROM print_commits c WHERE c.preflight_id=print_preflights.id)").bind(cutoff).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM execution_approvals WHERE expires_at<=?")
                .bind(cutoff)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(
            serde_json::json!({"expiredPreflights":preflights,"expiredApprovals":approvals,"applied":apply}),
        )
    }
}
