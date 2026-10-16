//! Trusted, measured artifact progress; independent of control revisions.
use super::*;
use serde_json::json;

impl Repository {
    pub async fn record_page_progress(
        &self,
        id: Uuid,
        run: Uuid,
        completed: u32,
        total: u32,
    ) -> Result<(), StorageError> {
        self.record_measured_page_progress(id, run, completed, Some(total), "artifact-verification")
            .await
    }

    pub async fn record_render_progress(
        &self,
        id: Uuid,
        run: Uuid,
        completed: u32,
        total: Option<u32>,
    ) -> Result<(), StorageError> {
        self.record_measured_page_progress(id, run, completed, total, "rendering")
            .await
    }

    async fn record_measured_page_progress(
        &self,
        id: Uuid,
        run: Uuid,
        completed: u32,
        total: Option<u32>,
        phase: &str,
    ) -> Result<(), StorageError> {
        if completed == 0
            || completed > 100
            || total.is_some_and(|total| total == 0 || total > 100 || completed > total)
        {
            return Err(StorageError::Conflict);
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE jobs SET revision=revision WHERE id=?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        let payload: Option<String> = sqlx::query_scalar("SELECT j.payload FROM jobs j JOIN worker_leases l ON j.id=l.job_id WHERE j.id=? AND l.run_id=? AND l.expires_at>? AND json_extract(j.payload,'$.state')='RIPPING'")
            .bind(id.to_string()).bind(run.to_string()).bind(now()).fetch_optional(&mut *tx).await?;
        let job: Job = serde_json::from_str(&payload.ok_or(StorageError::Conflict)?)?;
        if job.worker_run_id != Some(run) {
            return Err(StorageError::Conflict);
        }
        let previous: Option<String> = sqlx::query_scalar("SELECT d.payload FROM job_event_details d JOIN job_events e ON e.sequence=d.sequence WHERE e.job_id=? AND json_extract(d.payload,'$.progressPhase')=? ORDER BY e.sequence DESC LIMIT 1")
            .bind(id.to_string()).bind(phase).fetch_optional(&mut *tx).await?;
        if let Some(previous) = previous {
            let previous: serde_json::Value = serde_json::from_str(&previous)?;
            if previous["runId"] != run.to_string()
                || previous["totalPages"] != json!(total)
                || previous["completedPages"].as_u64() != Some(u64::from(completed - 1))
            {
                return Err(StorageError::Conflict);
            }
        } else if completed != 1 {
            return Err(StorageError::Conflict);
        }
        let event = sqlx::query("INSERT INTO job_events(job_id,state,created_at) VALUES(?,?,?)")
            .bind(id.to_string())
            .bind(serde_json::to_string(&job.state)?)
            .bind(now())
            .execute(&mut *tx)
            .await?;
        let details = json!({"schemaVersion":1,"stage":"RIPPING","revision":job.revision,
            "runId":run,"reasonCode":if phase == "rendering" { "PAGE_RENDERED" } else { "PAGE_ARTIFACT_VERIFIED" },"retryable":false,
            "progressPhase":phase,"completedPages":completed,"totalPages":total,
            "progressPercent":total.map(|total| completed * 100 / total),"completionKind":null});
        sqlx::query("INSERT INTO job_event_details(sequence,payload) VALUES(?,?)")
            .bind(event.last_insert_rowid())
            .bind(serde_json::to_string(&details)?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn progress_is_ordered_lease_bound_and_durable_without_changing_revision() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("db");
        let repo = Repository::open(&path).await.unwrap();
        let job: Job = serde_json::from_value(json!({"id":Uuid::new_v4(),"manifest":{"name":"test.pdf","format":"application/pdf"},"state":"QUEUED","source_sha256":"a".repeat(64),"created_at":now(),"updated_at":now(),"revision":0,"error_code":null,"selected_engine":"mupdf","worker_run_id":null})).unwrap();
        repo.insert(&job).await.unwrap();
        let run = Uuid::new_v4();
        let job = repo.claim_queued(job, run, now() + 60).await.unwrap();
        let pipeline = Uuid::new_v4();
        repo.create_pipeline(pipeline, job.id, job.revision, json!({}))
            .await
            .unwrap();
        assert!(repo.record_page_progress(job.id, run, 2, 2).await.is_err());
        assert!(
            repo.record_page_progress(job.id, Uuid::new_v4(), 1, 2)
                .await
                .is_err()
        );
        assert!(
            repo.record_render_progress(job.id, run, 2, None)
                .await
                .is_err()
        );
        repo.record_render_progress(job.id, run, 1, None)
            .await
            .unwrap();
        assert!(
            repo.record_render_progress(job.id, run, 1, None)
                .await
                .is_err()
        );
        assert!(
            repo.record_render_progress(job.id, run, 2, Some(2))
                .await
                .is_err()
        );
        repo.record_render_progress(job.id, run, 2, None)
            .await
            .unwrap();
        repo.record_page_progress(job.id, run, 1, 2).await.unwrap();
        assert!(repo.record_page_progress(job.id, run, 1, 2).await.is_err());
        assert!(repo.record_page_progress(job.id, run, 2, 3).await.is_err());
        sqlx::query("UPDATE worker_leases SET expires_at=? WHERE job_id=?")
            .bind(now() - 1)
            .bind(job.id.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        assert!(repo.record_page_progress(job.id, run, 2, 2).await.is_err());
        repo.renew_lease(job.id, run, now() + 60).await.unwrap();
        repo.record_page_progress(job.id, run, 2, 2).await.unwrap();
        assert_eq!(repo.get(job.id).await.unwrap().revision, job.revision);
        let reopened = Repository::open(&path).await.unwrap();
        let events = reopened
            .unified_pipeline_events(pipeline, 0, 100)
            .await
            .unwrap();
        let rendering: Vec<_> = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["progressPhase"] == "rendering")
            .collect();
        assert_eq!(rendering.len(), 2);
        assert!(rendering[1]["totalPages"].is_null());
        assert!(rendering[1]["progressPercent"].is_null());
        assert!(rendering[1]["completionMeaning"].is_null());
        let pages: Vec<_> = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["progressPhase"] == "artifact-verification")
            .collect();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0]["progressPercent"], 50);
        assert_eq!(pages[1]["progressPercent"], 100);
        assert!(pages[1]["completionMeaning"].is_null());
        repo.transition(job.clone(), JobState::Cancelled, None)
            .await
            .unwrap();
        assert!(repo.record_page_progress(job.id, run, 2, 2).await.is_err());
    }
}
