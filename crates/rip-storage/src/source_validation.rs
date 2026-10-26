//! Evidence from the trusted worker, distinct from immutable admission tickets.
use super::*;
use serde_json::{Value, json};
impl Repository {
    pub async fn record_source_validation(
        &self,
        id: Uuid,
        run: Uuid,
        hash: &str,
        bytes: u64,
        pages: u32,
        strict_pdf: bool,
    ) -> Result<Value, StorageError> {
        if !(1..=32 * 1024 * 1024).contains(&bytes) || !(1..=100).contains(&pages) {
            return Err(StorageError::Conflict);
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE jobs SET revision=revision WHERE id=?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        let payload: Option<String>=sqlx::query_scalar("SELECT j.payload FROM jobs j JOIN worker_leases l ON j.id=l.job_id WHERE j.id=? AND l.run_id=? AND l.expires_at>? AND json_extract(j.payload,'$.state')='RIPPING'")
            .bind(id.to_string()).bind(run.to_string()).bind(now()).fetch_optional(&mut *tx).await?;
        let job: Job = serde_json::from_str(&payload.ok_or(StorageError::Conflict)?)?;
        if job.worker_run_id != Some(run)
            || job.source_sha256 != hash
            || !rip_core::print::valid_digest(hash)
            || job.selected_engine
                != Some(
                    job.manifest
                        .engine
                        .resolve(job.manifest.format)
                        .map_err(|_| StorageError::Conflict)?,
                )
            || strict_pdf != (job.manifest.format == rip_core::DocumentFormat::Pdf)
        {
            return Err(StorageError::Conflict);
        }
        let report = json!({"schemaVersion":1,"profile":"org.jrip.source-validation-v1","jobId":id,"runId":run,
            "sourceSha256":hash,"sourceBytes":bytes,"pageCount":pages,"format":job.manifest.format,"selectedEngine":job.selected_engine,
            "allPagesInterpreted":true,"strictPdfInspection":strict_pdf,"scope":"bounded-page-content-interpretation","validatedAt":now()});
        sqlx::query("INSERT INTO source_validations(job_id,payload) VALUES(?,?)")
            .bind(id.to_string())
            .bind(serde_json::to_string(&report)?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(report)
    }
    /// Append once, while the same worker lease still owns the validated source.
    pub async fn record_source_profile_analysis(
        &self,
        id: Uuid,
        run: Uuid,
        analysis: Value,
    ) -> Result<Value, StorageError> {
        if analysis["schemaVersion"] != 1
            || analysis["profile"] != "org.jrip.source-color-profiles-v1"
            || analysis["automaticSelection"] != false
            || analysis["outputIntentIsInputProfile"] != false
            || !matches!(
                analysis["status"].as_str(),
                Some("DECLARATIONS_INSPECTED" | "INCOMPLETE")
            )
            || !analysis["profiles"]
                .as_array()
                .is_some_and(|profiles| profiles.len() <= 64)
            || serde_json::to_vec(&analysis)?.len() > 65536
        {
            return Err(StorageError::Conflict);
        }
        let digest = rip_digest::digest(&analysis).map_err(|_| StorageError::Conflict)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE jobs SET revision=revision WHERE id=?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        let raw:Option<String>=sqlx::query_scalar("SELECT v.payload FROM source_validations v JOIN worker_leases l ON v.job_id=l.job_id JOIN jobs j ON j.id=v.job_id WHERE v.job_id=? AND l.run_id=? AND l.expires_at>? AND json_extract(j.payload,'$.state')='RIPPING'")
            .bind(id.to_string()).bind(run.to_string()).bind(now()).fetch_optional(&mut *tx).await?;
        let mut report: Value = serde_json::from_str(&raw.ok_or(StorageError::Conflict)?)?;
        if report["runId"] != run.to_string()
            || report["sourceSha256"] != analysis["sourceSha256"]
            || report["sourceBytes"] != analysis["sourceBytes"]
            || report.get("colorProfileAnalysis").is_some()
        {
            return Err(StorageError::Conflict);
        }
        report["colorProfileAnalysis"] = json!({"analysis":analysis,"digestProfile":rip_digest::PROFILE,"analysisDigest":digest});
        sqlx::query("UPDATE source_validations SET payload=? WHERE job_id=?")
            .bind(report.to_string())
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(report)
    }
    pub async fn source_validation(&self, id: Uuid) -> Result<Value, StorageError> {
        let payload: Option<String> =
            sqlx::query_scalar("SELECT payload FROM source_validations WHERE job_id=?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;
        Ok(serde_json::from_str(
            &payload.ok_or(StorageError::NotFound)?,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn only_current_worker_can_record_hash_bound_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let repo = Repository::open_with_receipt_signer(&path, None)
            .await
            .unwrap();
        let mut job = Job {
            id: Uuid::new_v4(),
            manifest: serde_json::from_value(
                json!({"name":"test.pdf","format":"application/pdf","dpi":72}),
            )
            .unwrap(),
            state: JobState::Queued,
            source_sha256: "a".repeat(64),
            created_at: now(),
            updated_at: now(),
            revision: 0,
            error_code: None,
            selected_engine: Some(rip_core::EngineKind::MuPdf),
            worker_run_id: None,
        };
        repo.insert(&job).await.unwrap();
        let run = Uuid::new_v4();
        assert!(
            repo.record_source_validation(job.id, run, &job.source_sha256, 100, 2, true)
                .await
                .is_err()
        );
        job = repo.claim_queued(job, run, now() + 60).await.unwrap();
        assert!(
            repo.record_source_validation(job.id, Uuid::new_v4(), &job.source_sha256, 100, 2, true)
                .await
                .is_err()
        );
        assert!(
            repo.record_source_validation(job.id, run, &"b".repeat(64), 100, 2, true)
                .await
                .is_err()
        );
        assert!(
            repo.record_source_validation(job.id, run, &job.source_sha256, 100, 2, false)
                .await
                .is_err()
        );
        assert!(
            repo.record_source_validation(job.id, run, &job.source_sha256, 100, 101, true)
                .await
                .is_err()
        );
        sqlx::query("UPDATE worker_leases SET expires_at=0 WHERE job_id=?")
            .bind(job.id.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        assert!(
            repo.record_source_validation(job.id, run, &job.source_sha256, 100, 2, true)
                .await
                .is_err()
        );
        sqlx::query("UPDATE worker_leases SET expires_at=? WHERE job_id=?")
            .bind(now() + 60)
            .bind(job.id.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        let mut report = repo
            .record_source_validation(job.id, run, &job.source_sha256, 100, 2, true)
            .await
            .unwrap();
        assert_eq!(report["allPagesInterpreted"], true);
        assert!(
            repo.record_source_validation(job.id, run, &job.source_sha256, 100, 2, true)
                .await
                .is_err()
        );
        let analysis = json!({"schemaVersion":1,"profile":"org.jrip.source-color-profiles-v1","sourceSha256":job.source_sha256,"sourceBytes":100,"automaticSelection":false,"outputIntentIsInputProfile":false,"status":"DECLARATIONS_INSPECTED","profiles":[]});
        sqlx::query("UPDATE worker_leases SET expires_at=0 WHERE job_id=?")
            .bind(job.id.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        assert!(
            repo.record_source_profile_analysis(job.id, run, analysis.clone())
                .await
                .is_err()
        );
        sqlx::query("UPDATE worker_leases SET expires_at=? WHERE job_id=?")
            .bind(now() + 60)
            .bind(job.id.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        assert!(
            repo.record_source_profile_analysis(job.id, Uuid::new_v4(), analysis.clone())
                .await
                .is_err()
        );
        let mut bad = analysis.clone();
        bad["sourceSha256"] = json!("b".repeat(64));
        assert!(
            repo.record_source_profile_analysis(job.id, run, bad)
                .await
                .is_err()
        );
        report = repo
            .record_source_profile_analysis(job.id, run, analysis.clone())
            .await
            .unwrap();
        assert_eq!(
            report["colorProfileAnalysis"]["analysisDigest"],
            rip_digest::digest(&analysis).unwrap()
        );
        assert!(
            repo.record_source_profile_analysis(job.id, run, analysis)
                .await
                .is_err()
        );
        let reopened = Repository::open_with_receipt_signer(&path, None)
            .await
            .unwrap();
        assert_eq!(reopened.source_validation(job.id).await.unwrap(), report);
        repo.transition(job, JobState::Cancelled, None)
            .await
            .unwrap();
        assert_eq!(
            reopened
                .source_validation(report["jobId"].as_str().unwrap().parse().unwrap())
                .await
                .unwrap(),
            report
        );
    }
}
