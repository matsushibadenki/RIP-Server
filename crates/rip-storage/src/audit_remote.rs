//! Conditional HTTP object vault adapter; does not claim S3 or WORM compatibility.
use super::*;
use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, HeaderValue, IF_NONE_MATCH},
};
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error("invalid_remote_vault_configuration")]
    Configuration,
    #[error("remote_vault_transport_failed")]
    Transport,
    #[error("remote_vault_response_rejected")]
    Response,
    #[error("remote_vault_operation_conflict")]
    Conflict,
    #[error("remote_vault_content_mismatch")]
    Content,
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

pub struct RemoteVault {
    base: Url,
    client: Client,
    authorization: HeaderValue,
}
impl RemoteVault {
    pub fn new(endpoint: &str, token: &str) -> Result<Self, RemoteError> {
        let base = Url::parse(endpoint).map_err(|_| RemoteError::Configuration)?;
        if base.scheme() != "https"
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || !base.path().ends_with('/')
            || token.is_empty()
            || token.len() > 4096
        {
            return Err(RemoteError::Configuration);
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| RemoteError::Configuration)?;
        authorization.set_sensitive(true);
        let client = Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| RemoteError::Configuration)?;
        Ok(Self {
            base,
            client,
            authorization,
        })
    }
    async fn read(&self, id: Uuid) -> Result<Option<Vec<u8>>, RemoteError> {
        Ok(self.read_version(id).await?.map(|(bytes, _)| bytes))
    }
    async fn read_version(
        &self,
        id: Uuid,
    ) -> Result<Option<(Vec<u8>, Option<String>)>, RemoteError> {
        let url = self
            .base
            .join(&format!("{id}.json"))
            .map_err(|_| RemoteError::Configuration)?;
        let mut response = self
            .client
            .get(url)
            .header("cache-control", "no-cache")
            .header(AUTHORIZATION, self.authorization.clone())
            .send()
            .await
            .map_err(|_| RemoteError::Transport)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if response.status() != reqwest::StatusCode::OK
            || response
                .content_length()
                .is_some_and(|n| n > audit_archive::MAX_BYTES as u64)
            || response
                .headers()
                .get("content-encoding")
                .is_some_and(|h| h != "identity")
        {
            return Err(RemoteError::Response);
        }
        let etag = response
            .headers()
            .get("etag")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| RemoteError::Transport)? {
            if bytes.len() + chunk.len() > audit_archive::MAX_BYTES {
                return Err(RemoteError::Response);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some((bytes, etag)))
    }
    async fn write(&self, id: Uuid, bytes: Vec<u8>) -> Result<(), RemoteError> {
        let url = self
            .base
            .join(&format!("{id}.json"))
            .map_err(|_| RemoteError::Configuration)?;
        let status = self
            .client
            .put(url)
            .header(AUTHORIZATION, self.authorization.clone())
            .header(IF_NONE_MATCH, "*")
            .header("content-type", "application/json")
            .body(bytes)
            .send()
            .await
            .map_err(|_| RemoteError::Transport)?
            .status();
        if !matches!(status.as_u16(), 200 | 201 | 204 | 412) {
            return Err(RemoteError::Response);
        }
        Ok(())
    }
}

fn strong_etag(value: &str) -> bool {
    value.len() >= 2
        && value.len() <= 1024
        && value.starts_with('"')
        && value.ends_with('"')
        && value.as_bytes()[1..value.len() - 1]
            .iter()
            .all(|c| *c >= 0x21 && *c != b'"' && *c != 0x7f)
}

impl Repository {
    fn remote_operation_lock(&self, id: Uuid) -> Result<std::fs::File, RemoteError> {
        let db = std::fs::canonicalize(self.pool.connect_options().get_filename())
            .map_err(|_| RemoteError::Conflict)?;
        let key = format!("{:x}", Sha256::digest(db.as_os_str().as_encoded_bytes()));
        let path = db
            .parent()
            .ok_or(RemoteError::Conflict)?
            .join(format!(".jrip-remote-{key}-{id}.lock"));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map_err(|_| RemoteError::Conflict)?;
        file.try_lock().map_err(|_| RemoteError::Conflict)?;
        Ok(file)
    }

    pub async fn store_audit_remote(
        &self,
        id: Uuid,
        vault: &RemoteVault,
    ) -> Result<Value, RemoteError> {
        let _lock = self.remote_operation_lock(id)?;
        let blocked: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM audit_remote_prunes WHERE archive_id=? AND target=?)",
        )
        .bind(id.to_string())
        .bind(vault.base.as_str())
        .fetch_one(&self.pool)
        .await?;
        if blocked != 0 {
            return Err(RemoteError::Conflict);
        }
        let archive = self.audit_archive(id).await?;
        if !audit_archive::verify(&archive) || archive["payload"]["id"] != id.to_string() {
            return Err(RemoteError::Content);
        }
        let bytes = serde_json::to_vec(&archive)?;
        if bytes.len() > audit_archive::MAX_BYTES {
            return Err(RemoteError::Content);
        }
        vault.write(id, bytes.clone()).await?;
        if vault.read(id).await?.as_deref() != Some(bytes.as_slice()) {
            return Err(RemoteError::Content);
        }
        let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        let target = vault.base.as_str();
        let time = now();
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query("INSERT OR IGNORE INTO audit_remote_exports(archive_id,target,digest,bytes,verified_at) VALUES(?,?,?,?,?)")
            .bind(id.to_string()).bind(target).bind(&digest).bind(bytes.len() as i64).bind(time).execute(&mut *tx).await?.rows_affected()!=0;
        if inserted {
            sqlx::query("INSERT INTO audit_records(correlation_id,identity,operation,target,started_at,finished_at,status) VALUES(?,?,?,?,?,?,200)")
                .bind(Uuid::new_v4().to_string()).bind("local-audit-remote").bind("audit-remote-store")
                .bind(id.to_string()).bind(time).bind(time).execute(&mut *tx).await?;
        }
        let verified_at: i64 = sqlx::query_scalar(
            "SELECT verified_at FROM audit_remote_exports WHERE archive_id=? AND target=?",
        )
        .bind(id.to_string())
        .bind(target)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(
            json!({"archiveId":id,"target":target,"fileDigest":digest,"bytes":bytes.len(),"verifiedAt":verified_at,
            "replayed":!inserted,"storageMeaning":"http-readback","worm":false}),
        )
    }
    pub async fn remote_prune_record(
        &self,
        id: Uuid,
        vault: &RemoteVault,
    ) -> Result<Value, RemoteError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT payload FROM audit_remote_prunes WHERE archive_id=? AND target=?",
        )
        .bind(id.to_string())
        .bind(vault.base.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(serde_json::from_str(&value.ok_or(StorageError::NotFound)?)?)
    }

    async fn finish_remote_prune(
        &self,
        id: Uuid,
        vault: &RemoteVault,
        state: &str,
    ) -> Result<Value, RemoteError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE audit_remote_prunes SET state=state WHERE archive_id=? AND target=?")
            .bind(id.to_string())
            .bind(vault.base.as_str())
            .execute(&mut *tx)
            .await?;
        let payload: String = sqlx::query_scalar(
            "SELECT payload FROM audit_remote_prunes WHERE archive_id=? AND target=?",
        )
        .bind(id.to_string())
        .bind(vault.base.as_str())
        .fetch_one(&mut *tx)
        .await?;
        let mut record: Value = serde_json::from_str(&payload)?;
        if record["state"] == "CONFIRMED_ABSENT" || record["state"] == "REJECTED" {
            tx.commit().await?;
            return Ok(record);
        }
        record["state"] = json!(state);
        record["localSnapshotProtected"] = json!(state == "UNKNOWN" || state == "PREPARED");
        record["observedAt"] = json!(now());
        sqlx::query(
            "UPDATE audit_remote_prunes SET state=?,payload=? WHERE archive_id=? AND target=?",
        )
        .bind(state)
        .bind(serde_json::to_string(&record)?)
        .bind(id.to_string())
        .bind(vault.base.as_str())
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO audit_records(correlation_id,identity,operation,target,started_at,finished_at,status) VALUES(?,?,?,?,?,?,200)")
            .bind(Uuid::new_v4().to_string()).bind("local-audit-remote").bind(format!("audit-remote-prune-{state}"))
            .bind(id.to_string()).bind(now()).bind(now()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(record)
    }

    pub async fn reconcile_remote_prune(
        &self,
        id: Uuid,
        vault: &RemoteVault,
    ) -> Result<Value, RemoteError> {
        let _lock = self.remote_operation_lock(id)?;
        let record = self.remote_prune_record(id, vault).await?;
        if record["state"] == "CONFIRMED_ABSENT" || record["state"] == "REJECTED" {
            return Ok(record);
        }
        let state = match vault.read(id).await {
            Ok(None) => "CONFIRMED_ABSENT",
            _ => "UNKNOWN",
        };
        self.finish_remote_prune(id, vault, state).await
    }

    pub async fn prune_audit_remote(
        &self,
        id: Uuid,
        vault: &RemoteVault,
        before: i64,
        expected_digest: &str,
    ) -> Result<Value, RemoteError> {
        if before < 0 || before > now() {
            return Err(RemoteError::Conflict);
        }
        let _lock = self.remote_operation_lock(id)?;
        match self.remote_prune_record(id, vault).await {
            Ok(record) => {
                if record["fileDigest"] != expected_digest {
                    return Err(RemoteError::Conflict);
                }
                return Ok(record); // Never resend a DELETE, including after interruption.
            }
            Err(RemoteError::Storage(StorageError::NotFound)) => {}
            Err(error) => return Err(error),
        }
        let (remote, etag) = vault.read_version(id).await?.ok_or(RemoteError::Content)?;
        let etag = etag
            .filter(|e| strong_etag(e))
            .ok_or(RemoteError::Response)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE audit_archives SET payload=payload WHERE id=?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        let local: Option<String> =
            sqlx::query_scalar("SELECT payload FROM audit_archives WHERE id=?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        let archive: Value = serde_json::from_str(&local.ok_or(StorageError::NotFound)?)?;
        let bytes = serde_json::to_vec(&archive)?;
        let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        if !audit_archive::verify(&archive)
            || archive["payload"]["id"] != id.to_string()
            || bytes != remote
            || digest != expected_digest
        {
            return Err(RemoteError::Content);
        }
        let eligible: i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_remote_exports WHERE archive_id=? AND target=? AND digest=? AND bytes=? AND verified_at<?)")
            .bind(id.to_string()).bind(vault.base.as_str()).bind(&digest).bind(bytes.len() as i64).bind(before).fetch_one(&mut *tx).await?;
        if eligible == 0 {
            return Err(RemoteError::Conflict);
        }
        let record = json!({"profile":"org.jrip.audit-remote-prune-v1","archiveId":id,"target":vault.base.as_str(),
            "state":"PREPARED","fileDigest":digest,"bytes":bytes.len(),"etag":etag,"before":before,"preparedAt":now(),
            "confirmationMeaning":"observed-absent","sourceAuditRecordsRetained":true,"localSnapshotProtected":true});
        sqlx::query("INSERT INTO audit_remote_prunes(archive_id,target,state,payload) VALUES(?,?,'PREPARED',?)")
            .bind(id.to_string()).bind(vault.base.as_str()).bind(serde_json::to_string(&record)?).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO audit_records(correlation_id,identity,operation,target,started_at,finished_at,status) VALUES(?,?,?,?,?,?,200)")
            .bind(Uuid::new_v4().to_string()).bind("local-audit-remote").bind("audit-remote-prune-prepare")
            .bind(id.to_string()).bind(now()).bind(now()).execute(&mut *tx).await?;
        tx.commit().await?;
        let url = vault
            .base
            .join(&format!("{id}.json"))
            .map_err(|_| RemoteError::Configuration)?;
        let response = vault
            .client
            .delete(url)
            .header(AUTHORIZATION, vault.authorization.clone())
            .header("if-match", &etag)
            .send()
            .await;
        let state = match response {
            Ok(response) if response.status().as_u16() == 412 => "REJECTED",
            Ok(response) if matches!(response.status().as_u16(), 200 | 204) => {
                match vault.read(id).await {
                    Ok(None) => "CONFIRMED_ABSENT",
                    _ => "UNKNOWN",
                }
            }
            _ => "UNKNOWN",
        };
        self.finish_remote_prune(id, vault, state).await
    }

    /// Assess remote-copy retention without deleting objects or local evidence.
    pub async fn preview_remote_retention(
        &self,
        vault: &RemoteVault,
        before: i64,
        after: &str,
        limit: i64,
    ) -> Result<Value, RemoteError> {
        if before < 0
            || before > now()
            || !(1..=20).contains(&limit)
            || (!after.is_empty()
                && !Uuid::parse_str(after).is_ok_and(|id| id.to_string() == after))
        {
            return Err(StorageError::Conflict.into());
        }
        let rows = sqlx::query("SELECT archive_id,digest,bytes,verified_at,(SELECT state FROM audit_remote_prunes p WHERE p.archive_id=audit_remote_exports.archive_id AND p.target=audit_remote_exports.target) AS prune_state FROM audit_remote_exports WHERE target=? AND archive_id>? AND verified_at<? ORDER BY archive_id LIMIT ?")
            .bind(vault.base.as_str()).bind(after).bind(before).bind(limit+1).fetch_all(&self.pool).await?;
        let more = rows.len() > limit as usize;
        let mut cursor = after.to_owned();
        let mut candidates = Vec::new();
        for row in rows.into_iter().take(limit as usize) {
            cursor = row.get("archive_id");
            let id = Uuid::parse_str(&cursor).map_err(|_| RemoteError::Content)?;
            let digest: String = row.get("digest");
            let bytes: i64 = row.get("bytes");
            let verified_at: i64 = row.get("verified_at");
            let local = match self.audit_archive(id).await {
                Ok(archive)
                    if audit_archive::verify(&archive) && archive["payload"]["id"] == cursor =>
                {
                    let data = serde_json::to_vec(&archive)?;
                    if data.len() as i64 != bytes
                        || format!("sha256:{:x}", Sha256::digest(&data)) != digest
                    {
                        Some("LOCAL_COPY_DIGEST_MISMATCH")
                    } else {
                        None
                    }
                }
                Ok(_) => Some("LOCAL_SNAPSHOT_INVALID"),
                Err(StorageError::NotFound) => Some("LOCAL_SNAPSHOT_MISSING"),
                Err(_) => Some("LOCAL_EVIDENCE_UNAVAILABLE"),
            };
            let recorded_prune: Option<String> = row.get("prune_state");
            let reason = if recorded_prune.is_some() {
                "DELETION_ALREADY_RECORDED"
            } else if let Some(reason) = local {
                reason
            } else {
                match vault.read(id).await {
                    Ok(Some(value))
                        if value.len() as i64 == bytes
                            && format!("sha256:{:x}", Sha256::digest(&value)) == digest =>
                    {
                        "VERIFIED_LOCAL_AND_REMOTE_COPIES"
                    }
                    Ok(_) => "REMOTE_CONTENT_MISMATCH",
                    Err(_) => "REMOTE_EVIDENCE_UNAVAILABLE",
                }
            };
            candidates.push(json!({"archiveId":id,"fileDigest":digest,"bytes":bytes,"firstVerifiedAt":verified_at,
                "remoteCopyPruneCandidate":reason=="VERIFIED_LOCAL_AND_REMOTE_COPIES","reason":reason}));
        }
        Ok(
            json!({"dryRun":true,"remoteDeletionImplemented":true,"target":vault.base.as_str(),"before":before,
            "retentionAgeBasis":"first-remote-acknowledgement","candidates":candidates,"nextAfter":cursor,"hasMore":more,
            "storageMeaning":"http-readback","worm":false}),
        )
    }

    pub async fn audit_remote_status(
        &self,
        id: Uuid,
        vault: &RemoteVault,
    ) -> Result<Value, RemoteError> {
        let (digest, bytes) = match self.audit_archive(id).await {
            Ok(archive) => {
                if !audit_archive::verify(&archive) || archive["payload"]["id"] != id.to_string() {
                    return Err(RemoteError::Content);
                }
                let bytes = serde_json::to_vec(&archive)?;
                (
                    format!("sha256:{:x}", Sha256::digest(&bytes)),
                    bytes.len() as i64,
                )
            }
            Err(StorageError::NotFound) => {
                let record = self.audit_archive_prune_record(id).await?;
                (
                    record["fileDigest"]
                        .as_str()
                        .ok_or(RemoteError::Content)?
                        .to_owned(),
                    record["bytes"].as_i64().ok_or(RemoteError::Content)?,
                )
            }
            Err(error) => return Err(error.into()),
        };
        let recorded: Option<i64> = sqlx::query_scalar(
            "SELECT verified_at FROM audit_remote_exports WHERE archive_id=? AND target=?",
        )
        .bind(id.to_string())
        .bind(vault.base.as_str())
        .fetch_optional(&self.pool)
        .await?;
        let matches = vault.read(id).await?.is_some_and(|value| {
            value.len() as i64 == bytes && format!("sha256:{:x}", Sha256::digest(&value)) == digest
        });
        Ok(
            json!({"archiveId":id,"target":vault.base.as_str(),"acknowledged":recorded.is_some(),"verifiedAt":recorded,
            "contentMatches":matches,"storageMeaning":"http-readback","worm":false}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Bytes,
        extract::State,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicUsize, Ordering},
    };
    #[derive(Default)]
    struct Fake {
        object: Mutex<Option<Vec<u8>>>,
        mode: AtomicU8,
        leaks: AtomicUsize,
        deletes: AtomicUsize,
    }
    async fn put(
        State(fake): State<Arc<Fake>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> axum::response::Response {
        if headers
            .get("authorization")
            .is_none_or(|h| h != "Bearer test-secret")
            || headers.get("if-none-match").is_none_or(|h| h != "*")
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        if fake.mode.load(Ordering::SeqCst) == 1 {
            return (StatusCode::TEMPORARY_REDIRECT, [("location", "/leak")]).into_response();
        }
        let mut object = fake.object.lock().unwrap();
        if object.is_some() {
            return StatusCode::PRECONDITION_FAILED.into_response();
        }
        *object = Some(body.to_vec());
        StatusCode::CREATED.into_response()
    }
    async fn delete(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> StatusCode {
        if headers
            .get("authorization")
            .is_none_or(|h| h != "Bearer test-secret")
            || headers.get("if-match").is_none_or(|h| h != "\"v1\"")
        {
            return StatusCode::PRECONDITION_FAILED;
        }
        fake.deletes.fetch_add(1, Ordering::SeqCst);
        if fake.mode.load(Ordering::SeqCst) == 6 {
            return StatusCode::PRECONDITION_FAILED;
        }
        *fake.object.lock().unwrap() = None;
        if fake.mode.load(Ordering::SeqCst) == 4 {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::NO_CONTENT
        }
    }
    async fn read(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> axum::response::Response {
        if headers
            .get("authorization")
            .is_none_or(|h| h != "Bearer test-secret")
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        match fake.mode.load(Ordering::SeqCst) {
            2 => return (StatusCode::OK, vec![0u8; audit_archive::MAX_BYTES + 1]).into_response(),
            3 => return (StatusCode::OK, "tampered").into_response(),
            _ => {}
        }
        match fake.object.lock().unwrap().clone() {
            Some(bytes) if fake.mode.load(Ordering::SeqCst) == 5 => {
                (StatusCode::OK, bytes).into_response()
            }
            Some(bytes) => (StatusCode::OK, [("etag", "\"v1\"")], bytes).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }
    #[test]
    fn production_configuration_requires_https_and_secret_free_url() {
        for endpoint in [
            "http://example.test/",
            "https://user:secret@example.test/",
            "https://example.test/?token=x",
            "https://example.test/#x",
            "https://example.test/no-slash",
        ] {
            assert!(RemoteVault::new(endpoint, "token").is_err());
        }
        assert!(RemoteVault::new("https://example.test/vault/", "").is_err());
        assert!(RemoteVault::new("https://example.test/vault/", "bad\nvalue").is_err());
    }
    #[tokio::test]
    async fn conditional_object_store_readback_replay_and_failures_are_audited_conservatively() {
        let fake = Arc::new(Fake::default());
        let leak = fake.clone();
        let router = Router::new()
            .route("/vault/{id}", get(read).put(put).delete(delete))
            .route(
                "/leak",
                get(move || {
                    let leak = leak.clone();
                    async move {
                        leak.leaks.fetch_add(1, Ordering::SeqCst);
                        StatusCode::OK
                    }
                }),
            )
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        // Private test transport only; production constructor cannot use plaintext HTTP.
        let vault = RemoteVault {
            base: Url::parse(&format!("http://{address}/vault/")).unwrap(),
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            authorization: HeaderValue::from_static("Bearer test-secret"),
        };
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db");
        let repo = Repository::open_with_receipt_signer(&db, None)
            .await
            .unwrap();
        let seq = repo
            .begin_audit(Uuid::new_v4(), "operator", "test", None)
            .await
            .unwrap();
        repo.finish_audit(seq, 200).await.unwrap();
        let archive = repo.archive_audit(0, now() + 1, 100).await.unwrap();
        let id = Uuid::parse_str(archive["payload"]["id"].as_str().unwrap()).unwrap();
        fake.mode.store(1, Ordering::SeqCst);
        assert!(repo.store_audit_remote(id, &vault).await.is_err());
        assert_eq!(fake.leaks.load(Ordering::SeqCst), 0);
        fake.mode.store(2, Ordering::SeqCst);
        assert!(repo.store_audit_remote(id, &vault).await.is_err());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_remote_exports")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
        fake.mode.store(0, Ordering::SeqCst);
        sqlx::query("CREATE TRIGGER fail_remote_audit BEFORE INSERT ON audit_records WHEN NEW.operation='audit-remote-store' BEGIN SELECT RAISE(ABORT,'test failure'); END").execute(&repo.pool).await.unwrap();
        assert!(repo.store_audit_remote(id, &vault).await.is_err());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_remote_exports")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
        sqlx::query("DROP TRIGGER fail_remote_audit")
            .execute(&repo.pool)
            .await
            .unwrap();
        let first = repo.store_audit_remote(id, &vault).await.unwrap();
        assert_eq!(first["replayed"], false);
        let reopened = Repository::open_with_receipt_signer(&db, None)
            .await
            .unwrap();
        let replay = reopened.store_audit_remote(id, &vault).await.unwrap();
        assert_eq!(replay["replayed"], true);
        assert_eq!(first["verifiedAt"], replay["verifiedAt"]);
        assert_eq!(
            reopened.audit_remote_status(id, &vault).await.unwrap()["contentMatches"],
            true
        );
        fake.mode.store(3, Ordering::SeqCst);
        assert!(reopened.store_audit_remote(id, &vault).await.is_err());
        assert_eq!(
            reopened.audit_remote_status(id, &vault).await.unwrap()["contentMatches"],
            false
        );
        assert_eq!(
            *fake.object.lock().unwrap(),
            Some(serde_json::to_vec(&archive).unwrap())
        );
        fake.mode.store(0, Ordering::SeqCst);
        let fresh = reopened
            .preview_remote_retention(&vault, now() - 86400 * 30, "", 20)
            .await
            .unwrap();
        assert_eq!(fresh["candidates"], json!([]));
        sqlx::query("UPDATE audit_remote_exports SET verified_at=?")
            .bind(now() - 86400 * 40)
            .execute(&repo.pool)
            .await
            .unwrap();
        let preview = reopened
            .preview_remote_retention(&vault, now() - 86400 * 30, "", 1)
            .await
            .unwrap();
        assert_eq!(preview["candidates"][0]["remoteCopyPruneCandidate"], true);
        assert_eq!(preview["remoteDeletionImplemented"], true);
        assert_eq!(preview["hasMore"], false);
        assert_eq!(
            reopened
                .preview_remote_retention(&vault, now(), preview["nextAfter"].as_str().unwrap(), 1)
                .await
                .unwrap()["candidates"],
            json!([])
        );
        fake.mode.store(3, Ordering::SeqCst);
        let changed = reopened
            .preview_remote_retention(&vault, now(), "", 20)
            .await
            .unwrap();
        assert_eq!(
            changed["candidates"][0]["reason"],
            "REMOTE_CONTENT_MISMATCH"
        );
        fake.mode.store(2, Ordering::SeqCst);
        assert_eq!(
            reopened
                .preview_remote_retention(&vault, now(), "", 20)
                .await
                .unwrap()["candidates"][0]["reason"],
            "REMOTE_EVIDENCE_UNAVAILABLE"
        );
        sqlx::query("DELETE FROM audit_archives WHERE id=?")
            .bind(id.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        let missing = reopened
            .preview_remote_retention(&vault, now(), "", 20)
            .await
            .unwrap();
        assert_eq!(missing["candidates"][0]["remoteCopyPruneCandidate"], false);
        assert_eq!(missing["candidates"][0]["reason"], "LOCAL_SNAPSHOT_MISSING");
        assert!(
            reopened
                .preview_remote_retention(&vault, now() + 60, "", 20)
                .await
                .is_err()
        );
        assert!(
            reopened
                .preview_remote_retention(&vault, now(), "invalid", 20)
                .await
                .is_err()
        );
        assert!(
            reopened
                .preview_remote_retention(&vault, now(), "", 21)
                .await
                .is_err()
        );
        sqlx::query("INSERT INTO audit_archives(id,payload) VALUES(?,?)")
            .bind(id.to_string())
            .bind(serde_json::to_string(&archive).unwrap())
            .execute(&repo.pool)
            .await
            .unwrap();
        let file_digest = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&archive).unwrap())
        );
        fake.mode.store(5, Ordering::SeqCst);
        assert!(
            repo.prune_audit_remote(id, &vault, now(), &file_digest)
                .await
                .is_err()
        );
        assert_eq!(fake.deletes.load(Ordering::SeqCst), 0);
        let held = repo.remote_operation_lock(id).unwrap();
        assert!(repo.store_audit_remote(id, &vault).await.is_err());
        drop(held);
        fake.mode.store(0, Ordering::SeqCst);
        sqlx::query("CREATE TRIGGER fail_remote_prepare BEFORE INSERT ON audit_records WHEN NEW.operation='audit-remote-prune-prepare' BEGIN SELECT RAISE(ABORT,'test failure'); END").execute(&repo.pool).await.unwrap();
        assert!(
            repo.prune_audit_remote(id, &vault, now(), &file_digest)
                .await
                .is_err()
        );
        assert!(repo.remote_prune_record(id, &vault).await.is_err());
        assert_eq!(fake.deletes.load(Ordering::SeqCst), 0);
        sqlx::query("DROP TRIGGER fail_remote_prepare")
            .execute(&repo.pool)
            .await
            .unwrap();
        fake.mode.store(4, Ordering::SeqCst);
        let unknown = repo
            .prune_audit_remote(id, &vault, now(), &file_digest)
            .await
            .unwrap();
        assert_eq!(unknown["state"], "UNKNOWN");
        let restarted = Repository::open_with_receipt_signer(&db, None)
            .await
            .unwrap();
        assert_eq!(
            restarted
                .prune_audit_remote(id, &vault, now(), &file_digest)
                .await
                .unwrap(),
            unknown
        );
        assert_eq!(fake.deletes.load(Ordering::SeqCst), 1);
        assert!(restarted.store_audit_remote(id, &vault).await.is_err());
        let fs_vault = dir.path().join("fs-vault");
        std::fs::create_dir(&fs_vault).unwrap();
        let blocked = restarted
            .prune_audit_archive(id, &fs_vault, now(), archive["digest"].as_str().unwrap())
            .await
            .unwrap_err();
        assert!(blocked.to_string().contains("local snapshot protected"));
        let confirmed = restarted.reconcile_remote_prune(id, &vault).await.unwrap();
        assert_eq!(confirmed["state"], "CONFIRMED_ABSENT");
        assert_eq!(
            restarted.reconcile_remote_prune(id, &vault).await.unwrap(),
            confirmed
        );
        assert_eq!(fake.deletes.load(Ordering::SeqCst), 1);
        assert_eq!(restarted.audit_archive(id).await.unwrap(), archive);
        let rows = reopened.audit_records(0, 100).await.unwrap();
        assert_eq!(
            rows["records"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["operation"] == "audit-remote-store")
                .count(),
            1
        );
        assert!(
            !serde_json::to_string(&rows)
                .unwrap()
                .contains("test-secret")
        );
        let second = repo.archive_audit(0, now() + 1, 100).await.unwrap();
        let second_id = Uuid::parse_str(second["payload"]["id"].as_str().unwrap()).unwrap();
        fake.mode.store(0, Ordering::SeqCst);
        repo.store_audit_remote(second_id, &vault).await.unwrap();
        sqlx::query("UPDATE audit_remote_exports SET verified_at=? WHERE archive_id=?")
            .bind(now() - 86400 * 40)
            .bind(second_id.to_string())
            .execute(&repo.pool)
            .await
            .unwrap();
        fake.mode.store(6, Ordering::SeqCst);
        let second_digest = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&second).unwrap())
        );
        let rejected = repo
            .prune_audit_remote(second_id, &vault, now(), &second_digest)
            .await
            .unwrap();
        assert_eq!(rejected["state"], "REJECTED");
        assert_eq!(
            repo.prune_audit_remote(second_id, &vault, now(), &second_digest)
                .await
                .unwrap(),
            rejected
        );
        assert_eq!(fake.deletes.load(Ordering::SeqCst), 2);
        assert_eq!(
            *fake.object.lock().unwrap(),
            Some(serde_json::to_vec(&second).unwrap())
        );
        task.abort();
    }
}
