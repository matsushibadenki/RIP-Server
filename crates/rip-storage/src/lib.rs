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
    pub async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Job>, StorageError> {
        let rows = sqlx::query("SELECT payload FROM jobs ORDER BY priority DESC, created_at ASC, id ASC LIMIT ? OFFSET ?").bind(limit.clamp(1,100)).bind(offset.max(0)).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|r| serde_json::from_str(r.get("payload")).map_err(StorageError::from))
            .collect()
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
        };
        let mut legacy = serde_json::to_value(&j).unwrap();
        legacy.as_object_mut().unwrap().remove("selected_engine");
        legacy["manifest"].as_object_mut().unwrap().remove("engine");
        let old_job: Job = serde_json::from_value(legacy).unwrap();
        assert_eq!(old_job.manifest.engine, rip_core::EnginePreference::Auto);
        assert_eq!(old_job.selected_engine, None);
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
}
