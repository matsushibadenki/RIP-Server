//! Explicit AWS KMS Ed25519 audit signer. No private key import/export or fallback.
use crate::{Repository, StorageError, audit_archive, receipts, s3};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Client, Url,
    header::{HeaderMap, HeaderValue},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
#[derive(Debug, thiserror::Error)]
pub enum KmsError {
    #[error("invalid_kms_configuration")]
    Configuration,
    #[error("kms_request_failed")]
    Transport,
    #[error("kms_response_rejected")]
    Response,
    #[error("kms_public_key_or_signature_mismatch")]
    Key,
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
pub struct KmsSigner {
    url: Url,
    client: Client,
    credentials: s3::Credentials,
    key: String,
    public: [u8; 32],
}
pub(crate) fn valid_key_arn(value: &str) -> bool {
    let parts = value.split(':').collect::<Vec<_>>();
    parts.len() == 6
        && parts[0] == "arn"
        && matches!(parts[1], "aws" | "aws-cn" | "aws-us-gov")
        && parts[2] == "kms"
        && !parts[3].is_empty()
        && parts[3]
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && parts[4].len() == 12
        && parts[4].bytes().all(|b| b.is_ascii_digit())
        && parts[5].strip_prefix("key/").is_some_and(|id| {
            uuid::Uuid::parse_str(id).is_ok_and(|id| format!("key/{id}") == parts[5])
        })
}
impl KmsSigner {
    pub fn new(
        endpoint: &str,
        region: &str,
        access: &str,
        secret: &str,
        token: Option<&str>,
        key: &str,
        pinned: &[u8],
    ) -> Result<Self, KmsError> {
        let url = Url::parse(endpoint).map_err(|_| KmsError::Configuration)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || !valid_key_arn(key)
            || key.split(':').nth(3) != Some(region)
            || pinned.len() != 32
        {
            return Err(KmsError::Configuration);
        }
        let client = Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| KmsError::Configuration)?;
        Ok(Self {
            url,
            client,
            credentials: s3::Credentials::new(region, access, secret, token)
                .map_err(|_| KmsError::Configuration)?,
            key: key.into(),
            public: pinned.try_into().map_err(|_| KmsError::Configuration)?,
        })
    }
    pub fn from_env() -> Result<Self, KmsError> {
        let env = |key| std::env::var(key).map_err(|_| KmsError::Configuration);
        let token = match std::env::var("AWS_SESSION_TOKEN") {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(_) => return Err(KmsError::Configuration),
        };
        use std::io::Read;
        let mut pinned = Vec::new();
        std::fs::File::open(env("JRIP_KMS_PUBLIC_KEY")?)
            .map_err(|_| KmsError::Configuration)?
            .take(33)
            .read_to_end(&mut pinned)
            .map_err(|_| KmsError::Configuration)?;
        Self::new(
            &env("JRIP_KMS_URL")?,
            &env("AWS_REGION")?,
            &env("AWS_ACCESS_KEY_ID")?,
            &env("AWS_SECRET_ACCESS_KEY")?,
            token.as_deref(),
            &env("JRIP_KMS_KEY_ID")?,
            &pinned,
        )
    }
    async fn request(&self, target: &'static str, value: Value) -> Result<Value, KmsError> {
        let body = serde_json::to_vec(&value)?;
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            HeaderValue::from_static("application/x-amz-json-1.1"),
        );
        headers.insert("x-amz-target", HeaderValue::from_static(target));
        let headers = self
            .credentials
            .sign_service("kms", "POST", &self.url, headers, &body, crate::now())
            .map_err(|_| KmsError::Configuration)?;
        let mut response = self
            .client
            .post(self.url.clone())
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|_| KmsError::Transport)?;
        if response.status().as_u16() != 200 {
            return Err(KmsError::Response);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| KmsError::Transport)? {
            if bytes.len() + chunk.len() > 65536 {
                return Err(KmsError::Response);
            }
            bytes.extend_from_slice(&chunk);
        }
        rip_digest::parse(&bytes).map_err(|_| KmsError::Response)
    }
    async fn sign_archive(&self, archive: &mut Value) -> Result<(), KmsError> {
        if archive.get("signature").is_some()
            || archive.get("proof").is_some()
            || !audit_archive::verify(archive)
        {
            return Err(KmsError::Key);
        }
        let key = self
            .request("TrentService.GetPublicKey", json!({"KeyId":self.key}))
            .await?;
        let der = key["PublicKey"]
            .as_str()
            .and_then(|v| STANDARD.decode(v).ok())
            .ok_or(KmsError::Key)?;
        let prefix = [
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ];
        if key["KeyId"] != self.key
            || key["KeySpec"] != "ECC_NIST_EDWARDS25519"
            || key["KeyUsage"] != "SIGN_VERIFY"
            || !key["SigningAlgorithms"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == "ED25519_SHA_512"))
            || der.len() != 44
            || der[..12] != prefix
            || der[12..] != self.public
        {
            return Err(KmsError::Key);
        }
        let mut signed = archive.clone();
        signed["proof"] = json!({"signed":true,"type":"ed25519-kms-record"});
        signed["signature"] = json!({"profile":receipts::KMS_AUDIT_PROFILE,"algorithm":"Ed25519","keyId":format!("sha256:{:x}",Sha256::digest(self.public)),
            "publicKey":receipts::hex(&self.public),"encoding":"hex","digestProfile":rip_digest::PROFILE,
            "messageEncoding":"domain-sha256-jcs","kmsKeyId":self.key});
        let message = receipts::message(&signed)?;
        let response=self.request("TrentService.Sign",json!({"KeyId":self.key,"MessageType":"RAW","SigningAlgorithm":"ED25519_SHA_512","Message":STANDARD.encode(message)})).await?;
        if response["KeyId"] != self.key || response["SigningAlgorithm"] != "ED25519_SHA_512" {
            return Err(KmsError::Key);
        }
        let signature = response["Signature"]
            .as_str()
            .and_then(|v| STANDARD.decode(v).ok())
            .filter(|v| v.len() == 64)
            .ok_or(KmsError::Key)?;
        signed["signature"]["value"] = json!(receipts::hex(&signature));
        if !audit_archive::verify_signature(&signed, &self.public) {
            return Err(KmsError::Key);
        }
        *archive = signed;
        Ok(())
    }
}
impl Repository {
    /// Network operations occur outside the SQLite writer; revocation is rechecked at publication.
    pub async fn archive_audit_kms(
        &self,
        signer: &KmsSigner,
        after: i64,
        before: i64,
        limit: i64,
    ) -> Result<Value, KmsError> {
        if self.receipt_signer.is_some() {
            return Err(KmsError::Configuration);
        }
        let key_id = format!("sha256:{:x}", Sha256::digest(signer.public));
        let revoked: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM receipt_key_revocations WHERE key_id=?)",
        )
        .bind(&key_id)
        .fetch_one(&self.pool)
        .await?;
        if revoked != 0 {
            return Err(StorageError::ReceiptKeyRevoked.into());
        }
        let mut read = self.pool.begin().await?;
        let mut archive = audit_archive::prepare_snapshot(&mut read, after, before, limit).await?;
        read.commit().await?;
        if serde_json::to_vec(&archive)?.len() > audit_archive::MAX_BYTES {
            return Err(StorageError::Conflict.into());
        }
        signer.sign_archive(&mut archive).await?;
        let bytes = serde_json::to_vec(&archive)?;
        if bytes.len() > audit_archive::MAX_BYTES {
            return Err(StorageError::Conflict.into());
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT OR IGNORE INTO receipt_public_keys(key_id,public_key,first_seen) VALUES(?,?,?)",
        )
        .bind(&key_id)
        .bind(receipts::hex(&signer.public))
        .bind(crate::now())
        .execute(&mut *tx)
        .await?;
        let revoked: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM receipt_key_revocations WHERE key_id=?)",
        )
        .bind(&key_id)
        .fetch_one(&mut *tx)
        .await?;
        if revoked != 0 {
            return Err(StorageError::ReceiptKeyRevoked.into());
        }
        let id = archive["payload"]["id"]
            .as_str()
            .ok_or(StorageError::Conflict)?;
        sqlx::query("INSERT INTO audit_archives(id,payload) VALUES(?,?)")
            .bind(id)
            .bind(String::from_utf8(bytes).map_err(|_| StorageError::Conflict)?)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO audit_records(correlation_id,identity,operation,target,started_at,finished_at,status) VALUES(?,?,?,?,?,?,200)")
            .bind(uuid::Uuid::new_v4().to_string()).bind("local-audit-kms").bind("audit-archive-create-kms").bind(id).bind(crate::now()).bind(crate::now()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(archive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router, body::Bytes, extract::State, http::StatusCode, response::IntoResponse,
        routing::post,
    };
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::sync::{
        Arc,
        atomic::{AtomicU8, AtomicUsize, Ordering},
    };
    const ARN: &str = "arn:aws:kms:us-east-1:123456789012:key/00000000-0000-0000-0000-000000000001";
    struct Fake {
        key: Ed25519KeyPair,
        mode: AtomicU8,
        signs: AtomicUsize,
        entered: tokio::sync::Notify,
        resume: tokio::sync::Notify,
    }
    async fn kms(
        State(fake): State<Arc<Fake>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> axum::response::Response {
        let auth = headers.get("authorization").unwrap().to_str().unwrap();
        assert!(auth.contains("/kms/aws4_request"));
        assert!(auth.contains("x-amz-target"));
        assert_eq!(
            headers["x-amz-content-sha256"].to_str().unwrap(),
            format!("{:x}", Sha256::digest(&body))
        );
        let request: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(request["KeyId"], ARN);
        let mode = fake.mode.load(Ordering::SeqCst);
        if mode == 7 {
            return (
                StatusCode::TEMPORARY_REDIRECT,
                [("location", "https://must-not-follow.invalid/")],
            )
                .into_response();
        }
        if mode == 6 {
            return (StatusCode::OK, "x".repeat(65537)).into_response();
        }
        if mode == 11 {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        if headers["x-amz-target"] == "TrentService.GetPublicKey" {
            let mut der = vec![
                0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
            ];
            der.extend_from_slice(fake.key.public_key().as_ref());
            if mode == 1 {
                der[12] ^= 1;
            }
            return axum::Json(json!({"KeyId":if mode==10 {"wrong-key"}else{ARN},"KeySpec":if mode==2 {"RSA_2048"}else{"ECC_NIST_EDWARDS25519"},
                "KeyUsage":"SIGN_VERIFY","SigningAlgorithms":["ED25519_SHA_512"],"PublicKey":STANDARD.encode(der)})).into_response();
        }
        assert_eq!(headers["x-amz-target"], "TrentService.Sign");
        fake.signs.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request["MessageType"], "RAW");
        assert_eq!(request["SigningAlgorithm"], "ED25519_SHA_512");
        let message = STANDARD
            .decode(request["Message"].as_str().unwrap())
            .unwrap();
        assert!(message.starts_with(receipts::KMS_AUDIT_PROFILE.as_bytes()));
        assert!(message.len() < 4096);
        if mode == 8 {
            fake.entered.notify_one();
            fake.resume.notified().await;
        }
        if mode == 5 {
            return StatusCode::BAD_REQUEST.into_response();
        }
        let signature = if mode == 4 {
            vec![0u8; 64]
        } else {
            fake.key.sign(&message).as_ref().to_vec()
        };
        axum::Json(json!({"KeyId":if mode==3 {"wrong-key"}else{ARN},"SigningAlgorithm":if mode==9 {"ED25519_PH_SHA_512"}else{"ED25519_SHA_512"},"Signature":STANDARD.encode(signature)})).into_response()
    }
    struct Server(tokio::task::JoinHandle<()>);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    async fn setup() -> (KmsSigner, Arc<Fake>, Server) {
        let key = Ed25519KeyPair::from_pkcs8(
            Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        let fake = Arc::new(Fake {
            key,
            mode: AtomicU8::new(0),
            signs: AtomicUsize::new(0),
            entered: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new().route("/", post(kms)).with_state(fake.clone());
        let server = Server(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }));
        let mut signer = KmsSigner::new(
            "https://test.invalid/",
            "us-east-1",
            "TESTACCESS",
            "test-secret",
            None,
            ARN,
            fake.key.public_key().as_ref(),
        )
        .unwrap();
        // Test transport only; the production constructor has no HTTP/TLS bypass.
        signer.url = Url::parse(&format!("http://{address}/")).unwrap();
        signer.client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        (signer, fake, server)
    }
    async fn repo(root: &std::path::Path) -> Repository {
        let repo = Repository::open_with_receipt_signer(&root.join("db"), None)
            .await
            .unwrap();
        let seq = repo
            .begin_audit(uuid::Uuid::new_v4(), "operator", &"a".repeat(10000), None)
            .await
            .unwrap();
        repo.finish_audit(seq, 200).await.unwrap();
        repo
    }
    #[tokio::test]
    async fn kms_large_archive_is_pinned_verified_durable_and_revocation_checked_at_commit() {
        let (signer, fake, _server) = setup().await;
        let signer = Arc::new(signer);
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(dir.path()).await;
        let archive = repo
            .archive_audit_kms(&signer, 0, crate::now() + 1, 100)
            .await
            .unwrap();
        let public = fake.key.public_key().as_ref();
        assert!(serde_json::to_vec(&archive).unwrap().len() > 4096);
        assert!(audit_archive::verify_signature(&archive, public));
        assert!(!receipts::verify(&archive, public));
        assert!(
            repo.verify_audit_archive_current(&archive, public)
                .await
                .unwrap()
        );
        for field in ["kmsKeyId", "messageEncoding", "profile", "publicKey"] {
            let mut tampered = archive.clone();
            tampered["signature"][field] = json!("tampered");
            assert!(!audit_archive::verify_signature(&tampered, public));
        }
        let id = archive["payload"]["id"].as_str().unwrap().parse().unwrap();
        let reopened = Repository::open_with_receipt_signer(&dir.path().join("db"), None)
            .await
            .unwrap();
        assert_eq!(reopened.audit_archive(id).await.unwrap(), archive);
        fake.mode.store(8, Ordering::SeqCst);
        let task_repo = repo.clone();
        let task_signer = signer.clone();
        let task = tokio::spawn(async move {
            task_repo
                .archive_audit_kms(&task_signer, 0, crate::now() + 1, 100)
                .await
        });
        tokio::time::timeout(Duration::from_secs(3), fake.entered.notified())
            .await
            .unwrap();
        let key_id = archive["signature"]["keyId"].as_str().unwrap();
        // This must not wait for the remote signing call or a held SQLite writer.
        tokio::time::timeout(
            Duration::from_secs(2),
            repo.revoke_receipt_key(key_id, "retired", "operator"),
        )
        .await
        .unwrap()
        .unwrap();
        fake.resume.notify_one();
        assert!(matches!(
            task.await.unwrap(),
            Err(KmsError::Storage(StorageError::ReceiptKeyRevoked))
        ));
        assert!(
            !repo
                .verify_audit_archive_current(&archive, public)
                .await
                .unwrap()
        );
        assert!(audit_archive::verify_signature(&archive, public));
        assert_eq!(
            repo.audit_archives("", crate::now() + 1, 100)
                .await
                .unwrap()["archives"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let count = fake.signs.load(Ordering::SeqCst);
        assert!(matches!(
            repo.archive_audit_kms(&signer, 0, crate::now() + 1, 100)
                .await,
            Err(KmsError::Storage(StorageError::ReceiptKeyRevoked))
        ));
        assert_eq!(fake.signs.load(Ordering::SeqCst), count);
    }
    #[tokio::test]
    async fn kms_bad_key_algorithm_signature_and_transport_never_publish_or_fallback() {
        let (signer, fake, _server) = setup().await;
        for mode in [1, 2, 3, 4, 5, 6, 7, 9, 10, 11] {
            fake.mode.store(mode, Ordering::SeqCst);
            let dir = tempfile::tempdir().unwrap();
            let repo = repo(dir.path()).await;
            assert!(
                repo.archive_audit_kms(&signer, 0, crate::now() + 1, 100)
                    .await
                    .is_err(),
                "mode {mode}"
            );
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_archives")
                .fetch_one(&repo.pool)
                .await
                .unwrap();
            assert_eq!(count, 0);
            assert!(
                repo.receipt_keys("", 100).await.unwrap()["keys"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }
    #[test]
    fn kms_configuration_requires_https_immutable_arn_region_and_pin() {
        for (url, key, region, pin) in [
            ("http://localhost/", ARN, "us-east-1", vec![0; 32]),
            (
                "https://example.org/",
                "alias/key",
                "us-east-1",
                vec![0; 32],
            ),
            ("https://example.org/", ARN, "us-west-1", vec![0; 32]),
            ("https://example.org/", ARN, "us-east-1", vec![0; 31]),
            ("https://u:p@example.org/", ARN, "us-east-1", vec![0; 32]),
        ] {
            assert!(KmsSigner::new(url, region, "TESTACCESS", "secret", None, key, &pin).is_err());
        }
    }
}
