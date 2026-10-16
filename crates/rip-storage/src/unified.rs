//! A durable arrival-order index over immutable stage histories.
use super::*;
use serde_json::{Value, json};
pub(crate) async fn install(pool: &SqlitePool) -> Result<(), StorageError> {
    let mut tx = pool.begin().await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS print_event_log(sequence INTEGER PRIMARY KEY AUTOINCREMENT,kind TEXT NOT NULL,subject TEXT NOT NULL,origin_sequence INTEGER NOT NULL,historical INTEGER NOT NULL DEFAULT 0,UNIQUE(kind,origin_sequence))").execute(&mut *tx).await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS print_event_subject ON print_event_log(kind,subject,sequence)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS print_event_migrations(version INTEGER PRIMARY KEY)")
        .execute(&mut *tx)
        .await?;
    let first = sqlx::query("INSERT OR IGNORE INTO print_event_migrations(version) VALUES(1)")
        .execute(&mut *tx)
        .await?
        .rows_affected()
        == 1;
    for (kind, table, subject) in [
        ("rip", "job_events", "job_id"),
        ("color", "color_events", "attempt_id"),
        ("printer", "delivery_events", "delivery_id"),
        ("pipeline", "pipeline_events", "pipeline_id"),
    ] {
        // All identifiers are fixed program constants, never request input.
        sqlx::query(&format!("CREATE TRIGGER IF NOT EXISTS print_event_{kind} AFTER INSERT ON {table} BEGIN INSERT INTO print_event_log(kind,subject,origin_sequence) VALUES('{kind}',NEW.{subject},NEW.sequence); END")).execute(&mut *tx).await?;
        if first {
            sqlx::query(&format!("INSERT OR IGNORE INTO print_event_log(kind,subject,origin_sequence,historical) SELECT '{kind}',{subject},sequence,1 FROM {table} ORDER BY sequence")).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}
impl Repository {
    pub async fn unified_pipeline_events(
        &self,
        id: Uuid,
        after: i64,
        limit: i64,
    ) -> Result<Value, StorageError> {
        if after < 0 || !(1..=100).contains(&limit) {
            return Err(StorageError::Conflict);
        }
        let pipeline = self.pipeline(id).await?;
        let source = pipeline["sourceJob"]
            .as_str()
            .ok_or(StorageError::Conflict)?;
        let rows=sqlx::query("SELECT l.sequence,l.kind,l.subject,l.origin_sequence,l.historical,CASE l.kind WHEN 'rip' THEN json_object('state',json_extract(r.state,'$'),'createdAt',r.created_at,'metadata',json(d.payload)) WHEN 'printer' THEN json_object('state',p.state,'createdAt',p.created_at,'metadata',json(pd.payload)) WHEN 'color' THEN c.payload ELSE w.payload END AS payload FROM print_event_log l LEFT JOIN job_events r ON l.kind='rip' AND r.sequence=l.origin_sequence LEFT JOIN job_event_details d ON d.sequence=r.sequence LEFT JOIN delivery_events p ON l.kind='printer' AND p.sequence=l.origin_sequence LEFT JOIN delivery_event_details pd ON pd.sequence=p.sequence LEFT JOIN color_events c ON l.kind='color' AND c.sequence=l.origin_sequence LEFT JOIN pipeline_events w ON l.kind='pipeline' AND w.sequence=l.origin_sequence WHERE l.sequence>? AND ((l.kind='rip' AND l.subject=?) OR (l.kind IN ('color','printer','pipeline') AND l.subject=? AND (l.kind='pipeline' OR (l.kind='color' AND EXISTS(SELECT 1 FROM color_attempts ca WHERE ca.id=l.subject AND ca.source_job=?)) OR (l.kind='printer' AND EXISTS(SELECT 1 FROM deliveries da WHERE da.id=l.subject AND json_extract(da.payload,'$.ticket.sourceJob')=?))))) ORDER BY l.sequence LIMIT ?")
            .bind(after).bind(source).bind(id.to_string()).bind(source).bind(source).bind(limit+1).fetch_all(&self.pool).await?;
        let has_more = rows.len() > limit as usize;
        let mut cursor = after;
        let mut events = vec![];
        for row in rows.into_iter().take(limit as usize) {
            cursor = row.get("sequence");
            let kind: String = row.get("kind");
            let raw: Value = serde_json::from_str(row.get("payload"))?;
            let details = if matches!(kind.as_str(), "rip" | "printer") {
                &raw["metadata"]
            } else {
                &raw
            };
            let state = raw["state"].clone();
            let completion = if state == "COMPLETED" {
                match kind.as_str() {
                    "rip" => Some("raster_export"),
                    "color" => Some("color_managed_export"),
                    _ => Some("device-confirmed"),
                }
            } else {
                None
            };
            let stage = match kind.as_str() {
                "rip" => "rip",
                "color" => "color_management",
                "printer" => "printer",
                _ => raw["stage"].as_str().unwrap_or("orchestration"),
            };
            events.push(json!({"schemaVersion":1,"sequence":cursor,"pipelineId":id,"sourceJob":source,"stage":stage,"state":state,"createdAt":raw["createdAt"],"reasonCode":details.get("reasonCode").or_else(||details.get("reason")).cloned().unwrap_or(Value::Null),"revision":details["revision"],"progressPercent":details["progressPercent"],"progressPhase":details["progressPhase"],"completedPages":details["completedPages"],"totalPages":details["totalPages"],"retryable":details["retryable"].as_bool().unwrap_or(false),"automaticRetry":false,"completionMeaning":completion,"historicalImport":row.get::<i64,_>("historical")==1,"origin":{"kind":kind,"subjectId":row.get::<String,_>("subject"),"sequence":row.get::<i64,_>("origin_sequence")},"details":details}));
        }
        Ok(
            json!({"schemaVersion":1,"pipelineId":id,"events":events,"nextAfter":cursor,"hasMore":has_more,"ordering":"durable-arrival-order"}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn legacy_history_import_is_once_only_and_future_events_are_live() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("jobs.sqlite3");
        let repo = Repository::open(&path).await.unwrap();
        for kind in ["rip", "color", "printer", "pipeline"] {
            sqlx::query(&format!("DROP TRIGGER print_event_{kind}"))
                .execute(&repo.pool)
                .await
                .unwrap();
        }
        sqlx::query("DELETE FROM print_event_migrations")
            .execute(&repo.pool)
            .await
            .unwrap();
        let job:Job=serde_json::from_value(json!({"id":Uuid::new_v4(),"manifest":{"name":"old.pdf","format":"application/pdf"},"state":"COMPLETED","source_sha256":"a".repeat(64),"created_at":now(),"updated_at":now(),"revision":0,"error_code":null,"selected_engine":null,"worker_run_id":null})).unwrap();
        repo.insert(&job).await.unwrap();
        let id = Uuid::new_v4();
        repo.create_pipeline(id, job.id, 0, json!({}))
            .await
            .unwrap();
        let reopened = Repository::open(&path).await.unwrap();
        let history = reopened.unified_pipeline_events(id, 0, 100).await.unwrap();
        assert_eq!(history["events"].as_array().unwrap().len(), 2);
        assert!(
            history["events"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["historicalImport"] == true)
        );
        reopened
            .update_pipeline(id, 0, "COMPLETED", None, None)
            .await
            .unwrap();
        let live = reopened
            .unified_pipeline_events(id, history["nextAfter"].as_i64().unwrap(), 100)
            .await
            .unwrap();
        assert_eq!(live["events"].as_array().unwrap().len(), 1);
        assert_eq!(live["events"][0]["historicalImport"], false);
        let second = Repository::open(&path).await.unwrap();
        assert_eq!(
            second.unified_pipeline_events(id, 0, 100).await.unwrap()["events"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
}
