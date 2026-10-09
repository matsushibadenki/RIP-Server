#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut apply = false;
    let mut days = 30i64;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--apply" => apply = true,
            "--retention-days" => days = args.next().ok_or("retention days required")?.parse()?,
            _ => return Err("Usage: jrip-maintenance [--apply] [--retention-days 1..3650]".into()),
        }
    }
    if !(1..=3650).contains(&days) {
        return Err("retention days must be 1..3650".into());
    }
    let root = std::env::var("JRIP_DATA_ROOT").unwrap_or_else(|_| "./data".into());
    let app = jrip_server::open(root.into(), String::new()).await?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(app.root.join("maintenance.lock"))?;
    lock.try_lock()?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &jrip_server::maintenance::run(&app, rip_storage::now() - days * 86400, apply).await?
        )?
    );
    Ok(())
}
