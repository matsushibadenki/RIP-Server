use jrip_mcp::Session;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
const TOKEN: &str = "printer-observation-reader-token";
fn req(id: i64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}
fn call(name: &str, args: Value) -> Value {
    req(2, "tools/call", json!({"name":name,"arguments":args}))
}
fn value(reply: Value) -> Value {
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
#[tokio::test]
async fn stored_history_tools_and_resources_are_scoped_strict_and_never_probe_devices() {
    let queries = Arc::new(AtomicUsize::new(0));
    let called = queries.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = axum::Router::new().route(
        "/printer",
        axum::routing::post(move |bytes: axum::body::Bytes| {
            let called = called.clone();
            async move {
                let req = rip_ipp::Request::parse(&bytes).unwrap();
                assert_eq!(req.operation, rip_ipp::GET_PRINTER_ATTRIBUTES);
                called.fetch_add(1, Ordering::SeqCst);
                let mut r = rip_ipp::Response::new([1, 1], rip_ipp::OK, req.request_id);
                r.text(0x47, "attributes-charset", "utf-8");
                r.text(0x48, "attributes-natural-language", "en");
                r.group(4);
                r.text(0x49, "document-format-supported", "application/pdf");
                r.attribute(0x22, "printer-is-accepting-jobs", &[1]);
                r.attribute(0x23, "printer-state", &3i32.to_be_bytes());
                r.integer("marker-levels", 0);
                (
                    [(axum::http::header::CONTENT_TYPE, "application/ipp")],
                    r.finish(),
                )
            }
        }),
    );
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    app.printers = vec![jrip_server::delivery::Printer {
        id: "test".into(),
        uri: format!("ipp://{addr}/printer"),
        media: vec![],
        color_target: None,
    }];
    app.credentials = vec![jrip_server::Credential {
        token: TOKEN.into(),
        identity: "reader".into(),
        scopes: vec!["jobs:read".into()],
        validity: None,
    }];
    let mut session = Session::new(jrip_server::router(app), TOKEN.into()).with_access(
        "reader".into(),
        vec!["jobs:read".into()],
        None,
    );
    let initialized=session.handle(req(1,"initialize",json!({"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"},"capabilities":{}}))).await.unwrap();
    assert!(initialized["error"].is_null());
    session
        .handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;
    let empty = value(
        session
            .handle(call(
                "print_get_printer_observations",
                json!({"printerId":"test"}),
            ))
            .await
            .unwrap(),
    );
    assert!(empty["latest"].is_null());
    assert_eq!(queries.load(Ordering::SeqCst), 0);
    value(
        session
            .handle(call("print_get_printer", json!({"printerId":"test"})))
            .await
            .unwrap(),
    );
    assert_eq!(queries.load(Ordering::SeqCst), 1);
    let history = value(
        session
            .handle(call(
                "print_get_printer_observations",
                json!({"printerId":"test","after":0,"limit":50}),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(
        history["latest"]["snapshot"]["markerSupplies"]["markers"][0]["levelPercent"],
        0
    );
    let sequence = history["events"][0]["sequence"].as_i64().unwrap();
    let event = value(
        session
            .handle(call(
                "print_get_printer_observation",
                json!({"printerId":"test","sequence":sequence}),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(event["snapshot"], history["latest"]["snapshot"]);
    for (uri, expected) in [
        ("jrip://printers/test/observations".to_string(), history),
        (
            format!("jrip://printers/test/observations/{sequence}"),
            event,
        ),
    ] {
        let response = session
            .handle(req(3, "resources/read", json!({"uri":uri})))
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(
                response["result"]["contents"][0]["text"].as_str().unwrap()
            )
            .unwrap(),
            expected
        );
    }
    for (name, args) in [
        (
            "print_get_printer_observations",
            json!({"printerId":"../test"}),
        ),
        (
            "print_get_printer_observations",
            json!({"printerId":"test","after":-1}),
        ),
        (
            "print_get_printer_observations",
            json!({"printerId":"test","limit":101}),
        ),
        (
            "print_get_printer_observation",
            json!({"printerId":"test","sequence":0}),
        ),
        (
            "print_get_printer_observation",
            json!({"printerId":"test","sequence":"1"}),
        ),
    ] {
        assert_eq!(
            session.handle(call(name, args)).await.unwrap()["error"]["code"],
            -32602
        );
    }
    for suffix in ["0", "-1", "01", "+1", "1?x=y", "1/x", "9223372036854775808"] {
        assert_eq!(
            session
                .handle(req(
                    4,
                    "resources/read",
                    json!({"uri":format!("jrip://printers/test/observations/{suffix}")})
                ))
                .await
                .unwrap()["error"]["code"],
            -32602
        );
    }
    let mut forbidden = session.with_access("reader".into(), vec!["artifacts:read".into()], None);
    assert_eq!(
        forbidden
            .handle(req(
                5,
                "resources/read",
                json!({"uri":"jrip://printers/test/observations"})
            ))
            .await
            .unwrap()["error"]["code"],
        -32001
    );
    assert_eq!(
        forbidden
            .handle(call(
                "print_get_printer_observations",
                json!({"printerId":"test"})
            ))
            .await
            .unwrap()["result"]["isError"],
        true
    );
    assert_eq!(
        queries.load(Ordering::SeqCst),
        1,
        "history never probes the device"
    );
    task.abort();
}
