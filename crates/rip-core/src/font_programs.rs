//! Declared embedded-program identities; not actual raster font selection proof.
use crate::{ValidationError, print::valid_digest};
use serde::Deserialize;
use serde_json::Value;

pub const PROFILE: &str = "org.jrip.embedded-font-program-v1";
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Identity {
    profile: String,
    status: String,
    encoded_sha256: String,
    encoded_bytes: u64,
    field: String,
    sha256: Option<String>,
    bytes: Option<u64>,
    reason: Option<String>,
}

pub fn validate(policy: &Value) -> Result<(), ValidationError> {
    let invalid = || ValidationError("invalid_font_program_identity");
    if policy["programIdentityProfile"] != PROFILE {
        return Err(invalid());
    }
    let fonts = policy["fonts"]
        .as_array()
        .filter(|fonts| fonts.len() <= 128)
        .ok_or_else(invalid)?;
    let mut programs = std::collections::HashSet::new();
    let mut bytes = 0u64;
    for font in fonts {
        if !matches!(
            font["state"].as_str(),
            Some("EMBEDDED" | "SELF_CONTAINED_TYPE3" | "STANDARD14_SUBSTITUTION")
        ) {
            return Err(invalid());
        }
        if font["state"] != "EMBEDDED" {
            if font.get("programIdentity").is_some() {
                return Err(invalid());
            }
            continue;
        }
        let value: Identity =
            serde_json::from_value(font["programIdentity"].clone()).map_err(|_| invalid())?;
        if value.profile != PROFILE
            || !valid_digest(&value.encoded_sha256)
            || !(1..=32 * 1024 * 1024).contains(&value.encoded_bytes)
            || !matches!(value.field.as_str(), "FontFile" | "FontFile2" | "FontFile3")
        {
            return Err(invalid());
        }
        match value.status.as_str() {
            "HASHED" => {
                let digest = value
                    .sha256
                    .as_deref()
                    .filter(|hash| valid_digest(hash))
                    .ok_or_else(invalid)?;
                let size = value
                    .bytes
                    .filter(|bytes| (1..=16 * 1024 * 1024).contains(bytes))
                    .ok_or_else(invalid)?;
                if value.reason.is_some() {
                    return Err(invalid());
                }
                if programs.insert((value.encoded_sha256, digest.to_owned())) {
                    bytes += size;
                    if bytes > 64 * 1024 * 1024 {
                        return Err(invalid());
                    }
                }
            }
            "UNAVAILABLE" => {
                if value.sha256.is_some()
                    || value.bytes.is_some()
                    || !matches!(
                        value.reason.as_deref(),
                        Some(
                            "TOTAL_BYTE_LIMIT"
                                | "PROGRAM_BYTE_LIMIT"
                                | "FILTER_CHAIN_LIMIT"
                                | "UNSUPPORTED_FILTER"
                                | "UNSUPPORTED_PARAMETERS"
                                | "EMPTY_PROGRAM"
                                | "DECODE_LIMIT"
                                | "DECODE_FAILED"
                        )
                    )
                {
                    return Err(invalid());
                }
            }
            _ => return Err(invalid()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn policy() -> Value {
        json!({"programIdentityProfile":PROFILE,"fonts":[{"state":"EMBEDDED","programIdentity":{"profile":PROFILE,"status":"HASHED","field":"FontFile2","encodedSha256":"a".repeat(64),"encodedBytes":100,"sha256":"b".repeat(64),"bytes":200}}]})
    }
    #[test]
    fn rejects_forged_or_ambiguous_identity_metadata() {
        let p = policy();
        validate(&p).unwrap();
        for (field, value) in [
            ("sha256", json!("bad")),
            ("encodedBytes", json!(0)),
            ("bytes", json!(17 * 1024 * 1024)),
            ("status", json!("COMPLETE")),
            ("reason", json!("DECODE_FAILED")),
            ("field", json!("CharProcs")),
            ("text", json!("not allowed")),
        ] {
            let mut bad = p.clone();
            bad["fonts"][0]["programIdentity"][field] = value;
            assert!(validate(&bad).is_err());
        }
        let mut unavailable = p;
        let identity = unavailable["fonts"][0]["programIdentity"]
            .as_object_mut()
            .unwrap();
        identity.remove("sha256");
        identity.remove("bytes");
        identity.insert("status".into(), json!("UNAVAILABLE"));
        identity.insert("reason".into(), json!("DECODE_LIMIT"));
        validate(&unavailable).unwrap();
        unavailable["fonts"][0]["state"] = json!("STANDARD14_SUBSTITUTION");
        assert!(validate(&unavailable).is_err());
    }
    #[test]
    fn shared_streams_do_not_double_charge_but_distinct_programs_obey_total_limit() {
        let mut p = policy();
        p["fonts"][0]["programIdentity"]["bytes"] = json!(16 * 1024 * 1024);
        let font = p["fonts"][0].clone();
        p["fonts"] = json!(vec![font; 5]);
        validate(&p).unwrap();
        for (i, font) in p["fonts"].as_array_mut().unwrap().iter_mut().enumerate() {
            font["programIdentity"]["encodedSha256"] = json!(format!("{i:064x}"));
        }
        assert!(validate(&p).is_err());
    }
}
