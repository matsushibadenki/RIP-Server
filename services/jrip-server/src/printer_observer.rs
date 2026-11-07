//! Optional periodic observer. It never calls Print-Job, Cancel-Job or delivery retry.
use crate::{App, delivery::Printer, printer_observation};
use futures_util::{
    StreamExt,
    stream::{self, FuturesUnordered},
};
use serde::Serialize;
use std::{collections::HashSet, future::Future, time::Duration};
use uuid::Uuid;
pub enum ConfigError {
    Interval,
    Concurrency,
    Backoff,
    Environment(&'static str),
}
impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Interval => {
                "Observer interval must be 5–3600 seconds / 観測間隔は5〜3600秒 / 观察间隔须为5至3600秒"
            }
            Self::Concurrency => {
                "Observer concurrency must be 1–16 / 観測の並列数は1〜16 / 观察并发数须为1至16"
            }
            Self::Backoff => {
                "Maximum backoff must be at least the interval and at most 86400 seconds / 最大待機は観測間隔以上・86400秒以下 / 最大退避须不小于间隔且不超过86400秒"
            }
            Self::Environment(name) => {
                return write!(
                    f,
                    "Invalid observer environment setting {name} / 観測の環境設定が不正です / 观察环境设置无效"
                );
            }
        };
        f.write_str(message)
    }
}
impl std::fmt::Debug for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for ConfigError {}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    interval_seconds: u32,
    concurrency: usize,
    max_backoff_seconds: u32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            interval_seconds: 15,
            concurrency: 4,
            max_backoff_seconds: 300,
        }
    }
}
impl Config {
    pub fn new(
        interval_seconds: u32,
        concurrency: usize,
        max_backoff_seconds: u32,
    ) -> Result<Self, ConfigError> {
        if !(5..=3600).contains(&interval_seconds) {
            return Err(ConfigError::Interval);
        }
        if !(1..=16).contains(&concurrency) {
            return Err(ConfigError::Concurrency);
        }
        if !(interval_seconds..=86400).contains(&max_backoff_seconds) {
            return Err(ConfigError::Backoff);
        }
        Ok(Self {
            interval_seconds,
            concurrency,
            max_backoff_seconds,
        })
    }
    pub fn from_env() -> Result<Self, ConfigError> {
        fn setting(name: &'static str, default: u32) -> Result<u32, ConfigError> {
            match std::env::var(name) {
                Ok(value) => value.parse().map_err(|_| ConfigError::Environment(name)),
                Err(std::env::VarError::NotPresent) => Ok(default),
                Err(_) => Err(ConfigError::Environment(name)),
            }
        }
        let interval = setting("JRIP_PRINTER_OBSERVER_INTERVAL_SECONDS", 15)?;
        Self::new(
            interval,
            setting("JRIP_PRINTER_OBSERVER_CONCURRENCY", 4)? as usize,
            setting(
                "JRIP_PRINTER_OBSERVER_MAX_BACKOFF_SECONDS",
                interval.max(300),
            )?,
        )
    }
}
#[derive(Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub attempted: usize,
    pub successful: usize,
    pub failed: usize,
    pub skipped: usize,
    pub storage_errors: usize,
    pub lease_lost: usize,
}
impl Report {
    fn add(&mut self, other: Self) {
        self.attempted += other.attempted;
        self.successful += other.successful;
        self.failed += other.failed;
        self.skipped += other.skipped;
        self.storage_errors += other.storage_errors;
        self.lease_lost += other.lease_lost;
    }
}
async fn observe(app: &App, printer: &Printer, config: Config) -> Report {
    let mut report = Report::default();
    let binding = match printer_observation::binding(app, printer) {
        Ok(binding) => binding,
        Err(_) => {
            report.storage_errors = 1;
            return report;
        }
    };
    let lease = match app
        .repo
        .claim_printer_observer(&printer.id, &binding, Uuid::new_v4())
        .await
    {
        Ok(Some(lease)) => lease,
        Ok(None) => {
            report.skipped = 1;
            return report;
        }
        Err(_) => {
            report.storage_errors = 1;
            return report;
        }
    };
    report.attempted = 1;
    let success = match printer_observation::probe(app, printer).await {
        Ok(_) => {
            report.successful = 1;
            true
        }
        Err(error) if error.0.is_server_error() => {
            report.storage_errors = 1;
            return report;
        }
        Err(_) => {
            report.failed = 1;
            false
        }
    };
    match app
        .repo
        .finish_printer_observer(
            &lease,
            success,
            config.interval_seconds,
            config.max_backoff_seconds,
        )
        .await
    {
        Ok(true) => {}
        Ok(false) => report.lease_lost = 1,
        Err(_) => report.storage_errors += 1,
    }
    report
}
fn log(printer: &Printer, report: &Report) {
    if report.failed > 0 || report.storage_errors > 0 || report.lease_lost > 0 {
        tracing::warn!(printer_id=%printer.id,failed=report.failed,storage_errors=report.storage_errors,lease_lost=report.lease_lost,"printer observation incomplete");
    } else if report.successful > 0 {
        tracing::info!(printer_id=%printer.id,"printer observation recorded");
    }
}
/// One due pass; skipped registrations retain their persisted cooldown/lease.
pub async fn observe_once(app: &App, config: Config) -> Report {
    let mut pending = stream::iter(app.printers.iter().map(|printer| async move {
        let report = observe(app, printer, config).await;
        log(printer, &report);
        report
    }))
    .buffer_unordered(config.concurrency);
    let mut report = Report::default();
    while let Some(result) = pending.next().await {
        report.add(result);
    }
    report
}
/// Bounded, round-robin continuous scheduling. Slow peers do not create a pass barrier.
pub async fn run_until(app: &App, config: Config, shutdown: impl Future<Output = ()>) -> Report {
    let mut pending = FuturesUnordered::new();
    let mut in_flight = HashSet::new();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut cursor = 0;
    let mut scanned = 0;
    let mut total = Report::default();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _=&mut shutdown=>return total,
            _=interval.tick()=>scanned=0,
            Some((index,report))=pending.next(),if !pending.is_empty()=>{in_flight.remove(&index);log(&app.printers[index],&report);total.add(report);},
        }
        while pending.len() < config.concurrency && scanned < app.printers.len() {
            let index = cursor;
            cursor = (cursor + 1) % app.printers.len();
            scanned += 1;
            if !in_flight.insert(index) {
                continue;
            }
            pending.push(async move { (index, observe(app, &app.printers[index], config).await) });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observer_configuration_bounds_protect_cadence_and_fanout() {
        assert!(Config::new(5, 1, 5).is_ok());
        assert!(Config::new(3600, 16, 86400).is_ok());
        assert!(Config::new(4, 1, 10).is_err());
        assert!(Config::new(3601, 1, 7200).is_err());
        assert!(Config::new(5, 0, 10).is_err());
        assert!(Config::new(5, 17, 10).is_err());
        assert!(Config::new(15, 1, 14).is_err());
        assert!(Config::new(15, 1, 86401).is_err());
    }
}
