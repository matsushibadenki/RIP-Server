//! Historical output inspection never probes a device or queues a send.
use crate::{ApiError, App, artifacts, authorize, internal};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use rip_storage::{
    StorageError,
    delivery::{Delivery, DeliveryTicket},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use uuid::Uuid;
struct Verified {
    delivery: Delivery,
    snapshot: artifacts::ContentSnapshot,
    pages: Vec<Value>,
    contract: Option<Value>,
}
fn mismatch() -> ApiError {
    ApiError(StatusCode::CONFLICT, "delivery_contract_mismatch")
}
async fn verify(app: &App, id: Uuid) -> Result<Verified, ApiError> {
    let delivery = app.repo.delivery(id).await?;
    let ticket = delivery.ticket.clone();
    if !(1..=64 * 1024 * 1024).contains(&ticket.artifact_bytes) {
        return Err(mismatch());
    }
    let contract = match app.repo.delivery_contract(id).await {
        Ok(contract) => {
            if contract["ticketSha256"]
                != format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&contract["ticket"]).map_err(internal)?)
                )
                || contract["ticket"]["schemaVersion"] != 1
                || contract["ticket"]["jobId"] != ticket.source_job.to_string()
                || contract["ticket"]["attemptId"] != id.to_string()
                || !contract["ticket"]["sourceSha256"]
                    .as_str()
                    .is_some_and(rip_core::print::valid_digest)
                || serde_json::from_value::<DeliveryTicket>(contract["ticket"]["delivery"].clone())
                    .map_err(|_| mismatch())?
                    != ticket
            {
                return Err(mismatch());
            }
            Some(contract)
        }
        Err(StorageError::NotFound) => None,
        Err(error) => return Err(error.into()),
    };
    let permit = crate::preview::DECODERS
        .try_acquire()
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "artifact_budget_exhausted"))?;
    let path = app.root.join("deliveries").join(format!("{id}.pwg"));
    let mut snapshot =
        artifacts::content_snapshot(app, path, ticket.artifact_bytes, &ticket.artifact_sha256)
            .await?;
    let mut bytes = Vec::new();
    (&mut snapshot.0)
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(internal)?;
    snapshot.0.rewind().await.map_err(internal)?;
    let pages=tokio::task::spawn_blocking(move || {
        let _permit=permit;
        if bytes.len() as u64!=ticket.artifact_bytes || format!("{:x}",Sha256::digest(&bytes))!=ticket.artifact_sha256 {
            return Err(ApiError(StatusCode::CONFLICT,"delivery_artifact_mismatch"));
        }
        let pages=crate::delivery::raster_pages(&bytes,ticket.sides,ticket.sheet_back)
            .map_err(|_|ApiError(StatusCode::UNPROCESSABLE_ENTITY,"invalid_pwg_raster"))?;
        if pages.is_empty() || ticket.page_ranges.selected_count().is_some_and(|n|n!=pages.len()) {return Err(mismatch())}
        let mut facts=Vec::with_capacity(pages.len());
        for (index,page) in pages.iter().enumerate() {
            let name=&bytes[page.offset+1732..page.offset+1796];
            let end=name.iter().position(|b|*b==0).unwrap_or(64);
            if page.dpi!=ticket.dpi || page.document_type!=ticket.document_type || &name[..end]!=ticket.media.as_bytes() {return Err(mismatch())}
            facts.push(json!({"page":index+1,"width":page.width,"height":page.height,"dpi":[page.dpi,page.dpi],
                "bitsPerComponent":8,"documentType":page.document_type,"colorSpace":match page.document_type {"rgb_8"|"srgb_8"=>"rgb","cmyk_8"=>"cmyk",_=>"gray"}}));
        }
        Ok(facts)
    }).await.map_err(internal)??;
    if let Some(contract) = &contract
        && !contract["ticket"]["selectedSourcePages"].is_null()
    {
        let mapping = contract["ticket"]["selectedSourcePages"]
            .as_array()
            .ok_or_else(mismatch)?;
        let total = contract["ticket"]["sourcePageCount"]
            .as_u64()
            .filter(|n| (1..=100).contains(n))
            .ok_or_else(mismatch)?;
        if mapping.len() != pages.len()
            || mapping
                .iter()
                .any(|n| !n.as_u64().is_some_and(|n| (1..=total).contains(&n)))
            || mapping
                .windows(2)
                .any(|pair| pair[0].as_u64() >= pair[1].as_u64())
        {
            return Err(mismatch());
        }
    }
    Ok(Verified {
        delivery,
        snapshot,
        pages,
        contract,
    })
}
pub async fn describe(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    authorize(&app, &headers, "artifacts:read")?;
    let verified = verify(&app, id).await?;
    let d = &verified.delivery;
    let t = &d.ticket;
    let physical = verified.contract.as_ref().map(|c| &c["ticket"]);
    let source_hash = physical
        .and_then(|p| p.get("sourceSha256"))
        .cloned()
        .unwrap_or(Value::Null);
    let selected = physical
        .and_then(|p| p.get("selectedSourcePages"))
        .cloned()
        .unwrap_or(Value::Null);
    let contract_hash = verified
        .contract
        .as_ref()
        .map(|c| c["ticketSha256"].clone())
        .unwrap_or(Value::Null);
    let retention = app.repo.artifact_retention_policy(t.source_job).await?;
    let value = json!({"schemaVersion":1,"profile":rip_core::print::DELIVERY_ARTIFACT_DESCRIPTOR_PROFILE,"kind":"DeliveryArtifact",
        "contentId":format!("sha256:{}",t.artifact_sha256),"sha256":t.artifact_sha256,"bytes":t.artifact_bytes,"mediaType":"image/pwg-raster",
        "profileSha256":null,"colorReference":t.color_attempt_id.map(|id|format!("/api/v1/color/attempts/{id}")),
        "mcpReference":format!("jrip://deliveries/{id}/artifact/descriptor"),"contentChunksTemplate":format!("jrip://deliveries/{id}/artifact/chunks/{{offset}}/{{length}}"),
        "pageCount":verified.pages.len(),"pages":verified.pages,"copies":t.copies,"sides":t.sides,"sheetBack":t.sheet_back,
        "media":t.media,"selectedSourcePages":selected,"integrityVerified":true,"streamVerified":true,
        "physicalContractVerified":verified.contract.is_some(),"physicalTicketSha256":contract_hash,
        "physicalTicketDigestProfile":if verified.contract.is_some(){Some("org.jrip.serde-json-v1")}else{None},
        "provenance":{"jobId":t.source_job,"sourceSha256":source_hash,"colorAttemptId":t.color_attempt_id,"deliveryId":id},
        "execution":{"deliveryState":d.state,"deliveryRevision":d.revision},"printedAppearanceVerified":false,
        "reference":format!("/api/v1/deliveries/{id}/artifact"),
        "retention":{"reference":format!("/api/v1/jobs/{}/retention",t.source_job),"profile":retention["profile"],
            "revision":retention["revision"],"retainUntil":retention["retainUntil"],"meaning":"minimum-retention-not-automatic-deletion"},
        "expiresAt":null,"authorizationGranted":false});
    Ok((
        [(axum::http::header::CACHE_CONTROL, "private, no-store")],
        Json(value),
    )
        .into_response())
}
pub async fn download(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    authorize(&app, &headers, "artifacts:read")?;
    let verified = verify(&app, id).await?;
    let t = verified.delivery.ticket;
    artifacts::content_response(
        headers,
        verified.snapshot,
        artifacts::ContentMetadata {
            name: "delivery.pwg".into(),
            media_type: "image/pwg-raster".into(),
            bytes: t.artifact_bytes,
            sha256: t.artifact_sha256,
        },
    )
    .await
}
