use rip_storage::{Repository, now};
use std::process::Command;
use uuid::Uuid;

#[tokio::test]
async fn cli_exports_recovers_and_checks_independently_pinned_digest() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("db.sqlite");
    let output = dir.path().join("archive.json");
    let repo = Repository::open_with_receipt_signer(&database, None)
        .await
        .unwrap();
    let seq = repo
        .begin_audit(Uuid::new_v4(), "管理者", "print", None)
        .await
        .unwrap();
    repo.finish_audit(seq, 201).await.unwrap();
    let binary = env!("CARGO_BIN_EXE_jrip-audit");
    let run = Command::new(binary)
        .arg("create")
        .arg(&database)
        .args(["0", &(now() + 1).to_string(), "100"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let bytes = std::fs::read(&output).unwrap();
    let archive: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let digest = archive["digest"].as_str().unwrap();
    assert!(
        Command::new(binary)
            .arg("verify")
            .arg(&output)
            .arg(digest)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        !Command::new(binary)
            .arg("verify")
            .arg(&output)
            .arg("sha256:wrong")
            .status()
            .unwrap()
            .success()
    );
    let id = archive["payload"]["id"].as_str().unwrap();
    let listing = Command::new(binary)
        .arg("list")
        .arg(&database)
        .args(["-", &(now() + 1).to_string(), "100"])
        .output()
        .unwrap();
    assert!(listing.status.success());
    let listing: serde_json::Value = serde_json::from_slice(&listing.stdout).unwrap();
    assert_eq!(listing["archives"][0]["archiveId"], id);
    assert_eq!(listing["hasMore"], false);
    assert!(listing["archives"][0].get("records").is_none());
    let vault = dir.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    assert!(
        Command::new(binary)
            .arg("store")
            .arg(&database)
            .arg(id)
            .arg(&vault)
            .status()
            .unwrap()
            .success()
    );
    let status = Command::new(binary)
        .arg("vault-status")
        .arg(&database)
        .arg(id)
        .arg(&vault)
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["contentMatches"], true);
    assert_eq!(status["acknowledged"], true);
    let preview = Command::new(binary)
        .arg("retention-preview")
        .arg(&database)
        .arg(&vault)
        .args(["30", "-", "100"])
        .output()
        .unwrap();
    assert!(preview.status.success());
    let preview: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(preview["dryRun"], true);
    assert_eq!(preview["deletionImplemented"], true);
    assert_eq!(preview["candidates"], serde_json::json!([]));
    assert!(
        !Command::new(binary)
            .arg("retention-preview")
            .arg(&database)
            .arg(&vault)
            .args(["0", "-", "100"])
            .status()
            .unwrap()
            .success()
    );

    let recovered = dir.path().join("recovered.json");
    assert!(
        Command::new(binary)
            .arg("export")
            .arg(&database)
            .arg(id)
            .arg(&recovered)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(bytes, std::fs::read(&recovered).unwrap());
    assert!(
        !Command::new(binary)
            .arg("export")
            .arg(&database)
            .arg(id)
            .arg(&output)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(bytes, std::fs::read(&output).unwrap());
    let mut changed = archive.clone();
    changed["payload"]["records"][0]["operation"] = serde_json::json!("tampered");
    std::fs::write(&recovered, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(
        !Command::new(binary)
            .arg("verify")
            .arg(&recovered)
            .arg(digest)
            .status()
            .unwrap()
            .success()
    );
    let duplicate = dir.path().join("duplicate.json");
    std::fs::write(&duplicate, b"{\"digest\":\"x\",\"digest\":\"y\"}").unwrap();
    assert!(
        !Command::new(binary)
            .arg("verify")
            .arg(&duplicate)
            .arg(digest)
            .status()
            .unwrap()
            .success()
    );
}

#[tokio::test]
async fn signed_cli_requires_a_key_and_current_verification_checks_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("db.sqlite");
    let output = dir.path().join("signed.json");
    let private = dir.path().join("key.pk8");
    let public_path = dir.path().join("public.bin");
    let key =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let signer = rip_storage::receipts::ReceiptSigner::from_pkcs8(key.as_ref()).unwrap();
    std::fs::write(&private, key.as_ref()).unwrap();
    std::fs::write(&public_path, signer.public_key()).unwrap();
    let repo = Repository::open_with_receipt_signer(&database, None)
        .await
        .unwrap();
    let sequence = repo
        .begin_audit(Uuid::new_v4(), "admin", "test", None)
        .await
        .unwrap();
    repo.finish_audit(sequence, 200).await.unwrap();
    let binary = env!("CARGO_BIN_EXE_jrip-audit");
    assert!(
        !Command::new(binary)
            .env_remove("JRIP_RECEIPT_SIGNING_KEY")
            .arg("create-signed")
            .arg(&database)
            .args(["0", &(now() + 1).to_string(), "100"])
            .arg(&output)
            .status()
            .unwrap()
            .success()
    );
    assert!(!output.exists());
    assert!(
        Command::new(binary)
            .env("JRIP_RECEIPT_SIGNING_KEY", &private)
            .arg("create-signed")
            .arg(&database)
            .args(["0", &(now() + 1).to_string(), "100"])
            .arg(&output)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(binary)
            .arg("verify-signature")
            .arg(&output)
            .arg(&public_path)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(binary)
            .arg("verify-current")
            .arg(&database)
            .arg(&output)
            .arg(&public_path)
            .status()
            .unwrap()
            .success()
    );
    let archive: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    repo.revoke_receipt_key(
        archive["signature"]["keyId"].as_str().unwrap(),
        "compromised",
        "admin",
    )
    .await
    .unwrap();
    assert!(
        !Command::new(binary)
            .arg("verify-current")
            .arg(&database)
            .arg(&output)
            .arg(&public_path)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(binary)
            .arg("verify-signature")
            .arg(&output)
            .arg(&public_path)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        !Command::new(binary)
            .env("JRIP_RECEIPT_SIGNING_KEY", &private)
            .arg("create-signed")
            .arg(&database)
            .args(["0", &(now() + 1).to_string(), "100"])
            .arg(dir.path().join("revoked.json"))
            .status()
            .unwrap()
            .success()
    );
    assert!(!dir.path().join("revoked.json").exists());
}

#[tokio::test]
async fn prune_cli_requires_apply_and_retains_recoverable_vault_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("db");
    let vault = dir.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    let repo = Repository::open_with_receipt_signer(&database, None)
        .await
        .unwrap();
    let seq = repo
        .begin_audit(Uuid::new_v4(), "operator", "test", None)
        .await
        .unwrap();
    repo.finish_audit(seq, 200).await.unwrap();
    let mut archive = repo.archive_audit(0, now() + 1, 100).await.unwrap();
    archive["payload"]["createdAt"] = serde_json::json!(now() - 86400 * 40);
    archive["digest"] = serde_json::json!(rip_digest::digest(&archive["payload"]).unwrap());
    let id = archive["payload"]["id"].as_str().unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&database))
        .await
        .unwrap();
    sqlx::query("UPDATE audit_archives SET payload=? WHERE id=?")
        .bind(serde_json::to_string(&archive).unwrap())
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    repo.store_audit_archive(id.parse().unwrap(), &vault)
        .await
        .unwrap();
    let binary = env!("CARGO_BIN_EXE_jrip-audit");
    let digest = archive["digest"].as_str().unwrap();
    assert!(
        !Command::new(binary)
            .arg("prune")
            .arg(&database)
            .arg(id)
            .arg(&vault)
            .args(["30", digest])
            .status()
            .unwrap()
            .success()
    );
    assert!(repo.audit_archive(id.parse().unwrap()).await.is_ok());
    assert!(
        Command::new(binary)
            .arg("prune")
            .arg(&database)
            .arg(id)
            .arg(&vault)
            .args(["30", digest, "--apply"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(binary)
            .arg("prune-record")
            .arg(&database)
            .arg(id)
            .status()
            .unwrap()
            .success()
    );
    let status = Command::new(binary)
        .arg("vault-status")
        .arg(&database)
        .arg(id)
        .arg(&vault)
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["localSnapshotPruned"], true);
    assert_eq!(status["contentMatches"], true);
    assert!(vault.join(format!("{id}.json")).is_file());
}
