//! Source bytes are verified independently of previously recorded interpreter evidence.
use crate::{ApiError, App, artifacts, authorize, internal};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use rip_core::{DocumentFormat, Job};
use rip_storage::StorageError;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use uuid::Uuid;
fn mismatch() -> ApiError {
    ApiError(StatusCode::CONFLICT, "source_descriptor_mismatch")
}
async fn snapshot(app: &App, id: Uuid) -> Result<(Job, u64, artifacts::ContentSnapshot), ApiError> {
    let job = app.repo.get(id).await?;
    let path = app.root.join("jobs").join(id.to_string()).join("document");
    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "source_unavailable"))?;
    let bytes = metadata.len();
    if !metadata.is_file() || !(1..=rip_jobs::MAX_DOCUMENT_BYTES).contains(&bytes) {
        return Err(mismatch());
    }
    match app.repo.job_ticket(id).await {
        Ok(ticket)
            if ticket.document.sha256 != job.source_sha256
                || ticket.document.media_type != job.manifest.format
                || ticket.document.bytes.is_some_and(|n| n != bytes) =>
        {
            return Err(mismatch());
        }
        Ok(_) | Err(StorageError::NotFound) => {}
        Err(error) => return Err(error.into()),
    }
    let mut snapshot = artifacts::content_snapshot(app, path, bytes, &job.source_sha256).await?;
    let mut prefix = [0u8; 100];
    let mut used = 0;
    while used < prefix.len() {
        let n = snapshot
            .0
            .read(&mut prefix[used..])
            .await
            .map_err(internal)?;
        if n == 0 {
            break;
        }
        used += n;
    }
    if DocumentFormat::detect(&prefix[..used]).map_err(|_| mismatch())? != job.manifest.format {
        return Err(mismatch());
    }
    snapshot.0.rewind().await.map_err(internal)?;
    Ok((job, bytes, snapshot))
}
pub async fn describe(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    authorize(&app, &headers, "artifacts:read")?;
    let (job, bytes, _snapshot) = snapshot(&app, id).await?;
    let mut geometry = Value::Null;
    let mut page_profiles = Value::Null;
    let validation = match app.repo.source_validation(id).await {
        Ok(report) => {
            if report["profile"] != "org.jrip.source-validation-v1"
                || report["jobId"] != id.to_string()
                || report["sourceSha256"] != job.source_sha256
                || report["sourceBytes"] != bytes
                || report["format"]
                    != serde_json::to_value(job.manifest.format).map_err(internal)?
                || report["strictPdfInspection"] != (job.manifest.format == DocumentFormat::Pdf)
                || report["allPagesInterpreted"] != true
                || report["scope"] != "bounded-page-content-interpretation"
                || !report["validatedAt"].as_i64().is_some_and(|n| n >= 0)
                || !report["pageCount"]
                    .as_u64()
                    .is_some_and(|n| (1..=100).contains(&n))
            {
                return Err(mismatch());
            }
            let analysis = &report["colorProfileAnalysis"]["analysis"];
            if !analysis["pageProfileDeclarations"].is_null() {
                if analysis["sourceSha256"] != job.source_sha256
                    || analysis["sourceBytes"] != bytes
                    || report["strictPdfInspection"] != true
                    || report["colorProfileAnalysis"]["analysisDigest"]
                        != rip_digest::digest(analysis).map_err(internal)?
                {
                    return Err(mismatch());
                }
                rip_core::pages::validate_page_profile_declarations(
                    analysis,
                    report["pageCount"].as_u64().map(|n| n as u32),
                )
                .map_err(|_| mismatch())?;
                page_profiles = json!({"declarations":analysis["pageProfileDeclarations"],"profiles":analysis["profiles"],"analysisDigest":report["colorProfileAnalysis"]["analysisDigest"],"reference":format!("/api/v1/jobs/{id}/source-validation")});
            }
            if let Some(bounds) = report.get("sourceGeometry") {
                if report["strictPdfInspection"] != true
                    || bounds["profile"] != "org.jrip.pdf-display-bounds-v1"
                    || bounds["unit"] != "point-1/72-inch"
                    || bounds["meaning"] != "crop-rotation-user-unit-display-bounds"
                {
                    return Err(mismatch());
                }
                rip_core::pages::validate_source_geometry(
                    &bounds["pages"],
                    report["pageCount"].as_u64().unwrap() as u32,
                )
                .map_err(|_| mismatch())?;
                geometry = bounds.clone();
            }
            json!({"profile":report["profile"],"scope":report["scope"],"strictPdfInspection":report["strictPdfInspection"],
                "pageCount":report["pageCount"],"validatedAt":report["validatedAt"],
                "reference":format!("/api/v1/jobs/{id}/source-validation"),
                "profileAnalysisDigest":report["colorProfileAnalysis"]["analysisDigest"]})
        }
        Err(StorageError::NotFound) => Value::Null,
        Err(error) => return Err(error.into()),
    };
    let retention = app.repo.artifact_retention_policy(id).await?;
    let value = json!({"schemaVersion":1,"profile":rip_core::print::SOURCE_ARTIFACT_DESCRIPTOR_PROFILE,"kind":"SourceDocument",
        "uri":format!("jrip://jobs/{id}/source"),"contentId":format!("sha256:{}",job.source_sha256),"sha256":job.source_sha256,"bytes":bytes,"mediaType":job.manifest.format,
        "pageCount":validation["pageCount"],"dimensions":null,"sourceGeometry":geometry,"pageProfileDeclarations":page_profiles,"colorSpace":null,"profileSha256":null,
        "integrityVerified":true,"boundedInterpretationVerified":!validation.is_null(),"structureVerified":validation["strictPdfInspection"]==true,"validation":validation,
        "mcpReference":format!("jrip://jobs/{id}/source/descriptor"),"contentChunksTemplate":format!("jrip://jobs/{id}/source/chunks/{{offset}}/{{length}}"),
        "provenance":{"jobId":id,"admittedAt":job.created_at},"reference":format!("/api/v1/jobs/{id}/source"),
        "retention":{"reference":format!("/api/v1/jobs/{id}/retention"),"profile":retention["profile"],"revision":retention["revision"],
            "retainUntil":retention["retainUntil"],"meaning":"minimum-retention-not-automatic-deletion"},"expiresAt":null,"authorizationGranted":false});
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
    let (job, bytes, snapshot) = snapshot(&app, id).await?;
    let (extension, mime) = match job.manifest.format {
        DocumentFormat::Pdf => ("pdf", "application/pdf"),
        DocumentFormat::Postscript => ("ps", "application/postscript"),
        DocumentFormat::Eps => ("eps", "application/eps"),
    };
    artifacts::content_response(
        headers,
        snapshot,
        artifacts::ContentMetadata {
            name: format!("source.{extension}"),
            media_type: mime.into(),
            bytes,
            sha256: job.source_sha256,
        },
    )
    .await
}
