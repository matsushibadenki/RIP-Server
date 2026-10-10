#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let app = jrip_server::open(
        std::env::var("JRIP_DATA_ROOT")
            .unwrap_or("data".into())
            .into(),
        std::env::var("JRIP_API_TOKEN")?,
    )
    .await?;
    loop {
        if let Err(error) = jrip_server::inspection::tick(&app).await {
            tracing::error!(%error,"inspection pass failed");
        }
        tokio::select! {_=tokio::time::sleep(std::time::Duration::from_secs(1))=>{},_=tokio::signal::ctrl_c()=>break}
    }
    Ok(())
}
