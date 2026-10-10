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
