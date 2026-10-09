#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let root = std::env::var("JRIP_DATA_ROOT").unwrap_or_else(|_| "data".into());
    let app = jrip_server::open(root.into(), std::env::var("JRIP_API_TOKEN")?).await?;
    if app.printers.is_empty() {
        return Err("JRIP_PRINTER_CONFIG must configure at least one printer".into());
    }
    loop {
        tokio::select! {
            result=jrip_server::delivery::tick(&app)=> { if let Err(e)=result { tracing::error!(error=%e,"delivery pass failed"); } },
            _=tokio::signal::ctrl_c()=>break,
        }
        tokio::select! { _=tokio::time::sleep(std::time::Duration::from_secs(1))=>{}, _=tokio::signal::ctrl_c()=>break }
    }
    Ok(())
}
