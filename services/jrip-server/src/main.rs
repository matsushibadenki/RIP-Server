#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let token = std::env::var("JRIP_API_TOKEN")
        .map_err(|_| "Set JRIP_API_TOKEN (at least 24 characters)")?;
    if token.len() < 24 {
        return Err("JRIP_API_TOKEN must have at least 24 characters".into());
    }
    let root = std::env::var("JRIP_DATA_ROOT").unwrap_or_else(|_| "./data".into());
    let address = std::env::var("JRIP_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let app = jrip_server::open(root.into(), token).await?;
    let listener = tokio::net::TcpListener::bind(&address).await?;
    tracing::info!(%address,"J-RIP API listening");
    axum::serve(listener, jrip_server::router(app))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
