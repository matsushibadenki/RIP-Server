use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Retry {
    expected_revision: i64,
    request_id: Uuid,
}
pub async fn retry(
    State(app): State<App>,
    headers: HeaderMap,
    Path(source): Path<Uuid>,
    Json(request): Json<Retry>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:control")?;
    authorize(&app, &headers, "jobs:submit")?;
    if request.expected_revision < 0 {
        return Err(bad("invalid_revision"));
    }
    let original = app.repo.get(source).await?;
    let payload = json!({"capability":rip_acx::RIP_CAPABILITY_ID,"retrySource":source,"retryRevision":request.expected_revision,
        "requestHash":format!("sha256:{:x}",Sha256::digest(serde_json::to_vec(&original.manifest).map_err(internal)?)),
        "preflightDigest":format!("retry:{}:{}:{}",source,request.expected_revision,request.request_id)});
    app.repo
        .retry_preflight(
            request.request_id,
            source,
            request.expected_revision,
            &payload,
        )
        .await?;
    if let Some(id) = app.repo.committed_job(request.request_id).await? {
        return Ok(Json(
            json!({"job":app.repo.get(id).await?,"retryOf":source,"replayed":true}),
        ));
    }
    // Keep the original attempt and its receipts immutable; every retry creates a new job.
    let path = app
        .root
        .join("jobs")
        .join(source.to_string())
        .join("document");
    if tokio::fs::canonicalize(&path).await.map_err(internal)? != path {
        return Err(bad("invalid_source"));
    }
    let file = tokio::fs::File::open(path).await.map_err(internal)?;
    if !file.metadata().await.map_err(internal)?.is_file() {
        return Err(bad("invalid_source"));
    }
    let id = Uuid::new_v4();
    let directory = app.root.join("jobs").join(id.to_string());
    tokio::fs::create_dir(&directory).await.map_err(internal)?;
    let result = async {
        let mut file = file.take(MAX_DOCUMENT_BYTES as u64 + 1);
        let mut output = tokio::fs::File::create(directory.join("source.part"))
            .await
            .map_err(internal)?;
        tokio::io::copy(&mut file, &mut output)
            .await
            .map_err(internal)?;
        output.sync_all().await.map_err(internal)?;
        rip_jobs::publish_bound(
            &app.repo,
            &directory,
            id,
            original.manifest,
            Some((request.request_id, &original.source_sha256)),
        )
        .await
        .map_err(|e| match e {
            SubmitError::Invalid(code) => bad(code),
            SubmitError::Storage(e) => e.into(),
            SubmitError::Io(e) => internal(e),
        })
    }
    .await;
    match result {
        Ok(job) => Ok(Json(json!({"job":job,"retryOf":source,"replayed":false}))),
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&directory).await;
            if error.0 == StatusCode::CONFLICT
                && let Some(id) = app.repo.committed_job(request.request_id).await?
            {
                return Ok(Json(
                    json!({"job":app.repo.get(id).await?,"retryOf":source,"replayed":true}),
                ));
            }
            Err(error)
        }
    }
}
