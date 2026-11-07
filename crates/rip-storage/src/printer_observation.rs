//! Registration-scoped observations, not immutable print receipts or device control.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub const PROFILE: &str = "org.jrip.printer-observations-v1";
const WINDOW: i64 = 1000;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QueryFailure {
    Unavailable,
    InvalidResponse,
    ConfigurationUnavailable,
}
fn valid(id: &str, binding: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        && rip_core::print::valid_digest(binding)
}
fn snapshot_digest(value: &Value) -> Result<String, StorageError> {
    let keys = [
        "schemaVersion",
        "profile",
        "state",
        "acceptingJobs",
        "queuedJobCount",
        "reasons",
        "conditions",
        "markerSupplies",
        "printerSupplies",
    ];
    if !value
        .as_object()
        .is_some_and(|o| o.len() == keys.len() && o.keys().all(|k| keys.contains(&k.as_str())))
        || value["schemaVersion"] != 1
        || value["profile"] != "org.jrip.printer-state-snapshot-v1"
        || !value["state"]
            .as_i64()
            .is_some_and(|n| (3..=5).contains(&n))
        || !value["acceptingJobs"].is_boolean()
        || !(value["queuedJobCount"].is_null()
            || value["queuedJobCount"]
                .as_u64()
                .is_some_and(|n| n <= i32::MAX as u64))
        || !value["reasons"].as_array().is_some_and(|a| {
            a.len() <= 64
                && a.iter().all(|v| {
                    v.as_str().is_some_and(|s| {
                        !s.is_empty() && s.len() <= 255 && !s.chars().any(char::is_control)
                    })
                })
        })
        || value["conditions"]["profile"] != rip_core::printer_conditions::PROFILE
        || value["markerSupplies"]["profile"] != "org.jrip.cups-marker-supplies-v1"
        || value["printerSupplies"]["profile"] != "org.jrip.pwg-printer-supplies-v1"
        || serde_json::to_vec(value)?.len() > 256 * 1024
    {
        return Err(StorageError::Conflict);
    }
    rip_digest::digest(value).map_err(|_| StorageError::Conflict)
}
impl Repository {
    /// Order requests when they begin; a late completion cannot overwrite a newer completed query.
    pub async fn begin_printer_query(&self, id: &str, binding: &str) -> Result<i64, StorageError> {
        if !valid(id, binding) {
            return Err(StorageError::Conflict);
        }
        Ok(sqlx::query_scalar("INSERT INTO printer_observations(printer_id,binding,stream_id,issued,applied,body,pruned_through) VALUES(?,?,?,1,0,'{}',0) ON CONFLICT(printer_id,binding) DO UPDATE SET issued=issued+1 RETURNING issued")
            .bind(id).bind(binding).bind(Uuid::new_v4().to_string()).fetch_one(&self.pool).await?)
    }
    pub async fn finish_printer_query(
        &self,
        id: &str,
        binding: &str,
        serial: i64,
        result: Result<Value, QueryFailure>,
    ) -> Result<bool, StorageError> {
        self.finish_printer_query_at(id, binding, serial, result, now())
            .await
    }
    async fn finish_printer_query_at(
        &self,
        id: &str,
        binding: &str,
        serial: i64,
        result: Result<Value, QueryFailure>,
        at: i64,
    ) -> Result<bool, StorageError> {
        if !valid(id, binding) || serial < 1 || !(0..=i64::MAX - 30).contains(&at) {
            return Err(StorageError::Conflict);
        }
        let digest = result.as_ref().ok().map(snapshot_digest).transpose()?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE printer_observations SET applied=applied WHERE printer_id=? AND binding=?",
        )
        .bind(id)
        .bind(binding)
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query(
            "SELECT issued,applied,body,stream_id FROM printer_observations WHERE printer_id=? AND binding=?",
        )
        .bind(id)
        .bind(binding)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StorageError::NotFound)?;
        if serial > row.get::<i64, _>("issued") {
            return Err(StorageError::Conflict);
        }
        if serial <= row.get::<i64, _>("applied") {
            tx.commit().await?;
            return Ok(false);
        }
        let previous: Value = serde_json::from_str(row.get("body"))?;
        let failure = serde_json::to_value(result.as_ref().err())?;
        let new_digest = digest
            .as_ref()
            .map(|s| json!(s))
            .unwrap_or_else(|| previous["snapshotSha256"].clone());
        let changed =
            previous["queryFailure"] != failure || previous["snapshotSha256"] != new_digest;
        let first = previous["revision"].is_null();
        let changed = changed || first;
        let revision = previous["revision"].as_i64().unwrap_or(0) + i64::from(changed);
        let observed = if result.is_ok() {
            json!(at)
        } else {
            previous["lastObservedAt"].clone()
        };
        let body = json!({"schemaVersion":1,"profile":PROFILE,"printerId":id,"streamId":row.get::<String,_>("stream_id"),"revision":revision,"querySerial":serial,
            "changedAt":if changed{json!(at)}else{previous["changedAt"].clone()},"lastQueryAt":at,
            "queryChangedAt":if first || previous["queryFailure"]!=failure{json!(at)}else{previous["queryChangedAt"].clone()},
            "queryFailure":failure,"snapshotSha256":new_digest,"digestProfile":rip_digest::PROFILE,"lastObservedAt":observed,
            "expiresAt":observed.as_i64().map(|n|n+30),"isLive":false,"authorizationGranted":false});
        if let Ok(snapshot) = result {
            sqlx::query("INSERT OR IGNORE INTO printer_observation_contents(printer_id,binding,digest,payload) VALUES(?,?,?,?)")
                .bind(id).bind(binding).bind(digest).bind(serde_json::to_string(&snapshot)?).execute(&mut *tx).await?;
        }
        sqlx::query(
            "UPDATE printer_observations SET applied=?,body=? WHERE printer_id=? AND binding=?",
        )
        .bind(serial)
        .bind(serde_json::to_string(&body)?)
        .bind(id)
        .bind(binding)
        .execute(&mut *tx)
        .await?;
        if changed {
            let mut event = body.clone();
            event["kind"] = json!(if !failure.is_null() {
                "QUERY_FAILED"
            } else if !previous["queryFailure"].is_null() {
                "QUERY_RECOVERED"
            } else if first {
                "FIRST_REPORT"
            } else {
                "DEVICE_REPORT_CHANGED"
            });
            sqlx::query(
                "INSERT INTO printer_observation_events(printer_id,binding,payload) VALUES(?,?,?)",
            )
            .bind(id)
            .bind(binding)
            .bind(serde_json::to_string(&event)?)
            .execute(&mut *tx)
            .await?;
            let cutoff:Option<i64>=sqlx::query_scalar("SELECT sequence FROM printer_observation_events WHERE printer_id=? AND binding=? ORDER BY sequence DESC LIMIT 1 OFFSET ?").bind(id).bind(binding).bind(WINDOW-1).fetch_optional(&mut *tx).await?;
            if let Some(cutoff) = cutoff {
                let pruned:Option<i64>=sqlx::query_scalar("SELECT MAX(sequence) FROM printer_observation_events WHERE printer_id=? AND binding=? AND sequence<?").bind(id).bind(binding).bind(cutoff).fetch_one(&mut *tx).await?;
                if let Some(pruned) = pruned {
                    sqlx::query("UPDATE printer_observations SET pruned_through=MAX(pruned_through,?) WHERE printer_id=? AND binding=?").bind(pruned).bind(id).bind(binding).execute(&mut *tx).await?;
                    sqlx::query("DELETE FROM printer_observation_events WHERE printer_id=? AND binding=? AND sequence<?").bind(id).bind(binding).bind(cutoff).execute(&mut *tx).await?;
                    sqlx::query("DELETE FROM printer_observation_contents WHERE printer_id=? AND binding=? AND digest NOT IN (SELECT json_extract(payload,'$.snapshotSha256') FROM printer_observation_events WHERE printer_id=? AND binding=? AND json_type(payload,'$.snapshotSha256')='text') AND digest!=COALESCE(?, '')")
                        .bind(id).bind(binding).bind(id).bind(binding).bind(new_digest.as_str()).execute(&mut *tx).await?;
                }
            }
        }
        tx.commit().await?;
        Ok(true)
    }
    async fn printer_snapshot(
        connection: &mut sqlx::SqliteConnection,
        id: &str,
        binding: &str,
        digest: &Value,
    ) -> Result<Value, StorageError> {
        let Some(digest) = digest.as_str() else {
            return Ok(Value::Null);
        };
        let raw:String=sqlx::query_scalar("SELECT payload FROM printer_observation_contents WHERE printer_id=? AND binding=? AND digest=?").bind(id).bind(binding).bind(digest).fetch_optional(&mut *connection).await?.ok_or(StorageError::NotFound)?;
        let value: Value = serde_json::from_str(&raw)?;
        if snapshot_digest(&value)? != digest {
            return Err(StorageError::Conflict);
        }
        Ok(value)
    }
    /// Compact event summaries; exact retained snapshots are retrieved one event at a time.
    pub async fn printer_observations(
        &self,
        id: &str,
        binding: &str,
        after: i64,
        limit: u32,
    ) -> Result<Value, StorageError> {
        if !valid(id, binding) || after < 0 || !(1..=100).contains(&limit) {
            return Err(StorageError::Conflict);
        }
        let mut tx = self.pool.begin().await?;
        let head = sqlx::query(
            "SELECT body,pruned_through,stream_id FROM printer_observations WHERE printer_id=? AND binding=?",
        )
        .bind(id)
        .bind(binding)
        .fetch_optional(&mut *tx)
        .await?;
        let (mut latest, pruned, stream_id) = match head {
            Some(row) => (
                serde_json::from_str::<Value>(row.get("body"))?,
                row.get::<i64, _>("pruned_through"),
                Some(row.get::<String, _>("stream_id")),
            ),
            None => (Value::Null, 0, None),
        };
        if latest["revision"].is_null() {
            latest = Value::Null;
        } else {
            latest["snapshot"] =
                Self::printer_snapshot(&mut tx, id, binding, &latest["snapshotSha256"]).await?;
        }
        let rows=sqlx::query("SELECT sequence,payload FROM printer_observation_events WHERE printer_id=? AND binding=? AND sequence>? ORDER BY sequence LIMIT ?").bind(id).bind(binding).bind(after).bind(i64::from(limit)+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > limit as usize;
        let mut events = Vec::new();
        for row in rows.into_iter().take(limit as usize) {
            let mut event: Value = serde_json::from_str(row.get("payload"))?;
            event["sequence"] = json!(row.get::<i64, _>("sequence"));
            events.push(event);
        }
        let next = events
            .last()
            .and_then(|e| e["sequence"].as_i64())
            .unwrap_or(after);
        tx.commit().await?;
        Ok(
            json!({"schemaVersion":1,"profile":PROFILE,"printerId":id,"streamId":stream_id,"latest":latest,"events":events,"nextAfter":next,"hasMore":more,
            "prunedThrough":pruned,"historyTruncated":after<pruned,"retainedEventLimit":WINDOW,"isLive":false,"authorizationGranted":false}),
        )
    }
    pub async fn printer_observation_event(
        &self,
        id: &str,
        binding: &str,
        sequence: i64,
    ) -> Result<Value, StorageError> {
        if !valid(id, binding) || sequence < 1 {
            return Err(StorageError::Conflict);
        }
        let mut tx = self.pool.begin().await?;
        let raw:String=sqlx::query_scalar("SELECT payload FROM printer_observation_events WHERE printer_id=? AND binding=? AND sequence=?").bind(id).bind(binding).bind(sequence).fetch_optional(&mut *tx).await?.ok_or(StorageError::NotFound)?;
        let mut event: Value = serde_json::from_str(&raw)?;
        event["sequence"] = json!(sequence);
        event["snapshot"] =
            Self::printer_snapshot(&mut tx, id, binding, &event["snapshotSha256"]).await?;
        tx.commit().await?;
        Ok(event)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(count: u32) -> Value {
        json!({"schemaVersion":1,"profile":"org.jrip.printer-state-snapshot-v1","state":3,"acceptingJobs":true,"queuedJobCount":count,"reasons":[],
            "conditions":rip_core::printer_conditions::describe(3,&[]),"markerSupplies":{"profile":"org.jrip.cups-marker-supplies-v1","status":"UNREPORTED"},"printerSupplies":{"profile":"org.jrip.pwg-printer-supplies-v1","status":"UNREPORTED"}})
    }
    async fn record(
        repo: &Repository,
        binding: &str,
        at: i64,
        result: Result<Value, QueryFailure>,
    ) {
        let serial = repo.begin_printer_query("test", binding).await.unwrap();
        assert!(
            repo.finish_printer_query_at("test", binding, serial, result, at)
                .await
                .unwrap()
        );
    }
    #[tokio::test]
    async fn observations_deduplicate_survive_restart_separate_bindings_and_ignore_late_queries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite");
        let repo = Repository::open(&path).await.unwrap();
        let binding = "a".repeat(64);
        assert!(
            repo.printer_observations("test", &binding, 0, 100)
                .await
                .unwrap()["latest"]
                .is_null()
        );
        record(&repo, &binding, 10, Ok(snapshot(0))).await;
        record(&repo, &binding, 12, Ok(snapshot(0))).await;
        let first = repo
            .printer_observations("test", &binding, 0, 100)
            .await
            .unwrap();
        assert_eq!(first["latest"]["revision"], 1);
        assert_eq!(first["events"].as_array().unwrap().len(), 1);
        assert_eq!(first["latest"]["changedAt"], 10);
        assert_eq!(first["latest"]["lastObservedAt"], 12);
        let stream = first["streamId"].clone();
        Uuid::parse_str(stream.as_str().unwrap()).unwrap();
        record(&repo, &binding, 13, Err(QueryFailure::Unavailable)).await;
        record(&repo, &binding, 14, Err(QueryFailure::Unavailable)).await;
        let failed = repo
            .printer_observations("test", &binding, 0, 100)
            .await
            .unwrap();
        assert_eq!(failed["latest"]["revision"], 2);
        assert_eq!(failed["latest"]["lastQueryAt"], 14);
        assert_eq!(failed["latest"]["lastObservedAt"], 12);
        assert_eq!(failed["latest"]["snapshot"], snapshot(0));
        record(&repo, &binding, 15, Err(QueryFailure::InvalidResponse)).await;
        let older = repo.begin_printer_query("test", &binding).await.unwrap();
        let newer = repo.begin_printer_query("test", &binding).await.unwrap();
        repo.finish_printer_query_at("test", &binding, newer, Ok(snapshot(1)), 20)
            .await
            .unwrap();
        assert!(
            !repo
                .finish_printer_query_at(
                    "test",
                    &binding,
                    older,
                    Err(QueryFailure::Unavailable),
                    21
                )
                .await
                .unwrap()
        );
        let recovered = repo
            .printer_observations("test", &binding, 0, 100)
            .await
            .unwrap();
        assert_eq!(recovered["latest"]["revision"], 4);
        assert_eq!(recovered["latest"]["lastQueryAt"], 20);
        assert!(recovered["latest"]["queryFailure"].is_null());
        assert_eq!(recovered["events"][3]["kind"], "QUERY_RECOVERED");
        let repo = Repository::open(&path).await.unwrap();
        let page = repo
            .printer_observations("test", &binding, 0, 2)
            .await
            .unwrap();
        assert_eq!(page["streamId"], stream);
        assert_eq!(page["events"].as_array().unwrap().len(), 2);
        assert_eq!(page["hasMore"], true);
        assert!(page["events"][0].get("snapshot").is_none());
        let next = repo
            .printer_observations("test", &binding, page["nextAfter"].as_i64().unwrap(), 2)
            .await
            .unwrap();
        assert_eq!(next["events"].as_array().unwrap().len(), 2);
        assert_eq!(next["hasMore"], false);
        let sequence = first["events"][0]["sequence"].as_i64().unwrap();
        let event = repo
            .printer_observation_event("test", &binding, sequence)
            .await
            .unwrap();
        assert_eq!(event["lastObservedAt"], 10);
        assert_eq!(event["snapshot"], snapshot(0));
        assert_eq!(
            event["snapshotSha256"],
            rip_digest::digest(&snapshot(0)).unwrap()
        );
        let other = "b".repeat(64);
        assert!(
            repo.printer_observations("test", &other, 0, 100)
                .await
                .unwrap()["latest"]
                .is_null()
        );
        assert!(matches!(
            repo.printer_observation_event("test", &other, sequence)
                .await,
            Err(StorageError::NotFound)
        ));
        record(
            &repo,
            &other,
            22,
            Err(QueryFailure::ConfigurationUnavailable),
        )
        .await;
        let separate = repo
            .printer_observations("test", &other, 0, 100)
            .await
            .unwrap();
        assert_ne!(separate["streamId"], stream);
        assert!(separate["latest"]["snapshot"].is_null());
        assert!(separate["latest"]["lastObservedAt"].is_null());
        sqlx::query("UPDATE printer_observation_contents SET payload=? WHERE printer_id='test' AND binding=? AND digest=?")
            .bind(serde_json::to_string(&snapshot(99)).unwrap()).bind(&binding).bind(event["snapshotSha256"].as_str()).execute(&repo.pool).await.unwrap();
        assert!(matches!(
            repo.printer_observation_event("test", &binding, sequence)
                .await,
            Err(StorageError::Conflict)
        ));
    }
    #[tokio::test]
    async fn bounded_history_announces_pruning_and_garbage_collects_unreferenced_content() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::open(&dir.path().join("state.sqlite"))
            .await
            .unwrap();
        let binding = "a".repeat(64);
        for n in 0..1002 {
            record(&repo, &binding, n, Ok(snapshot(n as u32))).await;
        }
        let history = repo
            .printer_observations("test", &binding, 0, 100)
            .await
            .unwrap();
        assert_eq!(history["historyTruncated"], true);
        assert_eq!(history["prunedThrough"], 2);
        assert_eq!(history["events"][0]["sequence"], 3);
        assert_eq!(history["latest"]["snapshot"]["queuedJobCount"], 1001);
        assert!(matches!(
            repo.printer_observation_event("test", &binding, 1).await,
            Err(StorageError::NotFound)
        ));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM printer_observation_contents")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(count, 1000);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM printer_observation_events")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(count, 1000);
        assert!(
            repo.printer_observations("test", &binding, -1, 100)
                .await
                .is_err()
        );
        assert!(
            repo.printer_observations("test", &binding, 0, 101)
                .await
                .is_err()
        );
        assert!(repo.begin_printer_query("../test", &binding).await.is_err());
        let serial = repo.begin_printer_query("test", &binding).await.unwrap();
        let mut invalid = snapshot(1);
        invalid["uri"] = json!("ipp://private/device");
        assert!(
            repo.finish_printer_query_at("test", &binding, serial, Ok(invalid), 2000)
                .await
                .is_err()
        );
    }
}
