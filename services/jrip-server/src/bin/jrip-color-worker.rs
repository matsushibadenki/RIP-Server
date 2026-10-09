#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let app = jrip_server::open(
        std::env::var("JRIP_DATA_ROOT")
            .unwrap_or_else(|_| "data".into())
            .into(),
        std::env::var("JRIP_API_TOKEN")?,
    )
    .await?;
    loop {
        // An in-flight page finishes within its timeout; do not drop the pass and orphan Docker.
        if let Err(error) = jrip_server::color::tick(&app).await {
            tracing::error!(%error,"color worker pass failed");
        }
        tokio::select! { _=tokio::time::sleep(std::time::Duration::from_secs(1))=>{},_=tokio::signal::ctrl_c()=>break }
    }
    Ok(())
}
