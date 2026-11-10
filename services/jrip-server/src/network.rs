//! Optional private-network API listener, with desktop actions disabled.
use std::net::{IpAddr, SocketAddr};
pub fn lan_address(value: Option<&str>) -> std::io::Result<Option<SocketAddr>> {
    let Some(value) = value else { return Ok(None) };
    let address: SocketAddr = value
        .parse()
        .map_err(|_| std::io::Error::other("Invalid JRIP_LAN_LISTEN"))?;
    let private = match address.ip() {
        IpAddr::V4(ip) => ip.is_private(),
        IpAddr::V6(ip) => ip.is_unique_local(),
    };
    if !private || address.port() == 0 {
        return Err(std::io::Error::other(
            "JRIP_LAN_LISTEN requires a specific private LAN address and nonzero port",
        ));
    }
    Ok(Some(address))
}
pub fn lan_app(app: &crate::App) -> crate::App {
    let mut app = app.clone();
    app.local_finder = false;
    app
}
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: crate::App,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> std::io::Result<()> {
    axum::serve(
        listener,
        crate::router(app).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        if !*stop.borrow() {
            let _ = stop.changed().await;
        }
    })
    .await
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    #[test]
    fn binds_only_explicit_private_addresses() {
        assert!(lan_address(None).unwrap().is_none());
        for a in ["192.168.200.14:8080", "10.1.1.1:8080", "[fd00::1]:8080"] {
            assert!(lan_address(Some(a)).is_ok());
        }
        for a in [
            "0.0.0.0:8080",
            "127.0.0.1:8080",
            "8.8.8.8:8080",
            "192.168.1.1:0",
            "[::]:8080",
            "localhost:8080",
        ] {
            assert!(lan_address(Some(a)).is_err());
        }
    }
    #[tokio::test]
    async fn lan_preserves_authentication_and_disables_finder() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = crate::open(dir.path().into(), "test-token".into())
            .await
            .unwrap();
        app.local_finder = true;
        let lan = lan_app(&app);
        assert!(app.local_finder);
        assert!(!lan.local_finder);
        assert_eq!(app.require_print_approval, lan.require_print_approval);
        let router = crate::router(lan);
        let r = router
            .clone()
            .oneshot(Request::get("/api/v1/jobs").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        let r = router
            .oneshot(
                Request::get("/api/v1/console/actions")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let json: serde_json::Value =
            serde_json::from_slice(&to_bytes(r.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(json["finder"], false);
    }
}
