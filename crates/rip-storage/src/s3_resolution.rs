//! Reconciliation never issues DELETE; pinned version reads do not target replacements.
use crate::{
    Repository,
    audit_remote::{RemoteError, RemoteVault},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
impl RemoteVault {
    pub(crate) async fn s3_observe(&self, id: Uuid, version: Option<&str>) -> Value {
        match self.s3_observe_inner(id, version).await {
            Ok(value) => value,
            Err(_) => json!({"state":"UNAVAILABLE"}),
        }
    }
    async fn s3_observe_inner(
        &self,
        id: Uuid,
        version: Option<&str>,
    ) -> Result<Value, RemoteError> {
        let mut url = self
            .base
            .join(&format!("{id}.json"))
            .map_err(|_| RemoteError::Configuration)?;
        if let Some(version) = version {
            if !crate::s3::valid_version(version) {
                return Err(RemoteError::Configuration);
            }
            url.set_query(Some(&format!("versionId={}", crate::s3::encode(version))));
        }
        let credentials = self.s3.as_ref().ok_or(RemoteError::Configuration)?;
        let mut request_headers = reqwest::header::HeaderMap::new();
        request_headers.insert(
            "cache-control",
            reqwest::header::HeaderValue::from_static("no-cache"),
        );
        let headers = credentials.sign("GET", &url, request_headers, b"", crate::now())?;
        let mut response = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|_| RemoteError::Transport)?;
        let code = response.status().as_u16();
        let actual = response
            .headers()
            .get("x-amz-version-id")
            .and_then(|v| v.to_str().ok())
            .filter(|v| crate::s3::valid_version(v))
            .map(str::to_owned);
        let marker = response
            .headers()
            .get("x-amz-delete-marker")
            .is_some_and(|v| v == "true");
        let state = match (code, marker) {
            (200, false) => "PRESENT",
            (404, false) => "ABSENT",
            (404 | 405, true) => "DELETE_MARKER",
            _ => "UNAVAILABLE",
        };
        let mut observation = json!({"state":state,"httpStatus":code,"versionId":actual});
        if state == "PRESENT" {
            if version.is_some_and(|v| actual.as_deref() != Some(v)) {
                return Err(RemoteError::Response);
            }
            if response
                .headers()
                .get("content-encoding")
                .is_some_and(|v| v != "identity")
            {
                return Err(RemoteError::Response);
            }
            let mut bytes = 0usize;
            let mut hash = Sha256::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| RemoteError::Transport)? {
                bytes = bytes
                    .checked_add(chunk.len())
                    .filter(|n| *n <= crate::audit_archive::MAX_BYTES)
                    .ok_or(RemoteError::Response)?;
                hash.update(&chunk);
            }
            observation["fileDigest"] = json!(format!("sha256:{:x}", hash.finalize()));
            observation["bytes"] = json!(bytes);
        }
        Ok(observation)
    }
}
impl Repository {
    pub async fn s3_prune_evidence(
        &self,
        id: Uuid,
        vault: &RemoteVault,
    ) -> Result<Value, RemoteError> {
        if vault.s3.is_none() {
            return Err(RemoteError::Configuration);
        }
        let record = self.remote_prune_record(id, vault).await?;
        if !matches!(
            record["profile"].as_str(),
            Some("org.jrip.audit-remote-prune-v1" | "org.jrip.audit-remote-prune-v2")
        ) {
            return Err(RemoteError::Conflict);
        }

        let current = vault.s3_observe(id, None).await;
        let version = record["sourceVersionId"].as_str().filter(|v| *v != "null");
        let mut source = Value::Null;
        let (state, reason, meaning) = if let Some(version) = version {
            source = vault.s3_observe(id, Some(version)).await;
            match source["state"].as_str() {
                Some("PRESENT")
                    if source["fileDigest"] == record["fileDigest"]
                        && source["bytes"] == record["bytes"] =>
                {
                    (
                        "CONFIRMED_RETAINED",
                        "SOURCE_VERSION_VERIFIED",
                        "verified-source-version-retained",
                    )
                }
                Some("ABSENT") => (
                    "CONFIRMED_ABSENT",
                    "SOURCE_VERSION_ABSENT",
                    "source-version-observed-absent",
                ),
                Some("PRESENT") => ("UNKNOWN", "SOURCE_VERSION_CONTENT_MISMATCH", "unresolved"),
                _ => ("UNKNOWN", "SOURCE_VERSION_UNAVAILABLE", "unresolved"),
            }
        } else {
            match current["state"].as_str() {
                Some("ABSENT") => ("CONFIRMED_ABSENT", "CURRENT_KEY_ABSENT", "observed-absent"),
                Some("DELETE_MARKER") => ("UNKNOWN", "UNPINNED_RETAINED_VERSIONS", "unresolved"),
                Some("PRESENT") => ("UNKNOWN", "CURRENT_COPY_PRESENT", "unresolved"),
                _ => ("UNKNOWN", "CURRENT_KEY_UNAVAILABLE", "unresolved"),
            }
        };
        Ok(
            json!({"schemaVersion":1,"archiveId":id,"resolution":state,"reasonCode":reason,"confirmationMeaning":meaning,
            "current":current,"sourceVersion":source,"sourceVersionId":record["sourceVersionId"],"observedAt":crate::now(),"deleteResent":false,"physicalPurgeProven":false}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Bytes,
        extract::{OriginalUri, Query, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };
    struct Fake {
        bytes: Mutex<Option<Vec<u8>>>,
        deleted: AtomicBool,
        deletes: AtomicUsize,
        mode: u8,
        queries: Mutex<Vec<String>>,
    }
    async fn put(State(fake): State<Arc<Fake>>, headers: HeaderMap, body: Bytes) -> StatusCode {
        assert_eq!(headers["if-none-match"], "*");
        assert!(
            headers["authorization"]
                .to_str()
                .unwrap()
                .starts_with("AWS4-HMAC-SHA256")
        );
        let mut slot = fake.bytes.lock().unwrap();
        if slot.is_some() {
            return StatusCode::PRECONDITION_FAILED;
        }
        *slot = Some(body.to_vec());
        StatusCode::OK
    }
    async fn delete(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> StatusCode {
        assert_eq!(headers["if-match"], "\"source\"");
        fake.deletes.fetch_add(1, Ordering::SeqCst);
        fake.deleted.store(true, Ordering::SeqCst);
        // The service applied the current-key delete but the ACK was lost.
        StatusCode::INTERNAL_SERVER_ERROR
    }
    async fn read(
        State(fake): State<Arc<Fake>>,
        OriginalUri(uri): OriginalUri,
        Query(query): Query<std::collections::HashMap<String, String>>,
    ) -> axum::response::Response {
        if let Some(version) = query.get("versionId") {
            assert_eq!(version, "v+/=");
            fake.queries.lock().unwrap().push(uri.to_string());
            match fake.mode {
                2 | 8 => return StatusCode::NOT_FOUND.into_response(),
                3 => return StatusCode::FORBIDDEN.into_response(),
                7 => {
                    return (
                        StatusCode::METHOD_NOT_ALLOWED,
                        [
                            ("x-amz-delete-marker", "true"),
                            ("x-amz-version-id", "v+/="),
                        ],
                    )
                        .into_response();
                }
                9 => {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                _ => {}
            }
            let bytes = if fake.mode == 5 {
                b"wrong archive bytes".to_vec()
            } else {
                fake.bytes.lock().unwrap().clone().unwrap()
            };
            return (
                StatusCode::OK,
                [(
                    "x-amz-version-id",
                    if fake.mode == 4 {
                        "wrong-version"
                    } else {
                        "v+/="
                    },
                )],
                bytes,
            )
                .into_response();
        }
        if fake.deleted.load(Ordering::SeqCst) {
            if fake.mode == 8 {
                return (
                    StatusCode::OK,
                    [("x-amz-version-id", "replacement")],
                    b"replacement object".to_vec(),
                )
                    .into_response();
            }
            return (
                StatusCode::NOT_FOUND,
                [
                    ("x-amz-delete-marker", "true"),
                    ("x-amz-version-id", "marker-1"),
                ],
            )
                .into_response();
        }
        let bytes = fake.bytes.lock().unwrap().clone().unwrap();
        if fake.mode == 6 {
            return (StatusCode::OK, [("etag", "\"source\"")], bytes).into_response();
        }
        (
            StatusCode::OK,
            [
                ("etag", "\"source\""),
                (
                    "x-amz-version-id",
                    if fake.mode == 10 { "null" } else { "v+/=" },
                ),
            ],
            bytes,
        )
            .into_response()
    }
    struct Server(tokio::task::JoinHandle<()>);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    #[tokio::test]
    async fn pinned_versions_resolve_unknown_without_deleting_replacements_or_markers() {
        for (mode, state, reason) in [
            (0, "CONFIRMED_RETAINED", "SOURCE_VERSION_VERIFIED"),
            (2, "CONFIRMED_ABSENT", "SOURCE_VERSION_ABSENT"),
            (3, "UNKNOWN", "SOURCE_VERSION_UNAVAILABLE"),
            (4, "UNKNOWN", "SOURCE_VERSION_UNAVAILABLE"),
            (5, "UNKNOWN", "SOURCE_VERSION_CONTENT_MISMATCH"),
            (6, "UNKNOWN", "UNPINNED_RETAINED_VERSIONS"),
            (7, "UNKNOWN", "SOURCE_VERSION_UNAVAILABLE"),
            (8, "CONFIRMED_ABSENT", "SOURCE_VERSION_ABSENT"),
            (9, "UNKNOWN", "SOURCE_VERSION_UNAVAILABLE"),
            (10, "UNKNOWN", "UNPINNED_RETAINED_VERSIONS"),
        ] {
            let fake = Arc::new(Fake {
                bytes: Mutex::new(None),
                deleted: AtomicBool::new(false),
                deletes: AtomicUsize::new(0),
                mode,
                queries: Mutex::new(Vec::new()),
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let router = Router::new()
                .route("/vault/{id}", get(read).put(put).delete(delete))
                .with_state(fake.clone());
            let _server = Server(tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            }));
            let mut vault = RemoteVault::s3(
                "https://test.invalid/vault/",
                "us-east-1",
                "TESTACCESS",
                "test-secret",
                None,
            )
            .unwrap();
            vault.base = reqwest::Url::parse(&format!("http://{address}/vault/")).unwrap();
            vault.client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(Duration::from_millis(500))
                .build()
                .unwrap();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("db");
            let repo = Repository::open_with_receipt_signer(&path, None)
                .await
                .unwrap();
            let seq = repo
                .begin_audit(Uuid::new_v4(), "operator", "test", None)
                .await
                .unwrap();
            repo.finish_audit(seq, 200).await.unwrap();
            let archive = repo.archive_audit(0, crate::now() + 1, 100).await.unwrap();
            let id = archive["payload"]["id"].as_str().unwrap().parse().unwrap();
            let ack = repo.store_audit_remote(id, &vault).await.unwrap();
            sqlx::query("UPDATE audit_remote_exports SET verified_at=0")
                .execute(&repo.pool)
                .await
                .unwrap();
            let prepared = repo
                .prune_audit_remote(
                    id,
                    &vault,
                    crate::now(),
                    ack["fileDigest"].as_str().unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(prepared["state"], "UNKNOWN");
            let evidence = repo.s3_prune_evidence(id, &vault).await.unwrap();
            assert_eq!(evidence["resolution"], state, "mode {mode}");
            assert_eq!(evidence["reasonCode"], reason);
            assert_eq!(
                repo.remote_prune_record(id, &vault).await.unwrap()["state"],
                "UNKNOWN"
            );
            let reopened = Repository::open_with_receipt_signer(&path, None)
                .await
                .unwrap();
            let resolved = reopened.reconcile_remote_prune(id, &vault).await.unwrap();
            assert_eq!(resolved["state"], state, "mode {mode}");
            assert_eq!(resolved["localSnapshotProtected"], state == "UNKNOWN");
            assert_eq!(resolved["providerResolution"]["physicalPurgeProven"], false);
            if mode != 6 && mode != 10 {
                assert!(
                    fake.queries
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|q| q.ends_with("?versionId=v%2B%2F%3D"))
                );
            }
            let replay = reopened
                .prune_audit_remote(
                    id,
                    &vault,
                    crate::now(),
                    ack["fileDigest"].as_str().unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(replay, resolved);
            assert_eq!(fake.deletes.load(Ordering::SeqCst), 1);
            assert!(reopened.store_audit_remote(id, &vault).await.is_err());
            if state == "UNKNOWN" {
                let protected:i64=sqlx::query_scalar("SELECT COUNT(*) FROM audit_remote_prunes WHERE state IN ('UNKNOWN','PREPARED')").fetch_one(&repo.pool).await.unwrap();
                assert_eq!(protected, 1);
            }
        }
    }
}
