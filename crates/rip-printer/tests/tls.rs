use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use rip_printer::{NativeIpp, PrinterBackend};
use std::{convert::Infallible, sync::Arc};
use tokio_rustls::rustls::{ServerConfig, pki_types::PrivateKeyDer};

#[tokio::test]
async fn ipps_verifies_trust_name_and_sends_credentials_only_over_tls() {
    let dir = tempfile::tempdir().unwrap();
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");
    let status = std::process::Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-keyout",
            key_path.to_str().unwrap(),
            "-out",
            cert_path.to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "openssl test certificate generation failed"
    );
    let certs = rustls_pemfile::certs(&mut std::io::Cursor::new(std::fs::read(cert_path).unwrap()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key: PrivateKeyDer<'static> =
        rustls_pemfile::private_key(&mut std::io::Cursor::new(std::fs::read(key_path).unwrap()))
            .unwrap()
            .unwrap();
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs.clone(), key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(stream) = acceptor.accept(stream).await else {
                    return;
                };
                let service = hyper::service::service_fn(
                    |request: hyper::Request<hyper::body::Incoming>| async move {
                        assert_eq!(request.headers()["authorization"], "Bearer tls-test-token");
                        let body = request.into_body().collect().await.unwrap().to_bytes();
                        let message = rip_ipp::Request::parse(&body).unwrap();
                        assert_eq!(message.operation, rip_ipp::GET_PRINTER_ATTRIBUTES);
                        let mut reply = rip_ipp::Response::new([1, 1], 0, message.request_id);
                        reply.text(0x47, "attributes-charset", "utf-8");
                        reply.text(0x48, "attributes-natural-language", "en");
                        reply.group(4);
                        reply.text(0x49, "document-format-supported", "application/pdf");
                        reply.attribute(0x22, "printer-is-accepting-jobs", &[1]);
                        reply.enumeration("printer-state", 3);
                        Ok::<_, Infallible>(
                            hyper::Response::builder()
                                .header("content-type", "application/ipp")
                                .body(Full::new(Bytes::from(reply.finish())))
                                .unwrap(),
                        )
                    },
                );
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    let uri = format!("ipps://localhost:{port}/printer");
    let trusted = NativeIpp::new(&uri)
        .unwrap()
        .with_trust_anchors(certs.iter().map(|c| c.to_vec()).collect())
        .unwrap()
        .with_bearer("tls-test-token".into())
        .unwrap();
    assert_eq!(
        trusted.capabilities().await.unwrap().document_formats,
        vec!["application/pdf"]
    );
    assert!(NativeIpp::new(&uri).unwrap().capabilities().await.is_err());
    let mismatch = NativeIpp::new(&format!("ipps://127.0.0.1:{port}/printer"))
        .unwrap()
        .with_trust_anchors(certs.iter().map(|c| c.to_vec()).collect())
        .unwrap();
    assert!(mismatch.capabilities().await.is_err());
    assert!(
        NativeIpp::new("ipp://localhost/printer")
            .unwrap()
            .with_bearer("secret".into())
            .is_err()
    );
    assert!(
        NativeIpp::new(&uri)
            .unwrap()
            .with_bearer("bad\r\nheader".into())
            .is_err()
    );
    task.abort();
}
