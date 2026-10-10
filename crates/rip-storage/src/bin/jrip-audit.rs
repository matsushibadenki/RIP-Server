//! Local administrator audit export and offline integrity verification.
use rip_storage::{Repository, audit_archive};
use std::{
    io::{Read, Write},
    path::Path,
};

fn export(value: &serde_json::Value, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command,database,after,before,limit,output] if command == "create" => {
            if !Path::new(database).is_file() { return Err("existing repository database required".into()); }
            // Check existing destination before creating a durable snapshot.
            if Path::new(output).exists() { return Err("output already exists".into()); }
            let repo = Repository::open_with_receipt_signer(Path::new(database),None).await?;
            let archive = repo.archive_audit(after.parse()?,before.parse()?,limit.parse()?).await?;
            // Printed first: recover this immutable snapshot if file publication fails.
            println!("archiveId={} digest={}",archive["payload"]["id"].as_str().unwrap(),archive["digest"].as_str().unwrap());
            std::io::stdout().flush()?;
            export(&archive,output)?;
        }
        [command,database,id,output] if command == "export" => {
            if !Path::new(database).is_file() { return Err("existing repository database required".into()); }
            let repo = Repository::open_with_receipt_signer(Path::new(database),None).await?;
            export(&repo.audit_archive(id.parse()?).await?,output)?;
        }
        [command,input,pinned] if command == "verify" => {
            let mut bytes=Vec::new();
            std::fs::File::open(input)?.take((audit_archive::MAX_BYTES*2+1) as u64).read_to_end(&mut bytes)?;
            if bytes.len()>audit_archive::MAX_BYTES*2 { return Err("archive exceeds input limit".into()); }
            let archive = rip_digest::parse(&bytes)?;
            if !audit_archive::verify(&archive) || archive["digest"].as_str()!=Some(pinned) {
                return Err("archive integrity verification failed".into());
            }
            println!("integrity verified against supplied digest; authenticity depends on independent digest pinning");
        }
        _ => return Err("usage: jrip-audit create DATABASE AFTER BEFORE_UNIX_SECONDS LIMIT OUTPUT.json | export DATABASE ARCHIVE_ID OUTPUT.json | verify INPUT.json PINNED_SHA256".into()),
    }
    Ok(())
}
