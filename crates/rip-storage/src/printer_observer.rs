//! Read-only observer scheduling is independent of print delivery/retry state.
use super::*;
pub const LEASE_SECONDS: i64 = 60;
pub struct ObserverLease {
    id: String,
    binding: String,
    run: Uuid,
    failures: u32,
}
fn valid(id: &str, binding: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        && rip_core::print::valid_digest(binding)
}
impl Repository {
    pub async fn claim_printer_observer(
        &self,
        id: &str,
        binding: &str,
        run: Uuid,
    ) -> Result<Option<ObserverLease>, StorageError> {
        self.claim_printer_observer_at(id, binding, run, now())
            .await
    }
    async fn claim_printer_observer_at(
        &self,
        id: &str,
        binding: &str,
        run: Uuid,
        at: i64,
    ) -> Result<Option<ObserverLease>, StorageError> {
        if !valid(id, binding) || !(0..=i64::MAX - 86400).contains(&at) {
            return Err(StorageError::Conflict);
        }
        // Idle passes remain reads. The final UPDATE still fences competing claimants.
        let not_due:Option<i64>=sqlx::query_scalar("SELECT 1 FROM printer_observer_schedule WHERE printer_id=? AND binding=? AND (next_at>? OR (run_id IS NOT NULL AND lease_until>?))")
            .bind(id).bind(binding).bind(at).bind(at).fetch_optional(&self.pool).await?;
        if not_due.is_some() {
            return Ok(None);
        }
        sqlx::query("INSERT OR IGNORE INTO printer_observer_schedule(printer_id,binding,next_at,failures) VALUES(?,?,0,0)").bind(id).bind(binding).execute(&self.pool).await?;
        let failures:Option<i64>=sqlx::query_scalar("UPDATE printer_observer_schedule SET run_id=?,lease_until=? WHERE printer_id=? AND binding=? AND next_at<=? AND (run_id IS NULL OR lease_until<=?) RETURNING failures")
            .bind(run.to_string()).bind(at+LEASE_SECONDS).bind(id).bind(binding).bind(at).bind(at).fetch_optional(&self.pool).await?;
        failures
            .map(|n| {
                let failures = u32::try_from(n)
                    .ok()
                    .filter(|n| *n <= 16)
                    .ok_or(StorageError::Conflict)?;
                Ok(ObserverLease {
                    id: id.into(),
                    binding: binding.into(),
                    run,
                    failures,
                })
            })
            .transpose()
    }
    pub async fn finish_printer_observer(
        &self,
        lease: &ObserverLease,
        success: bool,
        interval: u32,
        max_backoff: u32,
    ) -> Result<bool, StorageError> {
        self.finish_printer_observer_at(lease, success, interval, max_backoff, now())
            .await
    }
    async fn finish_printer_observer_at(
        &self,
        lease: &ObserverLease,
        success: bool,
        interval: u32,
        max_backoff: u32,
        at: i64,
    ) -> Result<bool, StorageError> {
        if !(5..=3600).contains(&interval)
            || !(interval..=86400).contains(&max_backoff)
            || !(0..=i64::MAX - 86400).contains(&at)
        {
            return Err(StorageError::Conflict);
        }
        let failures = if success {
            0
        } else {
            lease.failures.saturating_add(1).min(16)
        };
        let delay = if success {
            interval
        } else {
            (u64::from(interval) << failures).min(u64::from(max_backoff)) as u32
        };
        let changed=sqlx::query("UPDATE printer_observer_schedule SET next_at=?,failures=?,last_finished_at=?,last_success_at=CASE WHEN ? THEN ? ELSE last_success_at END,run_id=NULL,lease_until=NULL WHERE printer_id=? AND binding=? AND run_id=? AND lease_until>?")
            .bind(at+i64::from(delay)).bind(i64::from(failures)).bind(at).bind(success).bind(at).bind(&lease.id).bind(&lease.binding).bind(lease.run.to_string()).bind(at).execute(&self.pool).await?;
        Ok(changed.rows_affected() == 1)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn schedules_fence_workers_back_off_recover_and_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite");
        let repo = Repository::open(&path).await.unwrap();
        let binding = "a".repeat(64);
        let (a, b) = tokio::join!(
            repo.claim_printer_observer_at("test", &binding, Uuid::new_v4(), 100),
            repo.claim_printer_observer_at("test", &binding, Uuid::new_v4(), 100)
        );
        let a = a.unwrap();
        let b = b.unwrap();
        assert_ne!(a.is_some(), b.is_some());
        let lease = a.or(b).unwrap();
        assert!(
            repo.finish_printer_observer_at(&lease, false, 5, 20, 101)
                .await
                .unwrap()
        );
        assert!(
            repo.claim_printer_observer_at("test", &binding, Uuid::new_v4(), 110)
                .await
                .unwrap()
                .is_none()
        );
        let repo = Repository::open(&path).await.unwrap();
        let second = repo
            .claim_printer_observer_at("test", &binding, Uuid::new_v4(), 111)
            .await
            .unwrap()
            .unwrap();
        assert!(
            repo.finish_printer_observer_at(&second, false, 5, 20, 112)
                .await
                .unwrap()
        );
        assert!(
            repo.claim_printer_observer_at("test", &binding, Uuid::new_v4(), 131)
                .await
                .unwrap()
                .is_none()
        );
        let third = repo
            .claim_printer_observer_at("test", &binding, Uuid::new_v4(), 132)
            .await
            .unwrap()
            .unwrap();
        assert!(
            repo.finish_printer_observer_at(&third, true, 5, 20, 133)
                .await
                .unwrap()
        );
        assert!(
            repo.claim_printer_observer_at("test", &binding, Uuid::new_v4(), 137)
                .await
                .unwrap()
                .is_none()
        );
        let abandoned = repo
            .claim_printer_observer_at("test", &binding, Uuid::new_v4(), 138)
            .await
            .unwrap()
            .unwrap();
        assert!(
            repo.claim_printer_observer_at("test", &binding, Uuid::new_v4(), 197)
                .await
                .unwrap()
                .is_none()
        );
        let recovered = repo
            .claim_printer_observer_at("test", &binding, Uuid::new_v4(), 198)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !repo
                .finish_printer_observer_at(&abandoned, true, 5, 20, 199)
                .await
                .unwrap()
        );
        assert!(
            repo.finish_printer_observer_at(&recovered, true, 5, 20, 199)
                .await
                .unwrap()
        );
        assert!(
            repo.claim_printer_observer_at("test", &"b".repeat(64), Uuid::new_v4(), 100)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            repo.claim_printer_observer_at("../test", &binding, Uuid::new_v4(), 200)
                .await
                .is_err()
        );
        assert!(
            repo.finish_printer_observer_at(&lease, true, 0, 20, 200)
                .await
                .is_err()
        );
    }
}
