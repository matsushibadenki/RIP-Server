//! HTTPS coordinator client. Failed/uncertain release conservatively retains quota.
use reqwest::{Client, Url, redirect::Policy};
use serde_json::{Value, json};
use std::{io, time::Duration};
use uuid::Uuid;
#[derive(Clone)]
pub struct SnapshotCoordinator {
    client: Client,
    endpoint: Url,
    token: String,
}
pub struct Reservation {
    coordinator: SnapshotCoordinator,
    pub id: Uuid,
    pub bytes: u64,
}
impl SnapshotCoordinator {
    pub fn from_env() -> io::Result<Option<Self>> {
        match (
            std::env::var("JRIP_SNAPSHOT_COORDINATOR_URL"),
            std::env::var("JRIP_SNAPSHOT_COORDINATOR_TOKEN"),
        ) {
            (Err(std::env::VarError::NotPresent), Err(std::env::VarError::NotPresent)) => Ok(None),
            (Ok(url), Ok(token)) => Self::new(&url, &token).map(Some),
            _ => Err(io::Error::other(
                "configure both snapshot coordinator URL and token",
            )),
        }
    }
    pub fn new(url: &str, token: &str) -> io::Result<Self> {
        let endpoint = Url::parse(url).map_err(|_| io::Error::other("invalid coordinator URL"))?;
        let loopback = endpoint
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|ip| ip.is_loopback());
        if !(endpoint.scheme() == "https" || endpoint.scheme() == "http" && loopback)
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !endpoint.path().ends_with('/')
            || token.is_empty()
            || token.len() > 4096
            || !token.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(io::Error::other(
                "invalid snapshot coordinator configuration",
            ));
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(io::Error::other)?;
        Ok(Self {
            client,
            endpoint,
            token: token.into(),
        })
    }
    async fn request(&self, path: &str, body: Value) -> io::Result<Value> {
        let mut response = self
            .client
            .post(self.endpoint.join(path).map_err(io::Error::other)?)
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|_| io::Error::other("snapshot coordinator unavailable"))?;
        let status = response.status();
        if !status.is_success() && status != reqwest::StatusCode::SERVICE_UNAVAILABLE {
            return Err(io::Error::other("snapshot coordinator rejected request"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| io::Error::other("coordinator read failed"))?
        {
            if bytes.len() + chunk.len() > 32768 {
                return Err(io::Error::other("coordinator response limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if !status.is_success() {
            return Err(io::Error::new(
                if value["error"]["code"] == "artifact_budget_exhausted" {
                    io::ErrorKind::WouldBlock
                } else {
                    io::ErrorKind::Other
                },
                "snapshot coordinator rejected request",
            ));
        }
        Ok(value)
    }
    pub async fn reserve(&self, bytes: u64) -> io::Result<Reservation> {
        if !(1..=super::budget::DEFAULT_CAPACITY).contains(&bytes) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid snapshot size",
            ));
        }
        let id = Uuid::new_v4();
        let rounded = bytes.div_ceil(super::budget::SLOT) * super::budget::SLOT;
        let response = self
            .request("reservations", json!({"requestId":id,"bytes":bytes}))
            .await
            .map_err(|e| io::Error::new(e.kind(), format!("snapshot reservation {id}: {e}")))?;
        if response["id"] != id.to_string()
            || response["bytes"] != rounded
            || response["state"] != "HELD"
        {
            return Err(io::Error::other(format!(
                "invalid snapshot reservation {id}"
            )));
        }
        Ok(Reservation {
            coordinator: self.clone(),
            id,
            bytes: rounded,
        })
    }
}
impl Reservation {
    pub async fn release(self) -> io::Result<()> {
        let response = self
            .coordinator
            .request(
                &format!("reservations/{}/release", self.id),
                json!({"expectedBytes":self.bytes}),
            )
            .await?;
        if response["id"] != self.id.to_string()
            || response["bytes"] != self.bytes
            || response["state"] != "RELEASED"
        {
            return Err(io::Error::other("invalid snapshot release response"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_rejects_insecure_redirectable_and_secret_bearing_urls() {
        for url in [
            "http://example.org/budget/",
            "http://localhost/budget/",
            "https://user:pass@example.org/budget/",
            "https://example.org/budget/?token=secret",
            "https://example.org/budget/#x",
            "file:///budget/",
            "https://example.org/budget",
        ] {
            assert!(SnapshotCoordinator::new(url, "token").is_err(), "{url}");
        }
        assert!(SnapshotCoordinator::new("https://example.org/budget/", "token").is_ok());
        assert!(SnapshotCoordinator::new("http://127.0.0.1:10000/budget/", "token").is_ok());
        assert!(SnapshotCoordinator::new("https://example.org/budget/", "bad token").is_err());
    }
    #[tokio::test]
    async fn lost_admission_ack_keeps_quota_and_redirects_are_not_followed() {
        use axum::{
            Router,
            http::{StatusCode, header},
            response::IntoResponse,
            routing::post,
        };
        let root = tempfile::tempdir().unwrap();
        let repo = crate::Repository::open(&root.path().join("db"))
            .await
            .unwrap();
        repo.configure_snapshot_budget(crate::budget::SLOT)
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let writer = repo.clone();
        let redirected = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let target = redirected.clone();
        let router = Router::new()
            .route(
                "/lost/reservations",
                post(move |axum::Json(body): axum::Json<Value>| {
                    let repo = writer.clone();
                    async move {
                        repo.reserve_snapshot(
                            body["requestId"].as_str().unwrap().parse().unwrap(),
                            "node",
                            1,
                        )
                        .await
                        .unwrap();
                        StatusCode::SERVICE_UNAVAILABLE
                    }
                }),
            )
            .route(
                "/redirect/reservations",
                post(|| async {
                    (
                        StatusCode::TEMPORARY_REDIRECT,
                        [(header::LOCATION, "/target")],
                    )
                        .into_response()
                }),
            )
            .route("/target",post(move |axum::Json(body):axum::Json<Value>| {
                target.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
                async move {axum::Json(json!({"id":body["requestId"],"bytes":crate::budget::SLOT,"state":"HELD"}))}
            }))
            .route(
                "/oversize/reservations",
                post(|| async { "x".repeat(32769) }),
            );
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let lost = SnapshotCoordinator::new(&format!("http://{address}/lost/"), "token").unwrap();
        assert!(lost.reserve(1).await.is_err());
        assert_eq!(
            repo.snapshot_budget_status(None, 100).await.unwrap()["heldBytes"],
            crate::budget::SLOT
        );
        for path in ["redirect", "oversize"] {
            let coordinator =
                SnapshotCoordinator::new(&format!("http://{address}/{path}/"), "token").unwrap();
            let error = coordinator.reserve(1).await.err().unwrap();
            if path == "oversize" {
                assert!(error.to_string().contains("response limit"));
            }
        }
        assert_eq!(redirected.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(lost.reserve(u64::MAX).await.is_err());
        server.abort();
    }
}
