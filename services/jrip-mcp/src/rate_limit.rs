//! Process-local identity buckets. Only authenticated identities allocate state.
use std::{collections::BTreeMap, time::Duration};
use tokio::{sync::Mutex, time::Instant};
const MINUTE: u128 = 60_000_000_000;
#[derive(Debug, Clone, Copy)]
pub struct RequestLimits {
    per_minute: u32,
    burst: u32,
}
impl Default for RequestLimits {
    fn default() -> Self {
        Self {
            per_minute: 120,
            burst: 30,
        }
    }
}
impl RequestLimits {
    pub fn requests_per_minute(self) -> u32 {
        self.per_minute
    }
    pub fn burst(self) -> u32 {
        self.burst
    }
    pub fn new(per_minute: u32, burst: u32) -> Result<Self, &'static str> {
        if !(1..=60000).contains(&per_minute) || !(1..=10000).contains(&burst) {
            return Err("MCP requests per minute must be 1..60000 and burst must be 1..10000");
        }
        Ok(Self { per_minute, burst })
    }
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        fn value(name: &str, default: u32) -> Result<u32, Box<dyn std::error::Error>> {
            match std::env::var(name) {
                Ok(value) => Ok(value.parse()?),
                Err(std::env::VarError::NotPresent) => Ok(default),
                Err(error) => Err(error.into()),
            }
        }
        let defaults = Self::default();
        Ok(Self::new(
            value("JRIP_MCP_REQUESTS_PER_MINUTE", defaults.per_minute)?,
            value("JRIP_MCP_REQUEST_BURST", defaults.burst)?,
        )?)
    }
}
struct Bucket {
    updated: Instant,
    credit: u128,
}
pub(crate) struct RateLimiter {
    limits: RequestLimits,
    buckets: Mutex<BTreeMap<String, Bucket>>,
}
impl RateLimiter {
    pub(crate) fn limits(&self) -> RequestLimits {
        self.limits
    }
    pub(crate) fn new(limits: RequestLimits) -> Self {
        Self {
            limits,
            buckets: Mutex::new(BTreeMap::new()),
        }
    }
    pub(crate) async fn admit(&self, identity: &str) -> Result<(), u64> {
        let mut buckets = self.buckets.lock().await;
        let now = Instant::now();
        let capacity = u128::from(self.limits.burst) * MINUTE;
        let bucket = buckets.entry(identity.to_owned()).or_insert(Bucket {
            updated: now,
            credit: capacity,
        });
        let refill = now
            .duration_since(bucket.updated)
            .as_nanos()
            .saturating_mul(u128::from(self.limits.per_minute));
        bucket.credit = bucket.credit.saturating_add(refill).min(capacity);
        bucket.updated = now;
        if bucket.credit >= MINUTE {
            bucket.credit -= MINUTE;
            Ok(())
        } else {
            let nanos = (MINUTE - bucket.credit).div_ceil(u128::from(self.limits.per_minute));
            Err(nanos.div_ceil(Duration::from_secs(1).as_nanos()) as u64)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_is_finite_and_cannot_disable_limits() {
        for (rate, burst) in [(0, 1), (1, 0), (60001, 1), (1, 10001), (u32::MAX, u32::MAX)] {
            assert!(RequestLimits::new(rate, burst).is_err());
        }
        assert!(RequestLimits::new(1, 1).is_ok());
        assert!(RequestLimits::new(60000, 10000).is_ok());
    }
    #[tokio::test(start_paused = true)]
    async fn fractional_refill_denials_and_idle_capacity_are_exact() {
        let limiter = RateLimiter::new(RequestLimits::new(120, 1).unwrap());
        assert_eq!(limiter.admit("agent").await, Ok(()));
        assert_eq!(limiter.admit("agent").await, Err(1));
        tokio::time::advance(Duration::from_millis(250)).await;
        assert_eq!(limiter.admit("agent").await, Err(1));
        tokio::time::advance(Duration::from_millis(250)).await;
        assert_eq!(limiter.admit("agent").await, Ok(()));
        tokio::time::advance(Duration::from_secs(600)).await;
        assert_eq!(limiter.admit("agent").await, Ok(()));
        assert_eq!(limiter.admit("agent").await, Err(1));
        assert_eq!(limiter.admit("other").await, Ok(()));
    }
    #[tokio::test(start_paused = true)]
    async fn retry_after_ceil_and_single_winner_under_concurrency() {
        let slow = RateLimiter::new(RequestLimits::new(1, 1).unwrap());
        slow.admit("agent").await.unwrap();
        assert_eq!(slow.admit("agent").await, Err(60));
        tokio::time::advance(Duration::from_secs(59)).await;
        assert_eq!(slow.admit("agent").await, Err(1));
        tokio::time::advance(Duration::from_secs(1)).await;
        slow.admit("agent").await.unwrap();
        let limiter = std::sync::Arc::new(RateLimiter::new(RequestLimits::new(1, 3).unwrap()));
        let mut tasks = vec![];
        for _ in 0..50 {
            let limiter = limiter.clone();
            tasks.push(tokio::spawn(
                async move { limiter.admit("agent").await.is_ok() },
            ));
        }
        let mut admitted = 0;
        for task in tasks {
            admitted += u32::from(task.await.unwrap());
        }
        assert_eq!(admitted, 3);
    }
}
