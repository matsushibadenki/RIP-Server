use super::*;

impl Repository {
    pub async fn begin_audit(
        &self,
        correlation_id: Uuid,
        identity: &str,
        operation: &str,
        target: Option<Uuid>,
    ) -> Result<i64, StorageError> {
        let result = sqlx::query("INSERT INTO audit_records(correlation_id,identity,operation,target,started_at) VALUES(?,?,?,?,?)")
            .bind(correlation_id.to_string()).bind(identity).bind(operation)
            .bind(target.map(|id| id.to_string())).bind(now()).execute(&self.pool).await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn finish_audit(&self, sequence: i64, status: u16) -> Result<(), StorageError> {
        sqlx::query("UPDATE audit_records SET status=?,finished_at=? WHERE sequence=? AND finished_at IS NULL")
            .bind(i64::from(status)).bind(now()).bind(sequence).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn audit_records(
        &self,
        after: i64,
        limit: i64,
    ) -> Result<serde_json::Value, StorageError> {
        if after < 0 || !(1..=100).contains(&limit) {
            return Err(StorageError::Conflict);
        }
        let rows =
            sqlx::query("SELECT * FROM audit_records WHERE sequence>? ORDER BY sequence LIMIT ?")
                .bind(after)
                .bind(limit + 1)
                .fetch_all(&self.pool)
                .await?;
        let has_more = rows.len() > limit as usize;
        let mut cursor = after;
        let records: Vec<_> = rows
            .into_iter()
            .take(limit as usize)
            .map(|row| {
                cursor = row.get("sequence");
                serde_json::json!({"sequence":cursor,
                "correlationId":row.get::<String,_>("correlation_id"),
                "identity":row.get::<String,_>("identity"),
                "operation":row.get::<String,_>("operation"),
                "target":row.get::<Option<String>,_>("target"),
                "startedAt":row.get::<i64,_>("started_at"),
                "finishedAt":row.get::<Option<i64>,_>("finished_at"),
                "status":row.get::<Option<i64>,_>("status")})
            })
            .collect();
        Ok(serde_json::json!({"records":records,"nextCursor":cursor,"hasMore":has_more}))
    }
}
