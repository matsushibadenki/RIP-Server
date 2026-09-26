#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let root = std::env::var("JRIP_DATA_ROOT").unwrap_or_else(|_| "./data".into());
    let address = std::env::var("JRIP_IPP_LISTEN").unwrap_or_else(|_| "127.0.0.1:8631".into());
    let socket: std::net::SocketAddr = address.parse()?;
    if !socket.ip().is_loopback() && std::env::var("JRIP_ALLOW_INSECURE_LAN").as_deref() != Ok("1")
    {
        return Err("LAN IPP needs JRIP_ALLOW_INSECURE_LAN=1 until IPPS is implemented".into());
    }
    let printer_uri = std::env::var("JRIP_PRINTER_URI")
        .unwrap_or_else(|_| "ipp://127.0.0.1:8631/ipp/print".into());
    let discovery = if std::env::var("JRIP_MDNS").as_deref() == Ok("1") {
        let hostname =
            std::env::var("JRIP_MDNS_HOSTNAME").unwrap_or_else(|_| "jrip-proof.local.".into());
        Some(jrip_ipp::discovery_info(socket, &printer_uri, &hostname)?)
    } else {
        None
    };
    let app = jrip_ipp::open(root.into(), printer_uri).await?;
    let listener = tokio::net::TcpListener::bind(socket).await?;
    let mdns = if let Some(info) = discovery {
        let daemon = mdns_sd::ServiceDaemon::new()?;
        daemon.register(info)?;
        tracing::info!("generic _ipp._tcp DNS-SD service registered");
        Some(daemon)
    } else {
        None
    };
    tracing::info!(%address,"IPP proof printer listening");
    axum::serve(listener, jrip_ipp::router(app))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    if let Some(daemon) = mdns {
        daemon.shutdown()?;
    }
    Ok(())
}
