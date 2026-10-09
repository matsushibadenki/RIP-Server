use super::*;
impl Repository {
    pub async fn save_approval(
        &self,
        id: Uuid,
        payload: &serde_json::Value,
        expires_at: i64,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO execution_approvals(id,payload,expires_at,revoked) VALUES(?,?,?,0)",
        )
        .bind(id.to_string())
        .bind(payload.to_string())
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    pub async fn approval(&self, id: Uuid) -> Result<serde_json::Value, StorageError> {
        let payload: Option<String> = sqlx::query_scalar(
            "SELECT payload FROM execution_approvals WHERE id=? AND expires_at>? AND revoked=0",
        )
        .bind(id.to_string())
        .bind(now())
        .fetch_optional(&self.pool)
        .await?;
        Ok(serde_json::from_str(
            &payload.ok_or(StorageError::NotFound)?,
        )?)
    }
    pub async fn revoke_approval(&self, id: Uuid) -> Result<(), StorageError> {
        let result = sqlx::query("UPDATE execution_approvals SET revoked=1 WHERE id=?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }
}
