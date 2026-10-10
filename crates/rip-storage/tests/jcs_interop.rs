#[test]
#[ignore = "requires Node.js for independent Ed25519/JCS verification"]
fn node_verifies_new_jcs_signature_and_rejects_tampering() {
    let private =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let signer = rip_storage::receipts::ReceiptSigner::from_pkcs8(private.as_ref()).unwrap();
    let mut receipt = serde_json::json!({"receiptId":"interop","proof":{"signed":false},"numbers":[1.0,-0.0,0.000001,1e-27],"unicode":{"😀":"简体中文","\u{e000}":"日本語"}});
    signer.sign(&mut receipt).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let public = dir.path().join("public.bin");
    let file = dir.path().join("receipt.json");
    std::fs::write(&public, signer.public_key()).unwrap();
    std::fs::write(&file, serde_json::to_vec(&receipt).unwrap()).unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/check-receipt-jcs-interop.mjs");
    let run = || {
        std::process::Command::new("node")
            .arg(&script)
            .arg(&public)
            .arg(&file)
            .output()
            .unwrap()
    };
    let output = run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    receipt["receiptId"] = serde_json::json!("tampered");
    std::fs::write(&file, serde_json::to_vec(&receipt).unwrap()).unwrap();
    assert!(!run().status.success());
}
