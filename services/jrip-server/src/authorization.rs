use super::*;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credential {
    pub token: String,
    pub identity: String,
    pub scopes: Vec<String>,
    #[serde(default)]
    pub validity: Option<CredentialValidity>,
}
/// A configured short-lived credential, evaluated in UTC epoch seconds.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialValidity {
    pub not_before: i64,
    pub expires_at: i64,
}
impl CredentialValidity {
    fn valid_window(&self) -> bool {
        self.not_before >= 0
            && self
                .expires_at
                .checked_sub(self.not_before)
                .is_some_and(|ttl| (1..=900).contains(&ttl))
    }
    fn active(&self, now: i64) -> bool {
        self.valid_window() && self.not_before <= now && now < self.expires_at
    }
}
pub fn load() -> Result<Vec<Credential>, Box<dyn std::error::Error>> {
    let Ok(path) = std::env::var("JRIP_AUTH_POLICY") else {
        return Ok(vec![]);
    };
    parse(&std::fs::read(path)?)
}
fn parse(bytes: &[u8]) -> Result<Vec<Credential>, Box<dyn std::error::Error>> {
    let credentials: Vec<Credential> = serde_json::from_slice(bytes)?;
    if credentials.is_empty() {
        return Err("Authorization policy must contain credentials".into());
    }
    let mut tokens = std::collections::HashSet::new();
    for credential in &credentials {
        if credential.token.len() < 24
            || credential
                .validity
                .as_ref()
                .is_some_and(|v| !v.valid_window())
            || credential.identity.trim().is_empty()
            || credential.token.bytes().any(|b| !b.is_ascii_graphic())
            || !tokens.insert(&credential.token)
            || credential.scopes.iter().any(|scope| {
                !matches!(
                    scope.as_str(),
                    "budgets:reserve"
                        | "jobs:read"
                        | "jobs:submit"
                        | "jobs:control"
                        | "artifacts:read"
                        | "printers:print"
                        | "settings:write"
                        | "admin"
                )
            })
        {
            return Err("Invalid authorization policy".into());
        }
    }
    Ok(credentials)
}
fn equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |n, (a, b)| n | (a ^ b)) == 0
}
impl App {
    pub fn access(&self, token: &str) -> Option<(String, Vec<String>)> {
        self.access_with_expiry(token)
            .map(|(identity, scopes, _)| (identity, scopes))
    }
    pub fn access_with_expiry(&self, token: &str) -> Option<(String, Vec<String>, Option<i64>)> {
        self.access_at(token, rip_storage::now())
    }
    fn access_at(&self, token: &str, now: i64) -> Option<(String, Vec<String>, Option<i64>)> {
        if self.credentials.is_empty() {
            return (!self.token.is_empty() && equal(token.as_bytes(), self.token.as_bytes()))
                .then(|| ("legacy-local".into(), vec!["admin".into()], None));
        }
        self.credentials
            .iter()
            .find(|c| equal(token.as_bytes(), c.token.as_bytes()))
            .filter(|c| c.validity.as_ref().is_none_or(|v| v.active(now)))
            .map(|c| {
                (
                    c.identity.clone(),
                    c.scopes.clone(),
                    c.validity.as_ref().map(|v| v.expires_at),
                )
            })
    }
}
pub(super) fn authorize(app: &App, headers: &HeaderMap, scope: &str) -> Result<(), ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    let (identity, scopes) = app
        .access(token)
        .ok_or(ApiError(StatusCode::UNAUTHORIZED, "unauthorized"))?;
    let allowed = scopes.iter().any(|s| s == scope || s == "admin");
    tracing::info!(%identity, %scope, allowed, "API authorization");
    if allowed {
        Ok(())
    } else {
        Err(ApiError(StatusCode::FORBIDDEN, "forbidden"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_validation_rejects_invalid_and_duplicate_credentials() {
        let valid = serde_json::json!({"identity":"reader","token":"reader-token-at-least-24-characters","scopes":["jobs:read"]});
        assert!(
            parse(
                serde_json::to_string(&vec![valid.clone()])
                    .unwrap()
                    .as_bytes()
            )
            .is_ok()
        );
        assert!(parse(b"[]").is_err());
        assert!(
            parse(
                serde_json::to_string(&vec![valid.clone(), valid.clone()])
                    .unwrap()
                    .as_bytes()
            )
            .is_err()
        );
        for (key, value) in [
            ("token", serde_json::json!("short")),
            (
                "token",
                serde_json::json!("a long token with invalid spaces"),
            ),
            ("identity", serde_json::json!(" ")),
            ("scopes", serde_json::json!(["unknown"])),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert!(parse(serde_json::to_string(&vec![invalid]).unwrap().as_bytes()).is_err());
        }
    }
}

#[cfg(test)]
mod validity_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn policy_rejects_partial_invalid_and_overlong_windows() {
        let mut credential = json!({"token":"short-lived-token-at-least-24-characters","identity":"agent","scopes":["jobs:read"]});
        for invalid in [
            json!({"notBefore":100}),
            json!({"expiresAt":200}),
            json!({"notBefore":-1,"expiresAt":100}),
            json!({"notBefore":100,"expiresAt":100}),
            json!({"notBefore":200,"expiresAt":100}),
            json!({"notBefore":100,"expiresAt":1001}),
            json!({"notBefore":true,"expiresAt":200}),
            json!({"notBefore":100,"expiresAt":"200"}),
            json!({"notBefore":100,"expiresAt":200,"extra":true}),
        ] {
            credential["validity"] = invalid;
            assert!(parse(&serde_json::to_vec(&json!([credential])).unwrap()).is_err());
        }
        credential["validity"] = json!({"notBefore":100,"expiresAt":1000});
        assert!(parse(&serde_json::to_vec(&json!([credential])).unwrap()).is_ok());
    }
    #[tokio::test]
    async fn access_checks_boundaries_each_time_without_legacy_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let token = "short-lived-token-at-least-24-characters";
        let mut app = open(dir.path().into(), token.into()).await.unwrap();
        app.credentials=parse(&serde_json::to_vec(&json!([
            {"token":token,"identity":"agent","scopes":["jobs:read"],"validity":{"notBefore":100,"expiresAt":1000}},
            {"token":"legacy-token-at-least-24-characters","identity":"operator","scopes":["jobs:control"]}
        ])).unwrap()).unwrap();
        assert!(app.access_at(token, 99).is_none());
        let (identity, scopes, expiry) = app.access_at(token, 100).unwrap();
        assert_eq!(identity, "agent");
        assert_eq!(scopes, vec!["jobs:read"]);
        assert_eq!(expiry, Some(1000));
        assert!(app.access_at(token, 999).is_some());
        assert!(app.access_at(token, 1000).is_none());
        assert!(app.access_at(token, 1001).is_none());
        assert!(app.access_at("unknown", 200).is_none());
        assert_eq!(
            app.access_at("legacy-token-at-least-24-characters", 1001)
                .unwrap()
                .2,
            None
        );
        app.credentials[0].validity = Some(CredentialValidity {
            not_before: 0,
            expires_at: i64::MAX,
        });
        assert!(app.access_at(token, 200).is_none());
    }
}
