use super::*;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credential {
    pub token: String,
    pub identity: String,
    pub scopes: Vec<String>,
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
            || credential.identity.trim().is_empty()
            || credential.token.bytes().any(|b| !b.is_ascii_graphic())
            || !tokens.insert(&credential.token)
            || credential.scopes.iter().any(|scope| {
                !matches!(
                    scope.as_str(),
                    "jobs:read"
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
        if self.credentials.is_empty() {
            return (!self.token.is_empty() && equal(token.as_bytes(), self.token.as_bytes()))
                .then(|| ("legacy-local".into(), vec!["admin".into()]));
        }
        self.credentials
            .iter()
            .find(|c| equal(token.as_bytes(), c.token.as_bytes()))
            .map(|c| (c.identity.clone(), c.scopes.clone()))
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
