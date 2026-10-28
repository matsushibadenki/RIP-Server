#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var("JRIP_API_TOKEN").map_err(|_| "Set JRIP_API_TOKEN")?;
    if token.len() < 24 {
        return Err("JRIP_API_TOKEN must have at least 24 characters".into());
    }
    let root = std::env::var("JRIP_DATA_ROOT").unwrap_or_else(|_| "./data".into());
    let app = jrip_server::open(root.into(), token).await?;
    let origins = std::env::var("JRIP_MCP_ALLOWED_ORIGINS")
        .unwrap_or_default()
        .split(',')
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_owned())
        .collect();
    let address: std::net::SocketAddr = std::env::var("JRIP_MCP_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8091".into())
        .parse()?;
    let options = jrip_mcp::http::HttpOptions::from_env()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    eprintln!(
        "MCP Streamable HTTP listening on {}",
        listener.local_addr()?
    );
    let (shutdown, signal) = tokio::sync::oneshot::channel();
    let server = axum::serve(
        listener,
        jrip_mcp::http::router_with_options(app, origins, options),
    )
    .with_graceful_shutdown(async {
        let _ = signal.await;
    })
    .into_future();
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => result?,
        _ = tokio::signal::ctrl_c() => {
            let _ = shutdown.send(());
            // Long-lived SSE connections must not keep a requested shutdown alive for 15 minutes.
            if let Ok(result) = tokio::time::timeout(std::time::Duration::from_secs(5), &mut server).await { result?; }
        }
    }
    Ok(())
}
