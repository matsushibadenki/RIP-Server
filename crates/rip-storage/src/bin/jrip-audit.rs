//! Local administrator audit export and offline integrity verification.
use rip_storage::{Repository, audit_archive};
use std::{
    io::{Read, Write},
    path::Path,
};

fn export(
    value: &serde_json::Value,
    path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

fn read_archive(path: &str) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((audit_archive::MAX_BYTES * 2 + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > audit_archive::MAX_BYTES * 2 {
        return Err("archive exceeds input limit".into());
    }
    Ok(rip_digest::parse(&bytes)?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command,database,after,before,limit,output] if command == "create" || command == "create-signed" => {
            if !Path::new(database).is_file() { return Err("existing repository database required".into()); }
            // Check existing destination before creating a durable snapshot.
            if Path::new(output).exists() { return Err("output already exists".into()); }
            let repo = if command == "create-signed" {
                if std::env::var_os("JRIP_RECEIPT_SIGNING_KEY").is_none() { return Err("JRIP_RECEIPT_SIGNING_KEY is required for create-signed".into()); }
                Repository::open(Path::new(database)).await?
            } else { Repository::open_with_receipt_signer(Path::new(database),None).await? };
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
            let archive = read_archive(input)?;
            if !audit_archive::verify(&archive) || archive["digest"].as_str()!=Some(pinned) {
                return Err("archive integrity verification failed".into());
            }
            println!("integrity verified against supplied digest; authenticity depends on independent digest pinning");
        }
        [command,input,public] if command == "verify-signature" => {
            let key = std::fs::read(public)?;
            let archive = read_archive(input)?;
            if !audit_archive::verify_signature(&archive,&key) { return Err("archive signature verification failed".into()); }
            println!("archive signature verified; revocation not checked");
        }
        [command,database,input,public] if command == "verify-current" => {
            if !Path::new(database).is_file() { return Err("existing repository database required".into()); }
            let key = std::fs::read(public)?;
            let archive = read_archive(input)?;
            let repo = Repository::open_with_receipt_signer(Path::new(database),None).await?;
            if !repo.verify_audit_archive_current(&archive,&key).await? { return Err("current archive trust verification failed".into()); }
            println!("archive signature verified against current repository key status");
        }
        [command,database,id,directory] if command == "store" || command == "vault-status" => {
            if !Path::new(database).is_file() { return Err("existing repository database required".into()); }
            let repo = Repository::open_with_receipt_signer(Path::new(database),None).await?;
            let result = if command == "store" {repo.store_audit_archive(id.parse()?,Path::new(directory)).await?}
                else {repo.audit_vault_status(id.parse()?,Path::new(directory)).await?};
            println!("{}",serde_json::to_string_pretty(&result)?);
        }
        [command,database,after,before,limit] if command == "list" => {
            if !Path::new(database).is_file() { return Err("existing repository database required".into()); }
            let repo = Repository::open_with_receipt_signer(Path::new(database),None).await?;
            println!("{}",serde_json::to_string_pretty(&repo.audit_archives(if after=="-" {""}else{after},before.parse()?,limit.parse()?).await?)?);
        }
        [command,database,directory,days,after,limit] if command == "retention-preview" => {
            if !Path::new(database).is_file() { return Err("existing repository database required".into()); }
            let days: i64 = days.parse()?;
            if !(1..=3650).contains(&days) { return Err("retention days must be 1..3650".into()); }
            let before = rip_storage::now()-days*86400;
            let repo = Repository::open_with_receipt_signer(Path::new(database),None).await?;
            println!("{}",serde_json::to_string_pretty(&repo.preview_audit_retention(before,Path::new(directory),if after=="-" {""}else{after},limit.parse()?).await?)?);
        }
        [command,database,id,directory,days,digest,apply] if command == "prune" && apply == "--apply" => {
            if !Path::new(database).is_file() { return Err("existing repository database required".into()); }
            let days: i64 = days.parse()?;
            if !(1..=3650).contains(&days) { return Err("retention days must be 1..3650".into()); }
            let repo = Repository::open_with_receipt_signer(Path::new(database),None).await?;
            println!("{}",serde_json::to_string_pretty(&repo.prune_audit_archive(id.parse()?,Path::new(directory),rip_storage::now()-days*86400,digest).await?)?);
        }
        [command,database,id] if command == "prune-record" => {
            if !Path::new(database).is_file() { return Err("existing repository database required".into()); }
            let repo = Repository::open_with_receipt_signer(Path::new(database),None).await?;
            println!("{}",serde_json::to_string_pretty(&repo.audit_archive_prune_record(id.parse()?).await?)?);
        }
        [command,database,id] if command == "remote-store" || command == "remote-status" => {
            if !Path::new(database).is_file() { return Err("existing repository database required".into()); }
            let endpoint = std::env::var("JRIP_AUDIT_VAULT_URL").map_err(|_|"JRIP_AUDIT_VAULT_URL required")?;
            let token = std::env::var("JRIP_AUDIT_VAULT_TOKEN").map_err(|_|"JRIP_AUDIT_VAULT_TOKEN required")?;
            let vault = rip_storage::audit_remote::RemoteVault::new(&endpoint,&token)?;
            let repo = Repository::open_with_receipt_signer(Path::new(database),None).await?;
            let result = if command=="remote-store" {repo.store_audit_remote(id.parse()?,&vault).await?}
                else {repo.audit_remote_status(id.parse()?,&vault).await?};
            println!("{}",serde_json::to_string_pretty(&result)?);
        }
        _ => return Err("usage: jrip-audit create|create-signed DATABASE AFTER BEFORE_UNIX_SECONDS LIMIT OUTPUT.json | export DATABASE ARCHIVE_ID OUTPUT.json | verify INPUT.json PINNED_SHA256 | verify-signature INPUT.json PUBLIC.bin | verify-current DATABASE INPUT.json PUBLIC.bin | store|vault-status DATABASE ARCHIVE_ID VAULT_DIRECTORY | list DATABASE AFTER_UUID_OR_DASH BEFORE_UNIX_SECONDS LIMIT | retention-preview DATABASE VAULT_DIRECTORY DAYS AFTER_UUID_OR_DASH LIMIT | prune DATABASE ARCHIVE_ID VAULT_DIRECTORY DAYS EXPECTED_ARCHIVE_DIGEST --apply | prune-record DATABASE ARCHIVE_ID | remote-store|remote-status DATABASE ARCHIVE_ID".into()),
    }
    Ok(())
}
