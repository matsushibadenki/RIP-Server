#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let mut shutdown = jrip_server::shutdown::Signals::new()?;
    let root = std::env::var("JRIP_DATA_ROOT").unwrap_or_else(|_| "data".into());
    let app = jrip_server::open(root.into(), std::env::var("JRIP_API_TOKEN")?).await?;
    if app.printers.is_empty() {
        return Err("JRIP_PRINTER_CONFIG must configure at least one printer".into());
    }
    loop {
        if let Err(e) = jrip_server::delivery::tick(&app).await {
            tracing::error!(error=%e,"delivery pass failed");
        }
        tokio::select! { _=tokio::time::sleep(std::time::Duration::from_secs(1))=>{}, _=shutdown.wait()=>break }
    }
    Ok(())
}
