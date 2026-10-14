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
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| RemoteError::Transport)? {
            if bytes.len() + chunk.len() > audit_archive::MAX_BYTES {
                return Err(RemoteError::Response);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(bytes))
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

impl Repository {
    pub async fn store_audit_remote(
        &self,
        id: Uuid,
        vault: &RemoteVault,
    ) -> Result<Value, RemoteError> {
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
            Some(bytes) => (StatusCode::OK, bytes).into_response(),
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
            .route("/vault/{id}", get(read).put(put))
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
        task.abort();
    }
}
