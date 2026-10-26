//! A single purge unit owns source, raster, color outputs and delivery spool.
use super::*;
use serde_json::{Value, json};
pub(crate) async fn install(pool: &SqlitePool) -> Result<(), StorageError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS artifact_retention_holds(job_id TEXT PRIMARY KEY,retain_until INTEGER NOT NULL,revision INTEGER NOT NULL,payload TEXT NOT NULL)").execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS artifact_retention_revisions(job_id TEXT NOT NULL,revision INTEGER NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(job_id,revision))").execute(pool).await?;
    sqlx::query("CREATE VIEW IF NOT EXISTS artifact_retention_dependencies AS
        SELECT c.source_job AS source_job,'color' AS kind,c.id AS id,
          (c.state NOT IN ('COMPLETED','FAILED','CANCELLED') OR EXISTS(SELECT 1 FROM color_cleanup x WHERE x.attempt_id=c.id)) AS active,
          CASE WHEN json_type(c.payload,'$.updatedAt')='integer' THEN json_extract(c.payload,'$.updatedAt') ELSE NULL END AS activity_at FROM color_attempts c
        UNION ALL SELECT json_extract(d.payload,'$.ticket.sourceJob'),'delivery',d.id,
          d.state NOT IN ('COMPLETED','CANCELLED','ABORTED','REJECTED'),
          (SELECT e.created_at FROM delivery_events e WHERE e.delivery_id=d.id ORDER BY e.sequence DESC LIMIT 1) FROM deliveries d
        UNION ALL SELECT p.source_job,'pipeline',p.id,p.state NOT IN ('COMPLETED','FAILED','CANCELLED'),
          (SELECT CASE WHEN json_type(e.payload,'$.createdAt')='integer' THEN json_extract(e.payload,'$.createdAt') ELSE NULL END FROM pipeline_events e WHERE e.pipeline_id=p.id ORDER BY e.sequence DESC LIMIT 1) FROM print_pipelines p").execute(pool).await?;
    for sql in [
        "CREATE INDEX IF NOT EXISTS artifact_color_source ON color_attempts(source_job)",
        "CREATE INDEX IF NOT EXISTS artifact_pipeline_source ON print_pipelines(source_job)",
        "CREATE INDEX IF NOT EXISTS artifact_delivery_source ON deliveries(json_extract(payload,'$.ticket.sourceJob'))",
    ] {
        sqlx::query(sql).execute(pool).await?;
    }
    Ok(())
}
// These aliases/placeholders are constants selected by internal callers, never user input.
pub(crate) fn guard(job: &str, cutoff: &str, clock: &str) -> String {
    format!(
        "NOT EXISTS(SELECT 1 FROM artifact_retention_dependencies a WHERE a.source_job={job} AND (a.active OR typeof(a.activity_at)!='integer' OR a.activity_at<0 OR a.activity_at>{cutoff})) AND NOT EXISTS(SELECT 1 FROM artifact_retention_holds h WHERE h.job_id={job} AND (typeof(h.retain_until)!='integer' OR h.retain_until>{clock}))"
    )
}
fn empty_policy(id: Uuid) -> Value {
    json!({"profile":"org.jrip.artifact-retention-policy-v1","jobId":id,"revision":0,"retainUntil":null,"scope":["source","raster","color-managed","delivery-spool"],"meaning":"minimum-retention-not-automatic-deletion"})
}
impl Repository {
    pub async fn artifact_retention_policy(&self, id: Uuid) -> Result<Value, StorageError> {
        let known:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1 UNION ALL SELECT 1 FROM job_events WHERE job_id=?1)").bind(id.to_string()).fetch_one(&self.pool).await?;
        if known != 1 {
            return Err(StorageError::NotFound);
        }
        let payload: Option<String> =
            sqlx::query_scalar("SELECT payload FROM artifact_retention_holds WHERE job_id=?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;
        Ok(match payload {
            Some(p) => serde_json::from_str(&p)?,
            None => empty_policy(id),
        })
    }
    pub async fn extend_artifact_retention(
        &self,
        id: Uuid,
        expected: i64,
        until: i64,
        actor: &str,
        reason: &str,
    ) -> Result<Value, StorageError> {
        let clock = now();
        if expected < 0
            || until <= clock
            || until > clock + 3650 * 86400
            || actor.trim().is_empty()
            || actor.len() > 256
            || reason.trim().is_empty()
            || reason.len() > 1024
        {
            return Err(StorageError::Conflict);
        }
        let mut tx = self.pool.begin().await?;
        // Serialize with claim_retention before examining the hold or purge journal.
        sqlx::query("UPDATE jobs SET revision=revision WHERE id=?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        let known:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1 UNION ALL SELECT 1 FROM job_events WHERE job_id=?1) AND NOT EXISTS(SELECT 1 FROM retention_tombstones WHERE job_id=?1)").bind(id.to_string()).fetch_one(&mut *tx).await?;
        if known != 1 {
            return Err(StorageError::Conflict);
        }
        let old = sqlx::query(
            "SELECT revision,retain_until FROM artifact_retention_holds WHERE job_id=?",
        )
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let (revision, previous) = old.map_or((0, 0), |r| {
            (r.get::<i64, _>("revision"), r.get::<i64, _>("retain_until"))
        });
        if expected != revision || until < previous {
            return Err(StorageError::Conflict);
        }
        if until == previous {
            let payload: String =
                sqlx::query_scalar("SELECT payload FROM artifact_retention_holds WHERE job_id=?")
                    .bind(id.to_string())
                    .fetch_one(&mut *tx)
                    .await?;
            tx.commit().await?;
            return Ok(serde_json::from_str(&payload)?);
        }
        let correlation = Uuid::new_v4();
        let mut policy = empty_policy(id);
        policy["correlationId"] = json!(correlation);
        policy["revision"] = json!(revision.checked_add(1).ok_or(StorageError::Conflict)?);
        policy["retainUntil"] = json!(until);
        policy["previousRetainUntil"] = if previous > 0 {
            json!(previous)
        } else {
            Value::Null
        };
        policy["updatedAt"] = json!(clock);
        policy["actor"] = json!(actor);
        policy["reason"] = json!(reason);
        sqlx::query("INSERT INTO artifact_retention_holds(job_id,retain_until,revision,payload) VALUES(?,?,?,?) ON CONFLICT(job_id) DO UPDATE SET retain_until=excluded.retain_until,revision=excluded.revision,payload=excluded.payload")
            .bind(id.to_string()).bind(until).bind(revision+1).bind(policy.to_string()).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO artifact_retention_revisions(job_id,revision,payload) VALUES(?,?,?)",
        )
        .bind(id.to_string())
        .bind(revision + 1)
        .bind(policy.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO audit_records(correlation_id,identity,operation,target,started_at,finished_at,status) VALUES(?,?,?,?,?,?,200)")
            .bind(correlation.to_string()).bind(actor).bind("artifact-retention-extend").bind(id.to_string()).bind(clock).bind(clock).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn old_job(repo: &Repository) -> Job {
        let job = Job {
            id: Uuid::new_v4(),
            manifest: serde_json::from_value(json!({"name":"test.pdf","format":"application/pdf"}))
                .unwrap(),
            state: JobState::Completed,
            source_sha256: "a".repeat(64),
            created_at: now() - 3 * 86400,
            updated_at: now() - 3 * 86400,
            revision: 0,
            error_code: None,
            selected_engine: None,
            worker_run_id: None,
        };
        repo.insert(&job).await.unwrap();
        job
    }
    async fn dependency(repo: &Repository, job: Uuid, kind: &str, clock: Option<i64>) -> Uuid {
        let id = Uuid::new_v4();
        match kind {
            "color" => {
                sqlx::query("INSERT INTO color_attempts(id,source_job,ticket,payload,state,revision,created_at) VALUES(?,?,'{}',?,'COMPLETED',0,0)").bind(id.to_string()).bind(job.to_string()).bind(json!({"updatedAt":clock}).to_string()).execute(&repo.pool).await.unwrap();
            }
            "delivery" => {
                sqlx::query("INSERT INTO deliveries(id,payload,state,revision,created_at) VALUES(?,?,'COMPLETED',0,0)").bind(id.to_string()).bind(json!({"ticket":{"sourceJob":job}}).to_string()).execute(&repo.pool).await.unwrap();
                if let Some(clock) = clock {
                    sqlx::query("INSERT INTO delivery_events(delivery_id,state,created_at) VALUES(?,'COMPLETED',?)").bind(id.to_string()).bind(clock).execute(&repo.pool).await.unwrap();
                }
            }
            "pipeline" => {
                sqlx::query("INSERT INTO print_pipelines(id,source_job,ticket,payload,state,revision,created_at) VALUES(?,?,'{}','{}','COMPLETED',0,0)").bind(id.to_string()).bind(job.to_string()).execute(&repo.pool).await.unwrap();
                sqlx::query("INSERT INTO pipeline_events(pipeline_id,payload) VALUES(?,?)")
                    .bind(id.to_string())
                    .bind(json!({"createdAt":clock}).to_string())
                    .execute(&repo.pool)
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        id
    }
    #[tokio::test]
    async fn recent_terminal_dependencies_and_unknown_ages_protect_source_and_orphans() {
        let root = tempfile::tempdir().unwrap();
        let repo = Repository::open_with_receipt_signer(&root.path().join("db"), None)
            .await
            .unwrap();
        let cutoff = now() - 86400;
        for kind in ["color", "delivery", "pipeline"] {
            let job = old_job(&repo).await;
            dependency(&repo, job.id, kind, Some(now())).await;
            assert!(
                !repo
                    .retention_candidates(cutoff)
                    .await
                    .unwrap()
                    .contains(&job.id),
                "{kind}"
            );
            assert!(repo.claim_retention(job.id, cutoff).await.is_err());
            repo.delete(job.clone()).await.unwrap();
            assert!(
                !repo
                    .orphan_retention_candidates(cutoff)
                    .await
                    .unwrap()
                    .contains(&job.id)
            );
            assert!(repo.claim_retention(job.id, cutoff).await.is_err());
            let old = old_job(&repo).await;
            dependency(&repo, old.id, kind, Some(now() - 2 * 86400)).await;
            assert!(
                repo.retention_candidates(cutoff)
                    .await
                    .unwrap()
                    .contains(&old.id)
            );
            repo.claim_retention(old.id, cutoff).await.unwrap();
            let missing = old_job(&repo).await;
            dependency(&repo, missing.id, kind, None).await;
            assert!(
                !repo
                    .retention_candidates(cutoff)
                    .await
                    .unwrap()
                    .contains(&missing.id)
            );
            assert!(repo.claim_retention(missing.id, cutoff).await.is_err());
            // JSON booleans become SQLite integers unless their original type is checked.
            if kind == "color" {
                sqlx::query("UPDATE color_attempts SET payload=json_set(payload,'$.updatedAt',json('true')) WHERE source_job=?").bind(missing.id.to_string()).execute(&repo.pool).await.unwrap();
                assert!(repo.claim_retention(missing.id, cutoff).await.is_err());
            } else if kind == "pipeline" {
                sqlx::query("UPDATE pipeline_events SET payload=json_set(payload,'$.createdAt',json('true')) WHERE pipeline_id IN (SELECT id FROM print_pipelines WHERE source_job=?)").bind(missing.id.to_string()).execute(&repo.pool).await.unwrap();
                assert!(repo.claim_retention(missing.id, cutoff).await.is_err());
            }
        }
    }
    #[tokio::test]
    async fn preview_does_not_authorize_purge_after_new_artifact_and_cleanup_keeps_pin() {
        let root = tempfile::tempdir().unwrap();
        let repo = Repository::open_with_receipt_signer(&root.path().join("db"), None)
            .await
            .unwrap();
        let cutoff = now() - 86400;
        let job = old_job(&repo).await;
        assert!(
            repo.retention_candidates(cutoff)
                .await
                .unwrap()
                .contains(&job.id)
        );
        let color = dependency(&repo, job.id, "color", Some(now())).await;
        assert!(repo.claim_retention(job.id, cutoff).await.is_err());
        assert!(repo.get(job.id).await.is_ok());
        sqlx::query(
            "UPDATE color_attempts SET payload=json_set(payload,'$.updatedAt',?) WHERE id=?",
        )
        .bind(now() - 2 * 86400)
        .bind(color.to_string())
        .execute(&repo.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO color_cleanup(attempt_id) VALUES(?)")
            .bind(color.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        assert!(repo.claim_retention(job.id, cutoff).await.is_err());
        sqlx::query("DELETE FROM color_cleanup WHERE attempt_id=?")
            .bind(color.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE color_attempts SET state='UNRECOGNIZED' WHERE id=?")
            .bind(color.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        assert!(repo.claim_retention(job.id, cutoff).await.is_err());
        sqlx::query("UPDATE color_attempts SET state='COMPLETED' WHERE id=?")
            .bind(color.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        repo.claim_retention(job.id, cutoff).await.unwrap();
    }
    #[tokio::test]
    async fn holds_are_monotonic_audited_atomic_and_expire_without_erasing_history() {
        let root = tempfile::tempdir().unwrap();
        let repo = Repository::open_with_receipt_signer(&root.path().join("db"), None)
            .await
            .unwrap();
        let cutoff = now() - 86400;
        let job = old_job(&repo).await;
        let until = now() + 3600;
        let first = repo
            .extend_artifact_retention(job.id, 0, until, "operator", "first reason")
            .await
            .unwrap();
        assert_eq!(
            repo.extend_artifact_retention(job.id, 1, until, "operator", "repeat")
                .await
                .unwrap(),
            first
        );
        assert!(
            repo.extend_artifact_retention(job.id, 0, until + 1, "operator", "stale")
                .await
                .is_err()
        );
        assert!(
            repo.extend_artifact_retention(job.id, 1, until - 1, "operator", "shorten")
                .await
                .is_err()
        );
        sqlx::query("CREATE TRIGGER reject_retention_audit BEFORE INSERT ON audit_records BEGIN SELECT RAISE(ABORT,'audit unavailable'); END").execute(&repo.pool).await.unwrap();
        assert!(
            repo.extend_artifact_retention(job.id, 1, until + 60, "operator", "must rollback")
                .await
                .is_err()
        );
        assert_eq!(repo.artifact_retention_policy(job.id).await.unwrap(), first);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artifact_retention_revisions")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
        sqlx::query("DROP TRIGGER reject_retention_audit")
            .execute(&repo.pool)
            .await
            .unwrap();
        let second = repo
            .extend_artifact_retention(job.id, 1, until + 60, "operator", "extension")
            .await
            .unwrap();
        assert_eq!(second["previousRetainUntil"], until);
        let audit = repo.audit_records(0, 100).await.unwrap();
        assert!(
            audit["records"]
                .as_array()
                .unwrap()
                .iter()
                .any(|record| record["correlationId"] == second["correlationId"])
        );
        repo.delete(job.clone()).await.unwrap();
        assert!(repo.claim_retention(job.id, cutoff).await.is_err());
        // Advance the fixture's hold clock without sleeping or shortening via the API.
        sqlx::query("UPDATE artifact_retention_holds SET retain_until=? WHERE job_id=?")
            .bind(now() - 1)
            .bind(job.id.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        assert!(
            repo.orphan_retention_candidates(cutoff)
                .await
                .unwrap()
                .contains(&job.id)
        );
        repo.claim_retention(job.id, cutoff).await.unwrap();
        assert!(
            repo.extend_artifact_retention(job.id, 2, until + 120, "operator", "too late")
                .await
                .is_err()
        );
        let history: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artifact_retention_revisions")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(history, 2);
        assert_eq!(
            repo.audit_records(0, 100).await.unwrap()["records"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    #[tokio::test]
    async fn legacy_missing_source_history_cannot_bypass_hold_as_abandoned_upload() {
        let root = tempfile::tempdir().unwrap();
        let repo = Repository::open_with_receipt_signer(&root.path().join("db"), None)
            .await
            .unwrap();
        let job = old_job(&repo).await;
        repo.extend_artifact_retention(job.id, 0, now() + 3600, "operator", "legacy hold")
            .await
            .unwrap();
        repo.delete(job.clone()).await.unwrap();
        sqlx::query("DELETE FROM job_events WHERE job_id=?")
            .bind(job.id.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM job_tickets WHERE job_id=?")
            .bind(job.id.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        assert!(repo.upload_has_registration(job.id).await.unwrap());
        assert!(repo.claim_upload_retention(job.id).await.is_err());
        // Even expiry must not invent terminal source history for upload GC.
        sqlx::query("DELETE FROM artifact_retention_holds WHERE job_id=?")
            .bind(job.id.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        assert!(repo.upload_has_registration(job.id).await.unwrap());
        assert!(repo.claim_upload_retention(job.id).await.is_err());
    }
    #[tokio::test]
    async fn hold_and_purge_claim_have_exactly_one_winner() {
        let root = tempfile::tempdir().unwrap();
        let repo = Repository::open_with_receipt_signer(&root.path().join("db"), None)
            .await
            .unwrap();
        let cutoff = now() - 86400;
        let job = old_job(&repo).await;
        let (hold, purge) = tokio::join!(
            repo.extend_artifact_retention(job.id, 0, now() + 3600, "operator", "concurrent hold"),
            repo.claim_retention(job.id, cutoff)
        );
        assert_ne!(hold.is_ok(), purge.is_ok());
        if hold.is_ok() {
            assert!(repo.get(job.id).await.is_ok());
            assert!(repo.pending_retention().await.unwrap().is_empty());
        } else {
            assert!(repo.get(job.id).await.is_err());
            assert_eq!(repo.pending_retention().await.unwrap(), vec![job.id]);
        }
    }
}
