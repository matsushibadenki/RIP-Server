use serde_json::Value;
#[test]
fn independent_kms_profile_verifies_offline_and_rejects_tampering() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("archive.json");
    let key = root.path().join("public.raw");
    let fixture = include_str!("../../../tests/corpus/kms-audit-archive.json");
    let value: Value = serde_json::from_str(fixture).unwrap();
    let public = value["signature"]["publicKey"].as_str().unwrap();
    let bytes = (0..public.len())
        .step_by(2)
        .map(|n| u8::from_str_radix(&public[n..n + 2], 16).unwrap())
        .collect::<Vec<_>>();
    std::fs::write(&key, bytes).unwrap();
    std::fs::write(&archive, fixture).unwrap();
    let check = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_jrip-audit"))
            .arg("verify-signature")
            .arg(&archive)
            .arg(&key)
            .output()
            .unwrap()
    };
    let result = check();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut tampered = value.clone();
    tampered["signature"]["kmsKeyId"] = serde_json::json!(
        "arn:aws:kms:us-east-1:123456789012:key/00000000-0000-0000-0000-000000000003"
    );
    std::fs::write(&archive, tampered.to_string()).unwrap();
    assert!(!check().status.success());
    std::fs::write(&archive, fixture).unwrap();
    std::fs::write(&key, [0u8; 32]).unwrap();
    assert!(!check().status.success());
}
#[tokio::test]
async fn kms_creation_cli_fails_closed_without_overwriting_or_local_fallback() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("db");
    let repo = rip_storage::Repository::open_with_receipt_signer(&db, None)
        .await
        .unwrap();
    let output = root.path().join("out.json");
    std::fs::write(&output, b"keep original").unwrap();
    let invoke = || {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_jrip-audit"));
        command
            .arg("create-kms-signed")
            .arg(&db)
            .args(["0", "1792198924", "100"])
            .arg(&output)
            .env_remove("JRIP_RECEIPT_SIGNING_KEY")
            .env_remove("JRIP_KMS_URL");
        command
    };
    let result = invoke().output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("output already exists"));
    assert_eq!(std::fs::read(&output).unwrap(), b"keep original");
    std::fs::remove_file(&output).unwrap();
    let result = invoke().output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid_kms_configuration"));
    assert!(!output.exists());
    let result = invoke()
        .env("JRIP_RECEIPT_SIGNING_KEY", "must-not-be-read")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("local and KMS signing"));
    assert!(
        repo.audit_archives("", rip_storage::now() + 1, 100)
            .await
            .unwrap()["archives"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
