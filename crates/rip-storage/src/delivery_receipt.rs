//! Immutable terminal delivery evidence, published atomically with the observation.
use crate::{
    Repository, StorageError,
    delivery::{Delivery, DeliveryState},
    receipts::{self, ReceiptSigner},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;
impl Repository {
    pub async fn delivery_result_receipt(&self, id: Uuid) -> Result<Value, StorageError> {
        let payload: String =
            sqlx::query_scalar("SELECT payload FROM delivery_result_receipts WHERE delivery_id=?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?
                .ok_or(StorageError::NotFound)?;
        Ok(serde_json::from_str(&payload)?)
    }
}
pub(crate) async fn store(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    delivery: &Delivery,
    signer: Option<&ReceiptSigner>,
) -> Result<(), StorageError> {
    let status = match delivery.state {
        DeliveryState::Completed => "succeeded",
        DeliveryState::Cancelled => "cancelled",
        DeliveryState::Aborted | DeliveryState::Rejected => "failed",
        _ => return Ok(()), // ACCEPTED and UNKNOWN do not establish a terminal result.
    };
    if delivery.state == DeliveryState::Completed && delivery.device_job_id.is_none_or(|id| id <= 0)
    {
        return Err(StorageError::Conflict);
    }
    let contract: Option<String> =
        sqlx::query_scalar("SELECT payload FROM delivery_contracts WHERE delivery_id=?")
            .bind(delivery.id.to_string())
            .fetch_optional(&mut **tx)
            .await?;
    let contract: Option<Value> = contract.map(|s| serde_json::from_str(&s)).transpose()?;
    let request =
        json!({"deliveryId":delivery.id,"ticket":delivery.ticket,"physicalContract":contract});
    let evidence = sqlx::query("SELECT payload FROM print_result_receipts WHERE job_id=?")
        .bind(delivery.ticket.source_job.to_string())
        .fetch_optional(&mut **tx)
        .await?;
    let raster_receipt_hash = evidence.map(|r| {
        format!(
            "sha256:{:x}",
            Sha256::digest(r.get::<String, _>("payload").as_bytes())
        )
    });
    let result = json!({"deliveryId":delivery.id,"sourceJob":delivery.ticket.source_job,"state":delivery.state,
        "revision":delivery.revision,"deviceJobId":delivery.device_job_id,"reasonCode":delivery.reason,
        "completionMeaning":if delivery.state==DeliveryState::Completed {"device-confirmed"}else{"delivery-terminal"},
        "confirmationSource":if delivery.reason.as_deref()==Some("REMOTE_JOB_RECONCILED") {"remote-job-reconciliation"}else if delivery.device_job_id.is_some(){"delivery-state-observation"}else{"delivery-state-record"},
        "printedAppearanceVerified":false,"rasterResultReceiptSha256":raster_receipt_hash,
        "request":request});
    let issued: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')")
        .fetch_one(&mut **tx)
        .await?;
    let mut receipt = json!({"acx":"0.1","receiptId":format!("{}:delivery-result",delivery.id),
        "provider":"urn:acx:provider:jrip-local","capability":"org.jrip.print.deliver","status":status,"issuedAt":issued,
        "requestHash":format!("sha256:{:x}",Sha256::digest(serde_json::to_vec(&request)?)),
        "resultHash":format!("sha256:{:x}",Sha256::digest(serde_json::to_vec(&result)?)),
        "effects":if delivery.state==DeliveryState::Completed {vec!["device-reported-completion"]}else{vec!["delivery-terminated"]},
        "proof":{"type":"plain-provider-record","signed":false,"profile":"org.jrip.delivery-result-receipt-v1",
            "digestProfile":"org.jrip.serde-json-v1","result":result}});
    if let Some(signer) = signer {
        receipts::sign_stored(tx, signer, &mut receipt, false).await?;
    }
    sqlx::query("INSERT INTO delivery_result_receipts(delivery_id,payload) VALUES(?,?)")
        .bind(delivery.id.to_string())
        .bind(serde_json::to_string(&receipt)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
