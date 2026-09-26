use rip_core::{Job, JobState};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{path::Path, time::Duration};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("job_not_found")]
    NotFound,
    #[error("state_conflict")]
    Conflict,
}
#[derive(Clone)]
pub struct Repository {
    pool: SqlitePool,
}
impl Repository {
    pub async fn open(path: &Path) -> Result<Self, StorageError> {
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
        sqlx::query("CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, payload TEXT NOT NULL, priority INTEGER NOT NULL, created_at INTEGER NOT NULL, revision INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS job_events (sequence INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL, state TEXT NOT NULL, created_at INTEGER NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS ipp_jobs (ipp_id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL UNIQUE)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS worker_leases (job_id TEXT PRIMARY KEY, run_id TEXT NOT NULL, expires_at INTEGER NOT NULL)").execute(&pool).await?;
        Ok(Self { pool })
    }
    pub async fn insert(&self, job: &Job) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO jobs VALUES (?, ?, ?, ?, ?)")
            .bind(job.id.to_string())
            .bind(serde_json::to_string(job)?)
            .bind(job.manifest.priority as i64)
            .bind(job.created_at)
            .bind(job.revision)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO job_events(job_id,state,created_at) VALUES(?,?,?)")
            .bind(job.id.to_string())
            .bind(serde_json::to_string(&job.state)?)
            .bind(job.updated_at)
            .execute(&mut *tx)
            .await?;
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
        sqlx::query("INSERT INTO job_events(job_id,state,created_at) VALUES(?,?,?)")
            .bind(job.id.to_string())
            .bind(serde_json::to_string(&JobState::Ripping)?)
            .bind(job.updated_at)
            .execute(&mut *tx)
            .await?;
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
                    sqlx::query("INSERT INTO job_events(job_id,state,created_at) VALUES(?,?,?)")
                        .bind(job.id.to_string())
                        .bind(serde_json::to_string(&JobState::Failed)?)
                        .bind(job.updated_at)
                        .execute(&mut *tx)
                        .await?;
                    recovered.push(job.id);
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
        mut job: Job,
        next: JobState,
        error: Option<String>,
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
        sqlx::query("INSERT INTO job_events(job_id,state,created_at) VALUES(?,?,?)")
            .bind(job.id.to_string())
            .bind(serde_json::to_string(&next)?)
            .bind(job.updated_at)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(job)
    }
    /// Remove metadata only after a terminal state; original files are retained for explicit retention maintenance.
    pub async fn delete(&self, job: Job) -> Result<(), StorageError> {
        if !job.state.terminal() {
            return Err(StorageError::Conflict);
        }
        let result = sqlx::query("DELETE FROM jobs WHERE id=? AND revision=?")
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
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
#[cfg(test)]
mod tests {
    use super::*;
    use rip_core::{ColorMode, DocumentFormat, Manifest};
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
