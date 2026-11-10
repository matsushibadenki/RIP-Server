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
    if app.local_finder && !listener.local_addr()?.ip().is_loopback() {
        return Err("Finder actions require a loopback API listener".into());
    }
    let lan_address =
        jrip_server::network::lan_address(std::env::var("JRIP_LAN_LISTEN").ok().as_deref())?;
    let lan_listener = if let Some(address) = lan_address {
        Some(tokio::net::TcpListener::bind(address).await?)
    } else {
        None
    };
    let mut signals = jrip_server::shutdown::Signals::new()?;
    let (stop, receive) = tokio::sync::watch::channel(false);
    let signal = tokio::spawn(async move {
        signals.wait().await;
        let _ = stop.send(true);
    });
    tracing::info!(%address,?lan_address,"J-RIP API listening");
    if let Some(lan_listener) = lan_listener {
        let lan_app = jrip_server::network::lan_app(&app);
        tokio::try_join!(
            jrip_server::network::serve(listener, app, receive.clone()),
            jrip_server::network::serve(lan_listener, lan_app, receive)
        )?;
    } else {
        jrip_server::network::serve(listener, app, receive).await?;
    }
    signal.abort();
    Ok(())
}
