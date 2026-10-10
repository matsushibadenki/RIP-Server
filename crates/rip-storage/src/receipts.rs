//! Optional durable receipt signatures. Trust must come from a pinned public key.
use crate::StorageError;
use ring::signature::{self, Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const LEGACY_PROFILE: &str = "org.jrip.receipt-ed25519-v1";
pub const PROFILE: &str = "org.jrip.receipt-ed25519-v2";

pub struct ReceiptSigner(Ed25519KeyPair);
impl ReceiptSigner {
    pub fn from_pkcs8(bytes: &[u8]) -> Result<Self, StorageError> {
        Ed25519KeyPair::from_pkcs8(bytes)
            .map(Self)
            .map_err(|_| StorageError::ReceiptKey)
    }
    pub fn public_key(&self) -> &[u8] {
        self.0.public_key().as_ref()
    }
    pub fn sign(&self, receipt: &mut Value) -> Result<(), StorageError> {
        let mut signed = receipt.clone();
        self.sign_profile(&mut signed, PROFILE)?;
        *receipt = signed;
        Ok(())
    }
    fn sign_profile(&self, receipt: &mut Value, profile: &str) -> Result<(), StorageError> {
        if !receipt.is_object()
            || !receipt["proof"].is_object()
            || receipt.get("signature").is_some()
        {
            return Err(StorageError::Conflict);
        }
        receipt["proof"]["signed"] = json!(true);
        receipt["proof"]["type"] = json!("ed25519-provider-record");
        let key = self.public_key();
        receipt["signature"] = json!({"profile":profile,"algorithm":"Ed25519",
            "keyId":format!("sha256:{:x}", Sha256::digest(key)),
            "publicKey":hex(key),"encoding":"hex","digestProfile":if profile==PROFILE {rip_digest::PROFILE}else{"org.jrip.serde-json-v1"}});
        let bytes = message(receipt)?;
        receipt["signature"]["value"] = json!(hex(self.0.sign(&bytes).as_ref()));
        Ok(())
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2)
        || !s
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}
fn message(receipt: &Value) -> Result<Vec<u8>, StorageError> {
    let mut body = receipt.clone();
    body["signature"]
        .as_object_mut()
        .ok_or(StorageError::Conflict)?
        .remove("value");
    let profile = receipt["signature"]["profile"]
        .as_str()
        .ok_or(StorageError::Conflict)?;
    let mut bytes = profile.as_bytes().to_vec();
    bytes.push(0);
    match profile {
        PROFILE => bytes.extend(rip_digest::canonical(&body).map_err(|_| StorageError::Conflict)?),
        LEGACY_PROFILE => bytes.extend(serde_json::to_vec(&body)?),
        _ => return Err(StorageError::Conflict),
    }
    Ok(bytes)
}
/// Cryptographic verification only; does not check revocation.
/// The embedded public key is informational; callers supply an independently trusted key.
pub fn verify(receipt: &Value, trusted_key: &[u8]) -> bool {
    let sig = &receipt["signature"];
    if trusted_key.len() != 32
        || !matches!(sig["profile"].as_str(), Some(PROFILE | LEGACY_PROFILE))
        || sig["algorithm"] != "Ed25519"
        || sig["encoding"] != "hex"
        || sig["digestProfile"]
            != if sig["profile"] == PROFILE {
                rip_digest::PROFILE
            } else {
                "org.jrip.serde-json-v1"
            }
        || receipt["proof"]["signed"] != true
        || sig["publicKey"] != hex(trusted_key)
        || sig["keyId"] != format!("sha256:{:x}", Sha256::digest(trusted_key))
    {
        return false;
    }
    let Some(bytes) = sig["value"].as_str().and_then(unhex) else {
        return false;
    };
    let Ok(message) = message(receipt) else {
        return false;
    };
    signature::UnparsedPublicKey::new(&signature::ED25519, trusted_key)
        .verify(&message, &bytes)
        .is_ok()
}

// Preserve only public material, including keys used before rotation.
pub(crate) async fn install(
    pool: &sqlx::SqlitePool,
    signer: Option<&ReceiptSigner>,
) -> Result<(), StorageError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS receipt_public_keys(key_id TEXT PRIMARY KEY, public_key TEXT NOT NULL, first_seen INTEGER NOT NULL)").execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS receipt_key_revocations(key_id TEXT PRIMARY KEY, revoked_at INTEGER NOT NULL, reason TEXT NOT NULL, actor TEXT NOT NULL)").execute(pool).await?;
    if let Some(signer) = signer {
        sqlx::query(
            "INSERT OR IGNORE INTO receipt_public_keys(key_id,public_key,first_seen) VALUES(?,?,?)",
        )
        .bind(format!("sha256:{:x}", Sha256::digest(signer.public_key())))
        .bind(hex(signer.public_key()))
        .bind(crate::now())
        .execute(pool)
        .await?;
    }
    Ok(())
}
impl crate::Repository {
    pub async fn receipt_keys(&self, after: &str, limit: i64) -> Result<Value, StorageError> {
        use sqlx::Row;
        if !(1..=100).contains(&limit) {
            return Err(StorageError::Conflict);
        }
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query("SELECT k.key_id,public_key,first_seen,revoked_at,reason,actor FROM receipt_public_keys k LEFT JOIN receipt_key_revocations r ON r.key_id=k.key_id WHERE k.key_id>? ORDER BY k.key_id LIMIT ?")
            .bind(after).bind(limit+1).fetch_all(&mut *tx).await?;
        let has_more = rows.len() > limit as usize;
        let active = self.receipt_signer.as_ref().map(|s| hex(s.public_key()));
        let configured = self.receipt_signer.as_ref().map(|s| key_id(s.public_key()));
        let blocked = match &configured {
            Some(id) => {
                sqlx::query_scalar::<_, i64>(
                    "SELECT EXISTS(SELECT 1 FROM receipt_key_revocations WHERE key_id=?)",
                )
                .bind(id)
                .fetch_one(&mut *tx)
                .await?
                    != 0
            }
            None => false,
        };
        tx.commit().await?;
        let keys: Vec<Value> = rows.into_iter().take(limit as usize).map(|r| {
            let key: String = r.get("public_key");
            json!({"keyId":r.get::<String,_>("key_id"),"publicKey":key,
                "firstSeenAt":r.get::<i64,_>("first_seen"),
                "configuredForThisProcess":active.as_ref()==Some(&key),
                "activeForThisProcess":active.as_ref()==Some(&key) && r.get::<Option<i64>,_>("revoked_at").is_none(),
                "revoked":r.get::<Option<i64>,_>("revoked_at").is_some(),"revokedAt":r.get::<Option<i64>,_>("revoked_at"),
                "revocationReason":r.get::<Option<String>,_>("reason"),"revokedBy":r.get::<Option<String>,_>("actor"),"algorithm":"Ed25519","encoding":"hex"})
        }).collect();
        let next = keys
            .last()
            .map(|k| k["keyId"].as_str().unwrap())
            .unwrap_or(after);
        Ok(
            json!({"profile":PROFILE,"digestProfile":rip_digest::PROFILE,"supportedSignatureProfiles":[PROFILE,LEGACY_PROFILE],
            "signingConfiguredForThisProcess":active.is_some(),"signingEnabledForThisProcess":active.is_some() && !blocked,
            "signingBlockedReason":if blocked {Some("KEY_REVOKED")}else{None},"keys":keys,"hasMore":has_more,
            "nextAfter":next,"trustEstablished":false,"requiresIndependentKeyPinning":true,
            "revocationSupported":true}),
        )
    }
}

pub fn valid_key_id(id: &str) -> bool {
    id.len() == 71
        && id.starts_with("sha256:")
        && id.as_bytes()[7..]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}
fn key_id(key: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(key))
}
// Check under the same SQLite writer transaction as the effect record.
pub(crate) async fn sign_stored(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    signer: &ReceiptSigner,
    receipt: &mut Value,
    acceptance: bool,
) -> Result<(), StorageError> {
    let id = key_id(signer.public_key());
    let revoked = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM receipt_key_revocations WHERE key_id=?)",
    )
    .bind(&id)
    .fetch_one(&mut **tx)
    .await?
        != 0;
    if revoked {
        if acceptance {
            return Err(StorageError::ReceiptKeyRevoked);
        }
        // Preserve terminal processing facts without issuing a revoked signature.
        receipt["proof"]["signingBlocked"] = json!({"code":"KEY_REVOKED","keyId":id});
        return Ok(());
    }
    signer.sign(receipt)
}
impl crate::Repository {
    /// Conservative current-trust check: reject any receipt from a revoked or unknown key.
    pub async fn verify_receipt_current(
        &self,
        receipt: &Value,
        trusted_key: &[u8],
    ) -> Result<bool, StorageError> {
        if !verify(receipt, trusted_key) {
            return Ok(false);
        }
        let valid = sqlx::query_scalar::<_,i64>("SELECT EXISTS(SELECT 1 FROM receipt_public_keys k WHERE k.key_id=? AND NOT EXISTS(SELECT 1 FROM receipt_key_revocations r WHERE r.key_id=k.key_id))")
            .bind(key_id(trusted_key)).fetch_one(&self.pool).await?;
        Ok(valid != 0)
    }
    pub async fn revoke_receipt_key(
        &self,
        id: &str,
        reason: &str,
        actor: &str,
    ) -> Result<Value, StorageError> {
        use sqlx::Row;
        if !valid_key_id(id)
            || !matches!(reason, "compromised" | "retired" | "operator-request")
            || actor.is_empty()
            || actor.len() > 256
        {
            return Err(StorageError::Conflict);
        }
        let mut tx = self.pool.begin().await?;
        // This first write serializes revocation against receipt publication.
        let inserted = sqlx::query("INSERT OR IGNORE INTO receipt_key_revocations(key_id,revoked_at,reason,actor) SELECT key_id,?,?,? FROM receipt_public_keys WHERE key_id=?")
            .bind(crate::now()).bind(reason).bind(actor).bind(id).execute(&mut *tx).await?.rows_affected();
        let row = sqlx::query("SELECT * FROM receipt_key_revocations WHERE key_id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StorageError::NotFound)?;
        if row.get::<String, _>("reason") != reason {
            return Err(StorageError::Conflict);
        }
        if inserted == 1 {
            sqlx::query("INSERT INTO audit_records(correlation_id,identity,operation,target,started_at,finished_at,status) VALUES(?,?,?,?,?,?,200)")
                .bind(uuid::Uuid::new_v4().to_string()).bind(actor).bind("receipt-key-revoke").bind(id).bind(crate::now()).bind(crate::now()).execute(&mut *tx).await?;
        }
        let record = json!({"keyId":id,"revoked":true,"revokedAt":row.get::<i64,_>("revoked_at"),"reason":row.get::<String,_>("reason"),"actor":row.get::<String,_>("actor"),"replayed":inserted==0});
        tx.commit().await?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn signer() -> ReceiptSigner {
        let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        ReceiptSigner::from_pkcs8(doc.as_ref()).unwrap()
    }
    #[test]
    fn signatures_require_trusted_key_and_cover_all_fields() {
        let signer = signer();
        let mut receipt = json!({"receiptId":"a","proof":{"signed":false},"result":{"pages":2}});
        signer.sign(&mut receipt).unwrap();
        assert!(verify(&receipt, signer.public_key()));
        let parsed: Value = serde_json::from_slice(&serde_json::to_vec(&receipt).unwrap()).unwrap();
        assert!(verify(&parsed, signer.public_key()));
        assert!(!verify(&receipt, self::signer().public_key()));
        for field in ["receiptId", "proof", "result", "extra"] {
            let mut tampered = receipt.clone();
            tampered[field] = json!("tampered");
            assert!(!verify(&tampered, signer.public_key()));
        }
        receipt["signature"]["value"] = json!("invalid");
        assert!(!verify(&receipt, signer.public_key()));
        assert!(ReceiptSigner::from_pkcs8(b"invalid").is_err());
        let mut legacy = json!({"receiptId":"legacy","proof":{"signed":false},"x":1.0});
        signer.sign_profile(&mut legacy, LEGACY_PROFILE).unwrap();
        assert!(verify(&legacy, signer.public_key()));
        let stored: Value = serde_json::from_slice(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(verify(&stored, signer.public_key()));
        let mut changed = legacy.clone();
        changed["x"] = json!(1);
        assert!(!verify(&changed, signer.public_key()));

        legacy["signature"]["digestProfile"] = json!(rip_digest::PROFILE);
        assert!(!verify(&legacy, signer.public_key()));
        let mut modern = json!({"receiptId":"modern","proof":{"signed":false},"x":1.0});
        signer.sign(&mut modern).unwrap();
        modern["x"] = json!(1);
        assert!(verify(&modern, signer.public_key()));
        let mut unsupported =
            json!({"receiptId":"out-of-range","proof":{"signed":false},"n":9007199254740993u64});
        let original = unsupported.clone();
        assert!(signer.sign(&mut unsupported).is_err());
        assert_eq!(unsupported, original);
    }
}
