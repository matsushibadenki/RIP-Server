//! Offline receipt key generation and verification.
use std::{
    io::{Read, Write},
    path::Path,
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, private, public] if command == "generate-key" => {
            let key = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).map_err(|_| "key generation failed")?;
            let signer = rip_storage::receipts::ReceiptSigner::from_pkcs8(key.as_ref())?;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)] {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            // Create both outputs before writing; existing files are never overwritten.
            let mut private_file = options.open(Path::new(private))?;
            let mut public_file = options.open(Path::new(public))?;
            private_file.write_all(key.as_ref())?;
            private_file.sync_all()?;
            public_file.write_all(signer.public_key())?;
            public_file.sync_all()?;
        }
        [command, public, receipt] if command == "verify" => {
            let key = std::fs::read(public)?;
            let mut bytes = Vec::new();
            std::fs::File::open(receipt)?.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 1024 * 1024 { return Err("receipt exceeds 1 MiB".into()); }
            let receipt = serde_json::from_slice(&bytes)?;
            if !rip_storage::receipts::verify(&receipt, &key) { return Err("receipt signature verification failed".into()); }
            println!("signature-verified; revocation not checked");
        }
        [command, database, public, receipt] if command == "verify-current" => {
            if !Path::new(database).is_file() { return Err("existing repository database required".into()); }
            let key = std::fs::read(public)?;
            let mut bytes=Vec::new();
            std::fs::File::open(receipt)?.take(1024*1024+1).read_to_end(&mut bytes)?;
            if bytes.len()>1024*1024 { return Err("receipt exceeds 1 MiB".into()); }
            let receipt=serde_json::from_slice(&bytes)?;
            let repo=rip_storage::Repository::open_with_receipt_signer(Path::new(database),None).await?;
            if !repo.verify_receipt_current(&receipt,&key).await? { return Err("current receipt trust verification failed".into()); }
            println!("verified against current repository key status");
        }
        _ => return Err("usage: jrip-receipt generate-key PRIVATE.pk8 PUBLIC.bin | verify PUBLIC.bin RECEIPT.json | verify-current DATABASE PUBLIC.bin RECEIPT.json".into()),
    }
    Ok(())
}
