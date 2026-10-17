//! Restricted AWS SigV4 for S3/KMS. Only a bounded S3 versionId query is supported.
use crate::audit_remote::RemoteError;
use reqwest::{
    Url,
    header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue},
};
use ring::hmac;
use sha2::{Digest, Sha256};
pub(crate) fn valid_version(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && value.bytes().all(|b| b.is_ascii_graphic())
}
pub(crate) fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
pub struct Credentials {
    region: String,
    access: String,
    secret: String,
    token: Option<String>,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn mac(key: &[u8], data: &str) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data.as_bytes())
        .as_ref()
        .to_vec()
}
impl Credentials {
    pub fn new(
        region: &str,
        access: &str,
        secret: &str,
        token: Option<&str>,
    ) -> Result<Self, RemoteError> {
        if region.is_empty()
            || region.len() > 64
            || !region
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            || access.is_empty()
            || access.len() > 128
            || !access.bytes().all(|c| c.is_ascii_alphanumeric())
            || secret.is_empty()
            || secret.len() > 256
            || !secret.bytes().all(|c| c.is_ascii_graphic())
            || token.is_some_and(|t| {
                t.is_empty() || t.len() > 8192 || !t.bytes().all(|c| c.is_ascii_graphic())
            })
        {
            return Err(RemoteError::Configuration);
        }
        Ok(Self {
            region: region.into(),
            access: access.into(),
            secret: secret.into(),
            token: token.map(str::to_owned),
        })
    }
    pub(crate) fn sign(
        &self,
        method: &str,
        url: &Url,
        headers: HeaderMap,
        body: &[u8],
        seconds: i64,
    ) -> Result<HeaderMap, RemoteError> {
        self.sign_service("s3", method, url, headers, body, seconds)
    }
    pub(crate) fn sign_service(
        &self,
        service: &str,
        method: &str,
        url: &Url,
        mut headers: HeaderMap,
        body: &[u8],
        seconds: i64,
    ) -> Result<HeaderMap, RemoteError> {
        if !matches!(service, "s3" | "kms") {
            return Err(RemoteError::Configuration);
        }
        // Reject URL forms requiring S3-specific path normalization/encoding policy.
        if !url
            .path()
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-_.~".contains(&b))
        {
            return Err(RemoteError::Configuration);
        }
        let query = if let Some(query) = url.query() {
            if service != "s3" {
                return Err(RemoteError::Configuration);
            }
            let pairs = url.query_pairs().collect::<Vec<_>>();
            if pairs.len() != 1 || pairs[0].0 != "versionId" || !valid_version(&pairs[0].1) {
                return Err(RemoteError::Configuration);
            }
            let canonical = format!("versionId={}", encode(&pairs[0].1));
            if query != canonical {
                return Err(RemoteError::Configuration);
            }
            canonical
        } else {
            String::new()
        };
        let instant = time::OffsetDateTime::from_unix_timestamp(seconds)
            .map_err(|_| RemoteError::Configuration)?;
        if !(1970..=9999).contains(&instant.year()) {
            return Err(RemoteError::Configuration);
        }
        let date = format!(
            "{:04}{:02}{:02}",
            instant.year(),
            instant.month() as u8,
            instant.day()
        );
        let stamp = format!(
            "{date}T{:02}{:02}{:02}Z",
            instant.hour(),
            instant.minute(),
            instant.second()
        );
        let host = match url.port() {
            Some(port) => format!(
                "{}:{port}",
                url.host_str().ok_or(RemoteError::Configuration)?
            ),
            None => url
                .host_str()
                .ok_or(RemoteError::Configuration)?
                .to_string(),
        };
        let digest = hex(&Sha256::digest(body));
        for (key, value) in [
            ("host", host),
            ("x-amz-date", stamp.clone()),
            ("x-amz-content-sha256", digest.clone()),
        ] {
            headers.insert(
                HeaderName::from_static(key),
                HeaderValue::from_str(&value).map_err(|_| RemoteError::Configuration)?,
            );
        }
        if let Some(token) = &self.token {
            let mut value = HeaderValue::from_str(token).map_err(|_| RemoteError::Configuration)?;
            value.set_sensitive(true);
            headers.insert("x-amz-security-token", value);
        }
        let mut sorted = std::collections::BTreeMap::new();
        for (name, value) in &headers {
            if name == AUTHORIZATION {
                return Err(RemoteError::Configuration);
            }
            let value = value
                .to_str()
                .map_err(|_| RemoteError::Configuration)?
                .split_ascii_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            sorted.insert(name.as_str(), value);
        }
        let canonical_headers = sorted
            .iter()
            .map(|(k, v)| format!("{k}:{v}\n"))
            .collect::<String>();
        let signed = sorted.keys().copied().collect::<Vec<_>>().join(";");
        let canonical = format!(
            "{method}\n{}\n{query}\n{canonical_headers}\n{signed}\n{digest}",
            url.path()
        );
        let scope = format!("{date}/{}/{service}/aws4_request", self.region);
        let to_sign = format!(
            "AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{}",
            hex(&Sha256::digest(canonical.as_bytes()))
        );
        let key = mac(format!("AWS4{}", self.secret).as_bytes(), &date);
        let key = mac(&key, &self.region);
        let key = mac(&key, service);
        let key = mac(&key, "aws4_request");
        let signature = hex(&mac(&key, &to_sign));
        let mut authorization = HeaderValue::from_str(&format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
            self.access
        ))
        .map_err(|_| RemoteError::Configuration)?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
        Ok(headers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_match_independent_botocore_vectors() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/corpus/s3-sigv4-botocore.json"))
                .unwrap();
        for case in cases.as_array().unwrap() {
            let credentials = Credentials::new(
                "us-east-1",
                "TESTACCESS",
                "test-secret-not-a-real-credential",
                case["token"].as_str(),
            )
            .unwrap();
            let headers = case["headers"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| {
                    (
                        HeaderName::from_bytes(k.as_bytes()).unwrap(),
                        HeaderValue::from_str(v.as_str().unwrap()).unwrap(),
                    )
                })
                .collect();
            let signed = credentials
                .sign_service(
                    case["service"].as_str().unwrap_or("s3"),
                    case["method"].as_str().unwrap(),
                    &Url::parse(case["url"].as_str().unwrap()).unwrap(),
                    headers,
                    case["body"].as_str().unwrap().as_bytes(),
                    case["seconds"].as_i64().unwrap(),
                )
                .unwrap();
            assert_eq!(
                signed[AUTHORIZATION],
                case["authorization"].as_str().unwrap()
            );
            assert_eq!(signed["x-amz-date"], case["date"].as_str().unwrap());
            assert_eq!(
                signed["x-amz-content-sha256"],
                case["payloadHash"].as_str().unwrap()
            );
            assert!(signed[AUTHORIZATION].is_sensitive());
            if case["token"].is_string() {
                assert!(signed["x-amz-security-token"].is_sensitive());
            }
        }
    }
    #[test]
    fn credentials_and_unsupported_paths_fail_closed() {
        assert!(Credentials::new("us/east", "access", "secret", None).is_err());
        assert!(Credentials::new("us-east-1", "access/key", "secret", None).is_err());
        assert!(Credentials::new("us-east-1", "access", "secret", Some("bad\nvalue")).is_err());
        let credentials = Credentials::new("us-east-1", "access", "secret", None).unwrap();
        for path in [
            "https://s3.example.org/a%20b/",
            "https://s3.example.org/bucket/?uploadId=x",
        ] {
            assert!(
                credentials
                    .sign("GET", &Url::parse(path).unwrap(), HeaderMap::new(), b"", 0)
                    .is_err()
            );
        }
    }
}
