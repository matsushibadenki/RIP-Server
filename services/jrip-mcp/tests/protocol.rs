use jrip_mcp::{Session, serve};
use serde_json::{Value, json};
const TOKEN: &str = "test-token-at-least-24-characters";
async fn session() -> (tempfile::TempDir, Session) {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let audit = app.repo.clone();
    (
        dir,
        Session::new(jrip_server::router(app), TOKEN.into()).with_audit(audit),
    )
}
fn req(id: i64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}
async fn initialize(s: &mut Session) {
    let reply = s.handle(req(1,"initialize",json!({"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"},"capabilities":{}}))).await.unwrap();
    assert_eq!(reply["result"]["protocolVersion"], "2025-11-25");
    assert!(
        s.handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await
            .is_none()
    );
}
#[tokio::test]
async fn handshake_discovery_and_validated_api_calls() {
    let (_dir, mut s) = session().await;
    assert_eq!(
        s.handle(req(0, "tools/list", json!({}))).await.unwrap()["error"]["code"],
        -32000
    );
    initialize(&mut s).await;
    let listing = s.handle(req(2, "tools/list", json!({}))).await.unwrap();
    assert_eq!(listing["result"]["tools"].as_array().unwrap().len(), 45);
    let stages = s
        .handle(req(
            90,
            "tools/call",
            json!({"name":"print_get_capabilities"}),
        ))
        .await
        .unwrap();
    assert_eq!(stages["result"]["isError"], false);
    let result = s
        .handle(req(3, "tools/call", json!({"name":"rip_get_capabilities"})))
        .await
        .unwrap();
    assert_eq!(result["result"]["isError"], false);
    let manifest: Value =
        serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        manifest["capabilities"][0]["id"],
        "org.jrip.rip.export_tiff"
    );
    let result = s
        .handle(req(
            4,
            "tools/call",
            json!({"name":"rip_list_jobs","arguments":{"limit":1}}),
        ))
        .await
        .unwrap();
    assert_eq!(result["result"]["content"][0]["text"], "[]");
    for args in [
        json!({"limit":101}),
        json!({"offset":-1}),
        json!({"limit":1.5}),
        json!({"hostPath":"/tmp"}),
    ] {
        assert_eq!(
            s.handle(req(
                5,
                "tools/call",
                json!({"name":"rip_list_jobs","arguments":args})
            ))
            .await
            .unwrap()["error"]["code"],
            -32602
        );
    }
    assert_eq!(
        s.handle(req(
            6,
            "tools/call",
            json!({"name":"rip_get_job","arguments":{"jobId":"../../health"}})
        ))
        .await
        .unwrap()["error"]["code"],
        -32602
    );
    let result = s
        .handle(req(
            7,
            "tools/call",
            json!({"name":"rip_get_job","arguments":{"jobId":uuid::Uuid::new_v4()}}),
        ))
        .await
        .unwrap();
    assert_eq!(result["result"]["isError"], true);
    for args in [
        json!({"jobId":uuid::Uuid::new_v4()}),
        json!({"jobId":uuid::Uuid::new_v4(),"expectedRevision":-1}),
        json!({"jobId":uuid::Uuid::new_v4(),"expectedRevision":1.5}),
    ] {
        assert_eq!(
            s.handle(req(
                9,
                "tools/call",
                json!({"name":"rip_hold_job","arguments":args})
            ))
            .await
            .unwrap()["error"]["code"],
            -32602
        );
    }
    assert_eq!(
        s.handle(req(8, "unknown", json!({}))).await.unwrap()["error"]["code"],
        -32601
    );
    assert!(
        s.handle(json!({"jsonrpc":"2.0","method":"unknown"}))
            .await
            .is_none()
    );
}
#[tokio::test]
async fn stdio_recovers_from_parse_error_and_emits_only_jsonrpc() {
    let (_dir, s) = session().await;
    let input = b"not-json\n{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}\n";
    let mut output = Vec::new();
    serve(&input[..], &mut output, s).await.unwrap();
    let text = String::from_utf8(output).unwrap();
    let replies: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["error"]["code"], -32700);
    assert_eq!(replies[1]["id"], 9);
}
#[tokio::test]
async fn oversized_messages_are_bounded() {
    let (_dir, s) = session().await;
    let input = vec![b'x'; jrip_mcp::MAX_MESSAGE_BYTES + 1];
    let mut output = Vec::new();
    assert_eq!(
        serve(&input[..], &mut output, s).await.unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    assert!(output.is_empty());
}

#[tokio::test]
async fn process_reads_existing_jobs_and_resumes_history() {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tokio::io::AsyncWriteExt;
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let router = jrip_server::router(app);
    let body = "--b\r\nContent-Disposition: form-data; name=\"manifest\"\r\n\r\n{\"name\":\"mcp.ps\",\"format\":\"application/postscript\",\"dpi\":300}\r\n--b\r\nContent-Disposition: form-data; name=\"document\"; filename=\"mcp.ps\"\r\n\r\n%!PS\nshowpage\n\r\n--b--\r\n";
    let response = router
        .oneshot(
            Request::post("/api/v1/jobs")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "multipart/form-data; boundary=b")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let job: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_jrip-mcp"))
        .env("JRIP_DATA_ROOT", dir.path())
        .env("JRIP_API_TOKEN", TOKEN)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let messages = [
        req(
            1,
            "initialize",
            json!({"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"},"capabilities":{}}),
        ),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        req(
            2,
            "tools/call",
            json!({"name":"rip_get_job","arguments":{"jobId":job["id"]}}),
        ),
        req(
            3,
            "tools/call",
            json!({"name":"rip_get_events","arguments":{"jobId":job["id"],"after":0}}),
        ),
        req(
            4,
            "tools/call",
            json!({"name":"rip_get_events","arguments":{"jobId":job["id"],"after":i64::MAX}}),
        ),
        req(
            5,
            "tools/call",
            json!({"name":"rip_hold_job","arguments":{"jobId":job["id"],"expectedRevision":0}}),
        ),
        req(
            6,
            "tools/call",
            json!({"name":"rip_release_job","arguments":{"jobId":job["id"],"expectedRevision":0}}),
        ),
        req(
            7,
            "tools/call",
            json!({"name":"rip_release_job","arguments":{"jobId":job["id"],"expectedRevision":1}}),
        ),
        req(
            8,
            "tools/call",
            json!({"name":"rip_cancel_job","arguments":{"jobId":job["id"],"expectedRevision":2}}),
        ),
    ];
    let mut stdin = child.stdin.take().unwrap();
    for message in messages {
        stdin
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }
    drop(stdin);
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let replies: Vec<Value> = text
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(replies.len(), 8);
    for (index, state, revision) in [(4, "HELD", 1), (6, "QUEUED", 2), (7, "CANCELLED", 3)] {
        assert_eq!(replies[index]["result"]["isError"], false);
        let changed: Value = serde_json::from_str(
            replies[index]["result"]["content"][0]["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(changed["state"], state);
        assert_eq!(changed["revision"], revision);
    }
    assert_eq!(replies[5]["result"]["isError"], true);
    let conflict: Value =
        serde_json::from_str(replies[5]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(conflict["error"]["code"], "state_conflict");
    let returned: Value =
        serde_json::from_str(replies[1]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(returned["id"], job["id"]);
    let history: Value =
        serde_json::from_str(replies[2]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(history["events"][0]["state"], "QUEUED");
    let empty: Value =
        serde_json::from_str(replies[3]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(empty["events"], json!([]));
}

fn tool_value(reply: Value) -> Value {
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
#[tokio::test]
async fn submit_retry_replay_and_notifications_share_persistent_state() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let inbox = tempfile::tempdir().unwrap();
    let document = b"%!PS-Adobe-3.0\nshowpage\n";
    tokio::fs::write(inbox.path().join("proof.ps"), document)
        .await
        .unwrap();
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let router = jrip_server::router(app);
    let mut s = Session::new(router.clone(), TOKEN.into()).with_access(
        "test-agent".into(),
        vec!["admin".into()],
        Some(inbox.path().into()),
    );
    initialize(&mut s).await;
    let intent = json!({"document":{"uri":"https://example.org/proof.ps","mediaType":"application/postscript","sha256":format!("{:x}",Sha256::digest(document))},"copies":{"mode":"required","value":1},"output":{"destination":"artifact"}});
    let preflight = tool_value(
        s.handle(req(
            10,
            "tools/call",
            json!({"name":"print_create_preflight","arguments":{"intent":intent}}),
        ))
        .await
        .unwrap(),
    );
    let args = json!({"preflightId":preflight["preflightId"],"preflightDigest":preflight["preflightDigest"],"sourceName":"proof.ps"});
    tokio::fs::write(
        inbox.path().join("corrupt.ps"),
        b"%!PS-Adobe-3.0\nchanged\n",
    )
    .await
    .unwrap();
    let large = tokio::fs::File::create(inbox.path().join("large.ps"))
        .await
        .unwrap();
    large
        .set_len(jrip_server::MAX_DOCUMENT_BYTES as u64 + 1)
        .await
        .unwrap();
    for source in ["corrupt.ps", "large.ps", "missing.ps"] {
        let mut invalid = args.clone();
        invalid["sourceName"] = json!(source);
        assert_eq!(
            s.handle(req(
                22,
                "tools/call",
                json!({"name":"rip_submit","arguments":invalid})
            ))
            .await
            .unwrap()["result"]["isError"],
            true
        );
    }
    let submitted = tool_value(
        s.handle(req(
            11,
            "tools/call",
            json!({"name":"rip_submit","arguments":args}),
        ))
        .await
        .unwrap(),
    );
    let id = &submitted["job"]["id"];
    let replay = tool_value(
        s.handle(req(
            12,
            "tools/call",
            json!({"name":"rip_submit","arguments":args}),
        ))
        .await
        .unwrap(),
    );
    assert_eq!(replay["job"]["id"], *id);
    assert_eq!(replay["replayed"], true);
    let uri = format!("jrip://jobs/{}/events", id.as_str().unwrap());
    assert_eq!(
        s.handle(req(13, "resources/subscribe", json!({"uri":uri})))
            .await
            .unwrap()["result"],
        json!({})
    );
    let updates = s.notifications().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["method"], "notifications/resources/updated");
    assert!(s.notifications().await.is_empty());
    let request_id = uuid::Uuid::new_v4();
    let retry_args = json!({"jobId":id,"expectedRevision":0,"requestId":request_id});
    assert_eq!(
        s.handle(req(
            14,
            "tools/call",
            json!({"name":"rip_retry_job","arguments":retry_args})
        ))
        .await
        .unwrap()["result"]["isError"],
        true
    );
    tool_value(
        s.handle(req(
            15,
            "tools/call",
            json!({"name":"rip_cancel_job","arguments":{"jobId":id,"expectedRevision":0}}),
        ))
        .await
        .unwrap(),
    );
    assert_eq!(s.notifications().await.len(), 1);
    let retry_args = json!({"jobId":id,"expectedRevision":1,"requestId":uuid::Uuid::new_v4()});
    let retry = tool_value(
        s.handle(req(
            16,
            "tools/call",
            json!({"name":"rip_retry_job","arguments":retry_args}),
        ))
        .await
        .unwrap(),
    );
    assert_ne!(retry["job"]["id"], *id);
    assert_eq!(retry["job"]["state"], "QUEUED");
    assert_eq!(
        retry["job"]["source_sha256"],
        submitted["job"]["source_sha256"]
    );
    let mut restarted = Session::new(
        jrip_server::router(
            jrip_server::open(dir.path().into(), TOKEN.into())
                .await
                .unwrap(),
        ),
        TOKEN.into(),
    );
    initialize(&mut restarted).await;
    let replay = tool_value(
        restarted
            .handle(req(
                17,
                "tools/call",
                json!({"name":"rip_retry_job","arguments":retry_args}),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(replay["job"]["id"], retry["job"]["id"]);
    assert_eq!(replay["replayed"], true);
    assert_eq!(
        s.handle(req(18, "resources/unsubscribe", json!({"uri":uri})))
            .await
            .unwrap()["result"],
        json!({})
    );
    assert!(s.notifications().await.is_empty());
    let read = s
        .handle(req(19, "resources/read", json!({"uri":uri})))
        .await
        .unwrap();
    let history: Value =
        serde_json::from_str(read["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(history["events"].as_array().unwrap().len(), 2);
    for source in ["../proof.ps", "/etc/passwd", "."] {
        let mut invalid = args.clone();
        invalid["sourceName"] = json!(source);
        assert_eq!(
            s.handle(req(
                20,
                "tools/call",
                json!({"name":"rip_submit","arguments":invalid})
            ))
            .await
            .unwrap()["result"]["isError"],
            true
        );
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(inbox.path().join("proof.ps"), inbox.path().join("link.ps"))
            .unwrap();
        let mut invalid = args.clone();
        invalid["sourceName"] = json!("link.ps");
        assert_eq!(
            s.handle(req(
                21,
                "tools/call",
                json!({"name":"rip_submit","arguments":invalid})
            ))
            .await
            .unwrap()["result"]["isError"],
            true
        );
    }
}
#[tokio::test]
async fn scoped_tools_hide_and_deny_mutations_and_subscriptions() {
    let (dir, _s) = session().await;
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let mut s = Session::new(jrip_server::router(app), TOKEN.into()).with_access(
        "reader".into(),
        vec!["jobs:read".into()],
        None,
    );
    initialize(&mut s).await;
    let tools = s.handle(req(1, "tools/list", json!({}))).await.unwrap();
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 22);
    assert_eq!(s.handle(req(2,"tools/call",json!({"name":"rip_cancel_job","arguments":{"jobId":uuid::Uuid::new_v4(),"expectedRevision":0}}))).await.unwrap()["result"]["isError"],true);
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let mut no_read = Session::new(jrip_server::router(app), TOKEN.into()).with_access(
        "submitter".into(),
        vec!["jobs:submit".into()],
        None,
    );
    initialize(&mut no_read).await;
    assert_eq!(
        no_read
            .handle(req(
                3,
                "resources/subscribe",
                json!({"uri":format!("jrip://jobs/{}/events",uuid::Uuid::new_v4())})
            ))
            .await
            .unwrap()["error"]["code"],
        -32001
    );
}
#[tokio::test]
async fn stdio_sends_notifications_without_additional_client_input() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let router = jrip_server::router(app);
    // Empty databases have no valid resource; use the lifecycle helper to publish a source.
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let body = "--b\r\nContent-Disposition: form-data; name=\"manifest\"\r\n\r\n{\"name\":\"notify.ps\",\"format\":\"application/postscript\",\"dpi\":300}\r\n--b\r\nContent-Disposition: form-data; name=\"document\"; filename=\"x\"\r\n\r\n%!PS\nshowpage\n\r\n--b--\r\n";
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/v1/jobs")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "multipart/form-data; boundary=b")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let job: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    let (client, server) = tokio::io::duplex(65536);
    let (reader, writer) = tokio::io::split(server);
    let task = tokio::spawn(serve(
        tokio::io::BufReader::new(reader),
        writer,
        Session::new(router, TOKEN.into()),
    ));
    let (reader, mut writer) = tokio::io::split(client);
    let mut reader = tokio::io::BufReader::new(reader);
    for message in [
        req(
            1,
            "initialize",
            json!({"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"},"capabilities":{}}),
        ),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        req(
            2,
            "resources/subscribe",
            json!({"uri":format!("jrip://jobs/{}/events",job["id"].as_str().unwrap())}),
        ),
    ] {
        writer
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }
    let mut notified = false;
    for _ in 0..3 {
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            reader.read_line(&mut line),
        )
        .await
        .unwrap()
        .unwrap();
        let message: Value = serde_json::from_str(&line).unwrap();
        if message["method"] == "notifications/resources/updated" {
            notified = true;
        }
    }
    assert!(notified);
    drop(writer);
    drop(reader);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn mcp_audit_records_tool_identity_without_arguments() {
    let (dir, mut session) = session().await;
    initialize(&mut session).await;
    let result = session
        .handle(req(
            2,
            "tools/call",
            json!({"name":"rip_list_jobs","arguments":{"limit":1}}),
        ))
        .await
        .unwrap();
    assert_eq!(result["result"]["isError"], false);
    let repo = rip_storage::Repository::open(&dir.path().join("jobs.sqlite3"))
        .await
        .unwrap();
    let records = repo.audit_records(0, 100).await.unwrap();
    assert!(
        records["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["operation"] == "rip_list_jobs" && record["status"] == 200)
    );
    assert!(!records.to_string().contains(TOKEN));
}

#[tokio::test]
async fn resources_and_revision_checked_deletion_share_api_state() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let repo = app.repo.clone();
    let router = jrip_server::router(app);
    let manifest = json!({"name":"test.pdf","format":"application/pdf","dpi":300});
    let body = format!(
        "--test\r\nContent-Disposition: form-data; name=\"manifest\"\r\n\r\n{manifest}\r\n--test\r\nContent-Disposition: form-data; name=\"document\"; filename=\"test.pdf\"\r\n\r\n%PDF-1.4\nmock\n\r\n--test--\r\n"
    );
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::post("/api/v1/jobs")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "multipart/form-data; boundary=test")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let job: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap(),
    )
    .unwrap();
    let job_id = job["id"].as_str().unwrap();
    let mut session = Session::new(router, TOKEN.into()).with_audit(repo.clone());
    initialize(&mut session).await;
    for suffix in ["", "/manifest", "/events"] {
        let reply = session
            .handle(req(
                2,
                "resources/read",
                json!({"uri":format!("jrip://jobs/{job_id}{suffix}")}),
            ))
            .await
            .unwrap();
        assert!(reply["result"]["contents"][0]["text"].is_string());
    }
    let printers = session
        .handle(req(3, "tools/call", json!({"name":"print_list_printers"})))
        .await
        .unwrap();
    let printers: Value =
        serde_json::from_str(printers["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(printers["printers"], json!([]));
    let canceled = session.handle(req(4,"tools/call",json!({"name":"rip_cancel_job","arguments":{"jobId":job_id,"expectedRevision":job["revision"]}}))).await.unwrap();
    let canceled: Value =
        serde_json::from_str(canceled["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let stale = session.handle(req(5,"tools/call",json!({"name":"rip_delete_job","arguments":{"jobId":job_id,"expectedRevision":job["revision"]}}))).await.unwrap();
    assert_eq!(stale["result"]["isError"], true);
    let deleted = session.handle(req(6,"tools/call",json!({"name":"rip_delete_job","arguments":{"jobId":job_id,"expectedRevision":canceled["revision"]}}))).await.unwrap();
    assert_eq!(deleted["result"]["isError"], false);
    assert!(
        repo.get(uuid::Uuid::parse_str(job_id).unwrap())
            .await
            .is_err()
    );
    let events = session
        .handle(req(
            7,
            "resources/read",
            json!({"uri":format!("jrip://jobs/{job_id}/events")}),
        ))
        .await
        .unwrap();
    assert!(events["result"]["contents"][0]["text"].is_string());
    let mut reader = session.with_access("reader".into(), vec!["jobs:read".into()], None);
    let forbidden = reader.handle(req(8,"tools/call",json!({"name":"print_cancel_delivery","arguments":{"deliveryId":uuid::Uuid::new_v4(),"expectedRevision":0}}))).await.unwrap();
    assert_eq!(forbidden["result"]["isError"], true);
    let forbidden = reader
        .handle(req(
            9,
            "resources/read",
            json!({"uri":format!("jrip://jobs/{job_id}/artifacts")}),
        ))
        .await
        .unwrap();
    assert_eq!(forbidden["error"]["code"], -32001);
    for suffix in [
        "/../manifest",
        "/events?after=0",
        "/",
        "/artifacts/private.tiff",
    ] {
        let reply = reader
            .handle(req(
                10,
                "resources/read",
                json!({"uri":format!("jrip://jobs/{job_id}{suffix}")}),
            ))
            .await
            .unwrap();
        assert_eq!(reply["error"]["code"], -32602);
    }
}

#[tokio::test]
async fn printer_capability_tool_rejects_paths_urls_and_control_characters() {
    let (_dir, mut session) = session().await;
    initialize(&mut session).await;
    for printer in [
        "../jobs",
        "ipp://remote/print",
        "printer\n",
        "printer?x=1",
        "",
    ] {
        for name in ["print_get_printer", "print_get_capabilities"] {
            let response = session
                .handle(req(
                    2,
                    "tools/call",
                    json!({"name":name,"arguments":{"printerId":printer}}),
                ))
                .await
                .unwrap();
            assert_eq!(response["error"]["code"], -32602);
        }
    }
}

#[tokio::test]
async fn color_tools_pin_policy_and_delegate_revision_checked_queue_control() {
    let directory = tempfile::tempdir().unwrap();
    let app = jrip_server::open(directory.path().into(), TOKEN.into())
        .await
        .unwrap();
    let job = rip_core::Job {
        id: uuid::Uuid::new_v4(),
        manifest: serde_json::from_value(
            json!({"name":"source.pdf","format":"application/pdf","dpi":300,"color_mode":"rgb"}),
        )
        .unwrap(),
        state: rip_core::JobState::Spooling,
        source_sha256: "a".repeat(64),
        created_at: 0,
        updated_at: 0,
        revision: 0,
        error_code: None,
        selected_engine: None,
        worker_run_id: None,
    };
    app.repo.insert(&job).await.unwrap();
    app.repo
        .complete_with_artifacts(
            job.clone(),
            &[rip_core::Artifact {
                page: 1,
                name: "page-000001.tiff".into(),
                media_type: "image/tiff".into(),
                bytes: 100,
                sha256: "b".repeat(64),
            }],
        )
        .await
        .unwrap();
    let job = app.repo.get(job.id).await.unwrap();
    let profile = rip_color::registry::Registry::open(app.root.join("color-profiles"))
        .unwrap()
        .register(&lcms2::Profile::new_srgb().icc().unwrap())
        .unwrap();
    let mut session = Session::new(jrip_server::router(app.clone()), TOKEN.into());
    initialize(&mut session).await;
    let caps = tool_value(
        session
            .handle(req(
                9,
                "tools/call",
                json!({"name":"color_get_capabilities","arguments":{}}),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(caps["capability"]["bitDepths"], json!([8]));
    assert_eq!(caps["capability"]["workerAvailability"], "not-probed");
    assert_eq!(caps["capabilityRevision"].as_str().unwrap().len(), 64);
    let id = uuid::Uuid::new_v4();
    let args = json!({"jobId":job.id,"requestId":id,"expectedRevision":job.revision,"inputProfileSha256":profile.sha256,"outputProfileSha256":profile.sha256,"policy":{"schemaVersion":1,"inputColor":"rgb","outputColor":"rgb","intent":"relative_colorimetric","blackPointCompensation":false,"blackPreservation":"none","tacPercent":null}});
    let queued = tool_value(
        session
            .handle(req(
                2,
                "tools/call",
                json!({"name":"color_apply","arguments":args}),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(queued["id"], id.to_string());
    assert_eq!(queued["state"], "QUEUED");
    let profile_value = tool_value(
        session
            .handle(req(
                3,
                "tools/call",
                json!({"name":"color_get_profile","arguments":{"profileSha256":profile.sha256}}),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(profile_value["sha256"], profile.sha256);
    assert_eq!(
        session
            .handle(req(
                4,
                "tools/call",
                json!({"name":"color_get_profile","arguments":{"profileSha256":"../escape"}})
            ))
            .await
            .unwrap()["error"]["code"],
        -32602
    );
    let cancel=tool_value(session.handle(req(5,"tools/call",json!({"name":"color_cancel_attempt","arguments":{"colorAttemptId":id,"expectedRevision":0}}))).await.unwrap());
    assert_eq!(cancel["state"], "CANCELLED");
    assert_eq!(session.handle(req(6,"tools/call",json!({"name":"color_cancel_attempt","arguments":{"colorAttemptId":id,"expectedRevision":0}}))).await.unwrap()["result"]["isError"],true);
    let events = tool_value(
        session
            .handle(req(
                7,
                "tools/call",
                json!({"name":"color_get_events","arguments":{"colorAttemptId":id}}),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(events["events"].as_array().unwrap().len(), 2);
    let mut reader = Session::new(jrip_server::router(app), TOKEN.into()).with_access(
        "reader".into(),
        vec!["jobs:read".into()],
        None,
    );
    initialize(&mut reader).await;
    assert_eq!(
        reader
            .handle(req(
                8,
                "tools/call",
                json!({"name":"color_apply","arguments":args})
            ))
            .await
            .unwrap()["result"]["isError"],
        true
    );
}

#[tokio::test]
async fn timeline_resource_subscription_resumes_and_respects_read_scope() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let job:rip_core::Job=serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"manifest":{"name":"timeline.pdf","format":"application/pdf"},"state":"COMPLETED","source_sha256":"a".repeat(64),"created_at":0,"updated_at":0,"revision":0,"error_code":null,"selected_engine":null,"worker_run_id":null})).unwrap();
    app.repo.insert(&job).await.unwrap();
    let id = uuid::Uuid::new_v4();
    app.repo
        .create_pipeline(id, job.id, 0, json!({}))
        .await
        .unwrap();
    let mut s = Session::new(jrip_server::router(app.clone()), TOKEN.into());
    initialize(&mut s).await;
    let uri = format!("jrip://pipelines/{id}/timeline");
    assert_eq!(
        s.handle(req(2, "resources/subscribe", json!({"uri":uri})))
            .await
            .unwrap()["result"],
        json!({})
    );
    assert_eq!(s.notifications().await[0]["params"]["uri"], uri);
    assert!(s.notifications().await.is_empty());
    let read = s
        .handle(req(3, "resources/read", json!({"uri":uri})))
        .await
        .unwrap();
    let snapshot: Value =
        serde_json::from_str(read["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    let after = snapshot["nextAfter"].clone();
    app.repo
        .update_pipeline(id, 0, "BLOCKED", Some("TEST_BLOCKED"), None)
        .await
        .unwrap();
    assert_eq!(s.notifications().await.len(), 1);
    assert!(s.notifications().await.is_empty());
    let mut restarted = Session::new(jrip_server::router(app.clone()), TOKEN.into());
    initialize(&mut restarted).await;
    let page=tool_value(restarted.handle(req(4,"tools/call",json!({"name":"print_get_pipeline_timeline","arguments":{"pipelineId":id,"after":after}}))).await.unwrap());
    let unified = tool_value(
        restarted
            .handle(req(
                40,
                "tools/call",
                json!({"name":"print_get_pipeline_ticket","arguments":{"pipelineId":id}}),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(unified["ticket"]["pipelineId"], id.to_string());
    assert_eq!(unified["execution"]["pipelineState"], "BLOCKED");
    assert_eq!(unified["execution"]["completed"], false);
    assert_eq!(page["events"].as_array().unwrap().len(), 1);
    assert_eq!(page["events"][0]["reasonCode"], "TEST_BLOCKED");
    assert_eq!(
        s.handle(req(5, "resources/unsubscribe", json!({"uri":uri})))
            .await
            .unwrap()["result"],
        json!({})
    );
    app.repo
        .update_pipeline(id, 1, "QUEUED", None, None)
        .await
        .unwrap();
    assert!(s.notifications().await.is_empty());
    let mut forbidden = Session::new(jrip_server::router(app), TOKEN.into()).with_access(
        "artifact-reader".into(),
        vec!["artifacts:read".into()],
        None,
    );
    initialize(&mut forbidden).await;
    for method in ["resources/read", "resources/subscribe"] {
        assert_eq!(
            forbidden
                .handle(req(6, method, json!({"uri":uri})))
                .await
                .unwrap()["error"]["code"],
            -32001
        );
    }
    for invalid in [
        format!("{uri}?after=0"),
        "jrip://pipelines/../../jobs/timeline".into(),
    ] {
        assert_eq!(
            s.handle(req(7, "resources/subscribe", json!({"uri":invalid})))
                .await
                .unwrap()["error"]["code"],
            -32602
        );
    }
}

#[tokio::test]
async fn stdio_pushes_timeline_updates_without_client_polling() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let job:rip_core::Job=serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"manifest":{"name":"push.pdf","format":"application/pdf"},"state":"COMPLETED","source_sha256":"a".repeat(64),"created_at":0,"updated_at":0,"revision":0,"error_code":null,"selected_engine":null,"worker_run_id":null})).unwrap();
    app.repo.insert(&job).await.unwrap();
    let id = uuid::Uuid::new_v4();
    app.repo
        .create_pipeline(id, job.id, 0, json!({}))
        .await
        .unwrap();
    let uri = format!("jrip://pipelines/{id}/timeline");
    let (client, server) = tokio::io::duplex(256 * 1024);
    let (read, write) = tokio::io::split(server);
    let task = tokio::spawn(serve(
        tokio::io::BufReader::new(read),
        write,
        Session::new(jrip_server::router(app.clone()), TOKEN.into()),
    ));
    let (read, mut write) = tokio::io::split(client);
    let mut read = tokio::io::BufReader::new(read);
    for message in [
        req(
            1,
            "initialize",
            json!({"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"},"capabilities":{}}),
        ),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        req(2, "resources/subscribe", json!({"uri":uri})),
    ] {
        write
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }
    loop {
        let mut line = String::new();
        tokio::time::timeout(std::time::Duration::from_secs(3), read.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        let value: Value = serde_json::from_str(&line).unwrap();
        if value["method"] == "notifications/resources/updated" {
            assert_eq!(value["params"]["uri"], uri);
            break;
        }
    }
    app.repo
        .update_pipeline(id, 0, "BLOCKED", Some("TEST_BLOCKED"), None)
        .await
        .unwrap();
    let mut line = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(3), read.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let value: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["method"], "notifications/resources/updated");
    assert_eq!(value["params"]["uri"], uri);
    drop(read);
    drop(write);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn atomic_pipeline_tool_uses_inbox_and_requires_all_scopes() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = tempfile::tempdir().unwrap();
    tokio::fs::write(inbox.path().join("proof.pdf"), b"%PDF-1.7\nsource")
        .await
        .unwrap();
    let mut app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    app.printers = vec![jrip_server::delivery::Printer {
        id: "test".into(),
        uri: "ipp://127.0.0.1:9/ipp/print".into(),
        media: vec![jrip_server::delivery::Media {
            name: "square".into(),
            width_microns: 25400,
            height_microns: 25400,
        }],
        color_target: None,
    }];
    let repo = app.repo.clone();
    let router = jrip_server::router(app);
    let mut s = Session::new(router.clone(), TOKEN.into()).with_access(
        "test-agent".into(),
        vec!["admin".into()],
        Some(inbox.path().into()),
    );
    initialize(&mut s).await;
    let args = json!({"sourceName":"proof.pdf","manifest":{"name":"proof.pdf","format":"application/pdf","dpi":72,"color_mode":"rgb"},"pipeline":{"requestId":uuid::Uuid::new_v4(),"expectedRevision":0,"printerId":"test","media":"square"}});
    let accepted = tool_value(
        s.handle(req(
            10,
            "tools/call",
            json!({"name":"print_submit_pipeline","arguments":args}),
        ))
        .await
        .unwrap(),
    );
    assert!(accepted["jobId"].is_string(), "{accepted}");
    assert_eq!(
        tool_value(
            s.handle(req(
                11,
                "tools/call",
                json!({"name":"print_submit_pipeline","arguments":args})
            ))
            .await
            .unwrap()
        ),
        accepted
    );
    assert_eq!(repo.list(100, 0).await.unwrap().len(), 1);
    let mut invalid = args.clone();
    invalid["sourceName"] = json!("../proof.pdf");
    assert_eq!(
        s.handle(req(
            12,
            "tools/call",
            json!({"name":"print_submit_pipeline","arguments":invalid})
        ))
        .await
        .unwrap()["result"]["isError"],
        true
    );
    let mut limited = Session::new(router, TOKEN.into()).with_access(
        "submitter".into(),
        vec!["jobs:submit".into()],
        Some(inbox.path().into()),
    );
    initialize(&mut limited).await;
    let tools = limited
        .handle(req(13, "tools/list", json!({})))
        .await
        .unwrap();
    assert!(
        !tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "print_submit_pipeline")
    );
    assert_eq!(
        limited
            .handle(req(
                14,
                "tools/call",
                json!({"name":"print_submit_pipeline","arguments":args})
            ))
            .await
            .unwrap()["result"]["isError"],
        true
    );
}

#[tokio::test]
async fn inspection_reader_is_scoped_and_owner_bound() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    app.credentials = vec![
        jrip_server::Credential {
            validity: None,
            token: TOKEN.into(),
            identity: "owner".into(),
            scopes: vec!["admin".into()],
        },
        jrip_server::Credential {
            validity: None,
            token: "other".into(),
            identity: "other".into(),
            scopes: vec!["admin".into()],
        },
    ];
    let id = uuid::Uuid::new_v4();
    let report = json!({"inspectionId":id,"sourceSha256":"a".repeat(64),"pages":[{"page":1,"widthPoints":72,"heightPoints":72}]});
    app.repo
        .save_document_inspection(id, "owner", &report)
        .await
        .unwrap();
    let router = jrip_server::router(app);
    let mut owner = Session::new(router.clone(), TOKEN.into());
    initialize(&mut owner).await;
    let args = json!({"inspectionId":id});
    assert_eq!(
        tool_value(
            owner
                .handle(req(
                    10,
                    "tools/call",
                    json!({"name":"print_get_document_inspection","arguments":args})
                ))
                .await
                .unwrap()
        ),
        report
    );
    let mut other = Session::new(router.clone(), "other".into());
    initialize(&mut other).await;
    assert_eq!(
        other
            .handle(req(
                11,
                "tools/call",
                json!({"name":"print_get_document_inspection","arguments":args})
            ))
            .await
            .unwrap()["result"]["isError"],
        true
    );
    let mut reader = Session::new(router, TOKEN.into()).with_access(
        "owner".into(),
        vec!["jobs:read".into()],
        None,
    );
    initialize(&mut reader).await;
    assert_eq!(
        reader
            .handle(req(
                12,
                "tools/call",
                json!({"name":"print_get_document_inspection","arguments":args})
            ))
            .await
            .unwrap()["result"]["isError"],
        true
    );
}

#[tokio::test]
async fn asynchronous_inspection_tools_use_inbox_owner_and_revision_control() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = tempfile::tempdir().unwrap();
    tokio::fs::write(inbox.path().join("proof.pdf"), b"%PDF-1.4\nsource")
        .await
        .unwrap();
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let repo = app.repo.clone();
    let mut session = Session::new(jrip_server::router(app), TOKEN.into()).with_access(
        "legacy-local".into(),
        vec!["admin".into()],
        Some(inbox.path().into()),
    );
    initialize(&mut session).await;
    let id = uuid::Uuid::new_v4();
    let args = json!({"inspectionId":id,"sourceName":"proof.pdf"});
    let accepted = tool_value(
        session
            .handle(req(
                10,
                "tools/call",
                json!({"name":"print_submit_document_inspection","arguments":args}),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(accepted["state"], "QUEUED");
    assert_eq!(
        tool_value(
            session
                .handle(req(
                    11,
                    "tools/call",
                    json!({"name":"print_submit_document_inspection","arguments":args})
                ))
                .await
                .unwrap()
        ),
        accepted
    );
    let status=tool_value(session.handle(req(12,"tools/call",json!({"name":"print_get_document_inspection_request","arguments":{"inspectionId":id}}))).await.unwrap());
    assert_eq!(status["inspectionId"], id.to_string());
    let uri = format!("jrip://inspections/{id}/events");
    assert_eq!(
        session
            .handle(req(20, "resources/subscribe", json!({"uri":uri})))
            .await
            .unwrap()["result"],
        json!({})
    );
    assert_eq!(session.notifications().await[0]["params"]["uri"], uri);
    assert!(session.notifications().await.is_empty());
    let first=tool_value(session.handle(req(21,"tools/call",json!({"name":"print_get_document_inspection_events","arguments":{"inspectionId":id,"limit":1}}))).await.unwrap());
    assert_eq!(first["events"][0]["state"], "QUEUED");
    let cancelled=tool_value(session.handle(req(13,"tools/call",json!({"name":"print_cancel_document_inspection","arguments":{"inspectionId":id,"expectedRevision":0}}))).await.unwrap());
    assert_eq!(cancelled["state"], "CANCELLED");
    assert_eq!(cancelled["revision"], 1);
    assert_eq!(session.notifications().await.len(), 1);
    assert!(session.notifications().await.is_empty());
    let delta=tool_value(session.handle(req(22,"tools/call",json!({"name":"print_get_document_inspection_events","arguments":{"inspectionId":id,"after":first["nextAfter"]}}))).await.unwrap());
    assert_eq!(delta["events"].as_array().unwrap().len(), 1);
    assert_eq!(delta["events"][0]["state"], "CANCELLED");
    let read = session
        .handle(req(23, "resources/read", json!({"uri":uri})))
        .await
        .unwrap();
    let history: Value =
        serde_json::from_str(read["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(history["events"].as_array().unwrap().len(), 2);
    assert_eq!(
        session
            .handle(req(24, "resources/unsubscribe", json!({"uri":uri})))
            .await
            .unwrap()["result"],
        json!({})
    );
    assert_eq!(session.handle(req(14,"tools/call",json!({"name":"print_cancel_document_inspection","arguments":{"inspectionId":id,"expectedRevision":0}}))).await.unwrap()["result"]["isError"],true);
    assert!(repo.document_inspection(id, "legacy-local").await.is_err());
    assert_eq!(session.handle(req(15,"tools/call",json!({"name":"print_submit_document_inspection","arguments":{"inspectionId":uuid::Uuid::new_v4(),"sourceName":"../proof.pdf"}}))).await.unwrap()["result"]["isError"],true);
}

#[tokio::test]
async fn inspection_subscriptions_require_submit_scope_and_owner() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    app.credentials = vec![
        jrip_server::Credential {
            validity: None,
            token: TOKEN.into(),
            identity: "owner".into(),
            scopes: vec!["jobs:submit".into()],
        },
        jrip_server::Credential {
            validity: None,
            token: "other-token".into(),
            identity: "other".into(),
            scopes: vec!["jobs:submit".into()],
        },
        jrip_server::Credential {
            validity: None,
            token: "reader-token".into(),
            identity: "owner".into(),
            scopes: vec!["jobs:read".into()],
        },
    ];
    let id = uuid::Uuid::new_v4();
    app.repo
        .enqueue_inspection(id, "owner", &"a".repeat(64), 1, uuid::Uuid::new_v4())
        .await
        .unwrap();
    let uri = format!("jrip://inspections/{id}/events");
    for (token, actor, scopes, expected) in [
        (TOKEN, "owner", vec!["jobs:submit".into()], None),
        (
            "other-token",
            "other",
            vec!["jobs:submit".into()],
            Some(-32002),
        ),
        (
            "reader-token",
            "owner",
            vec!["jobs:read".into()],
            Some(-32001),
        ),
    ] {
        let mut session = Session::new(jrip_server::router(app.clone()), token.into()).with_access(
            actor.into(),
            scopes,
            None,
        );
        initialize(&mut session).await;
        let result = session
            .handle(req(2, "resources/subscribe", json!({"uri":uri})))
            .await
            .unwrap();
        if let Some(code) = expected {
            assert_eq!(result["error"]["code"], code);
            assert!(session.notifications().await.is_empty());
        } else {
            assert_eq!(result["result"], json!({}));
            assert_eq!(session.notifications().await.len(), 1);
            app.repo
                .set_inspection_state(id, 0, "FAILED", Some("TEST_FAILURE"), None)
                .await
                .unwrap();
            assert_eq!(session.notifications().await.len(), 1);
            let delta=tool_value(session.handle(req(3,"tools/call",json!({"name":"print_get_document_inspection_events","arguments":{"inspectionId":id,"after":1}}))).await.unwrap());
            assert_eq!(delta["events"][0]["reasonCode"], "TEST_FAILURE");
        }
    }
}

#[tokio::test]
async fn stdio_pushes_inspection_updates_with_submit_scope_only() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    app.credentials = vec![jrip_server::Credential {
        validity: None,
        token: TOKEN.into(),
        identity: "owner".into(),
        scopes: vec!["jobs:submit".into()],
    }];
    let id = uuid::Uuid::new_v4();
    app.repo
        .enqueue_inspection(id, "owner", &"a".repeat(64), 1, uuid::Uuid::new_v4())
        .await
        .unwrap();
    let uri = format!("jrip://inspections/{id}/events");
    let (client, server) = tokio::io::duplex(256 * 1024);
    let (read, write) = tokio::io::split(server);
    let session = Session::new(jrip_server::router(app.clone()), TOKEN.into()).with_access(
        "owner".into(),
        vec!["jobs:submit".into()],
        None,
    );
    let task = tokio::spawn(serve(tokio::io::BufReader::new(read), write, session));
    let (read, mut write) = tokio::io::split(client);
    let mut read = tokio::io::BufReader::new(read);
    for message in [
        req(
            1,
            "initialize",
            json!({"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"},"capabilities":{}}),
        ),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        req(2, "resources/subscribe", json!({"uri":uri})),
    ] {
        write
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }
    for revision in 0..2 {
        loop {
            let mut line = String::new();
            tokio::time::timeout(std::time::Duration::from_secs(3), read.read_line(&mut line))
                .await
                .unwrap()
                .unwrap();
            let value: Value = serde_json::from_str(&line).unwrap();
            if value["method"] == "notifications/resources/updated" {
                assert_eq!(value["params"]["uri"], uri);
                break;
            }
        }
        if revision == 0 {
            app.repo
                .set_inspection_state(id, 0, "FAILED", Some("TEST_FAILURE"), None)
                .await
                .unwrap();
        }
    }
    drop(read);
    drop(write);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn receipt_discovery_and_reads_require_read_scope_and_validate_arguments() {
    let (_dir, mut s) = session().await;
    initialize(&mut s).await;
    let reply = s
        .handle(req(
            2,
            "tools/call",
            json!({"name":"print_get_receipt_keys","arguments":{}}),
        ))
        .await
        .unwrap();
    assert_eq!(reply["result"]["isError"], false);
    let keys: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(keys["requiresIndependentKeyPinning"], true);
    assert_eq!(keys["keys"], json!([]));
    for args in [json!({"after":"x&limit=100"}), json!({"limit":101})] {
        let reply = s
            .handle(req(
                3,
                "tools/call",
                json!({"name":"print_get_receipt_keys","arguments":args}),
            ))
            .await
            .unwrap();
        assert!(reply.get("error").is_some());
    }
    for name in ["print_get_receipt", "print_get_result_receipt"] {
        let reply = s
            .handle(req(
                4,
                "tools/call",
                json!({"name":name,"arguments":{"jobId":uuid::Uuid::new_v4().to_string()}}),
            ))
            .await
            .unwrap();
        assert_eq!(reply["result"]["isError"], true);
    }
    let missing=s.handle(req(6,"tools/call",json!({"name":"print_get_delivery_result_receipt","arguments":{"deliveryId":uuid::Uuid::new_v4().to_string()}}))).await.unwrap();
    assert_eq!(missing["result"]["isError"], true);
    let mut denied = s.with_access("submitter".into(), vec!["jobs:submit".into()], None);
    let reply = denied
        .handle(req(
            5,
            "tools/call",
            json!({"name":"print_get_receipt_keys","arguments":{}}),
        ))
        .await
        .unwrap();
    assert!(reply.get("error").is_some() || reply["result"]["isError"] == true);
}

#[tokio::test]
async fn receipt_key_revocation_tool_is_admin_only_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let private =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    app.repo = rip_storage::Repository::open_with_receipt_signer(
        &app.root.join("jobs.sqlite3"),
        Some(rip_storage::receipts::ReceiptSigner::from_pkcs8(private.as_ref()).unwrap()),
    )
    .await
    .unwrap();
    let catalog = app.repo.receipt_keys("", 50).await.unwrap();
    let key = catalog["keys"][0]["keyId"].clone();
    let repo = app.repo.clone();
    let router = jrip_server::router(app);
    let mut reader = Session::new(router.clone(), TOKEN.into()).with_access(
        "reader".into(),
        vec!["jobs:read".into()],
        None,
    );
    initialize(&mut reader).await;
    let denied=reader.handle(req(2,"tools/call",json!({"name":"print_revoke_receipt_key","arguments":{"keyId":key,"reason":"compromised"}}))).await.unwrap();
    assert_eq!(denied["result"]["isError"], true);
    let mut admin = Session::new(router, TOKEN.into()).with_audit(repo.clone());
    initialize(&mut admin).await;
    let invalid = admin
        .handle(req(
            3,
            "tools/call",
            json!({"name":"print_revoke_receipt_key","arguments":{"keyId":key,"reason":"unknown"}}),
        ))
        .await
        .unwrap();
    assert!(invalid.get("error").is_some());
    let reply=admin.handle(req(4,"tools/call",json!({"name":"print_revoke_receipt_key","arguments":{"keyId":key,"reason":"compromised"}}))).await.unwrap();
    assert_eq!(reply["result"]["isError"], false);
    assert_eq!(
        repo.receipt_keys("", 50).await.unwrap()["keys"][0]["revoked"],
        true
    );
    let audit = repo.audit_records(0, 100).await.unwrap();
    assert!(
        audit["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["operation"] == "print_revoke_receipt_key" && r["status"] == 200)
    );
}

#[tokio::test]
async fn source_validation_resource_is_trusted_read_only_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let id = uuid::Uuid::new_v4();
    let run = uuid::Uuid::new_v4();
    let job:rip_core::Job=serde_json::from_value(json!({"id":id,"manifest":{"name":"proof.ps","format":"application/postscript","dpi":72},"state":"QUEUED","source_sha256":"a".repeat(64),"created_at":0,"updated_at":0,"revision":0,"error_code":null,"selected_engine":"ghostscript","worker_run_id":null})).unwrap();
    app.repo.insert(&job).await.unwrap();
    app.repo
        .claim_queued(job, run, rip_storage::now() + 60)
        .await
        .unwrap();
    let report = app
        .repo
        .record_source_validation(id, run, &"a".repeat(64), 100, 1, false)
        .await
        .unwrap();
    let router = jrip_server::router(app);
    let mut reader = Session::new(router.clone(), TOKEN.into()).with_access(
        "reader".into(),
        vec!["jobs:read".into()],
        None,
    );
    initialize(&mut reader).await;
    let uri = format!("jrip://jobs/{id}/source-validation");
    let reply = reader
        .handle(req(5, "resources/read", json!({"uri":uri})))
        .await
        .unwrap();
    let actual: Value =
        serde_json::from_str(reply["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(actual, report);
    let templates = reader
        .handle(req(6, "resources/templates/list", json!({})))
        .await
        .unwrap();
    assert!(
        templates["result"]["resourceTemplates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["uriTemplate"] == "jrip://jobs/{jobId}/source-validation")
    );
    let mut denied = Session::new(router, TOKEN.into()).with_access(
        "writer".into(),
        vec!["jobs:submit".into()],
        None,
    );
    initialize(&mut denied).await;
    assert_eq!(
        denied
            .handle(req(7, "resources/read", json!({"uri":uri})))
            .await
            .unwrap()["error"]["code"],
        -32001
    );
}

#[tokio::test]
#[ignore = "requires Docker, both RIP images and built jrip-worker in the target profile directory"]
async fn mcp_submission_runs_the_mandatory_worker_validation_gate() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let root = app.root.clone();
    let repo = app.repo.clone();
    let inbox = dir.path().join("inbox");
    std::fs::create_dir(&inbox).unwrap();
    std::fs::write(
        inbox.join("proof.pdf"),
        include_bytes!("../../../tests/corpus/colors.pdf"),
    )
    .unwrap();
    std::fs::write(
        inbox.join("bad.ps"),
        b"%!PS-Adobe-3.0\nshowpage\nTHIS_OPERATOR_DOES_NOT_EXIST\n",
    )
    .unwrap();
    let mut session = Session::new(jrip_server::router(app), TOKEN.into()).with_access(
        "agent".into(),
        vec!["admin".into()],
        Some(inbox.clone()),
    );
    initialize(&mut session).await;
    let executable = std::env::current_exe().unwrap();
    let worker = executable
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("jrip-worker{}", std::env::consts::EXE_SUFFIX));
    assert!(
        worker.is_file(),
        "build jrip-server --bin jrip-worker first"
    );
    for (name, format, valid) in [
        ("proof.pdf", "application/pdf", true),
        ("bad.ps", "application/postscript", false),
    ] {
        use sha2::{Digest, Sha256};
        let data = std::fs::read(inbox.join(name)).unwrap();
        let intent = json!({"document":{"uri":format!("https://example.org/{name}"),"mediaType":format,"sha256":format!("{:x}",Sha256::digest(data))},"dpi":{"mode":"required","value":72},"copies":{"mode":"required","value":1},"output":{"destination":"artifact"}});
        let pf = tool_value(
            session
                .handle(req(
                    9,
                    "tools/call",
                    json!({"name":"print_create_preflight","arguments":{"intent":intent}}),
                ))
                .await
                .unwrap(),
        );
        let reply=session.handle(req(10,"tools/call",json!({"name":"rip_submit","arguments":{"preflightId":pf["preflightId"],"preflightDigest":pf["preflightDigest"],"sourceName":name}}))).await.unwrap();
        assert_ne!(reply["result"]["isError"], true, "{reply}");
        let submitted = tool_value(reply);
        let id: uuid::Uuid = submitted["job"]["id"].as_str().unwrap().parse().unwrap();
        let result = tokio::process::Command::new(&worker)
            .arg(id.to_string())
            .env("JRIP_DATA_ROOT", &root)
            .output()
            .await
            .unwrap();
        assert_eq!(
            result.status.success(),
            valid,
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            repo.get(id).await.unwrap().state,
            if valid {
                rip_core::JobState::Completed
            } else {
                rip_core::JobState::Failed
            }
        );
        let resource = session
            .handle(req(
                11,
                "resources/read",
                json!({"uri":format!("jrip://jobs/{id}/source-validation")}),
            ))
            .await
            .unwrap();
        if valid {
            let report: Value =
                serde_json::from_str(resource["result"]["contents"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(report["allPagesInterpreted"], true);
            assert_eq!(report["strictPdfInspection"], true);
            let receipt = tool_value(
                session
                    .handle(req(
                        12,
                        "tools/call",
                        json!({"name":"print_get_result_receipt","arguments":{"jobId":id}}),
                    ))
                    .await
                    .unwrap(),
            );
            assert_eq!(receipt["proof"]["result"]["sourceValidation"], report);
        } else {
            assert_eq!(resource["error"]["code"], -32002);
            assert!(
                !root
                    .join("jobs")
                    .join(id.to_string())
                    .join("raster")
                    .exists()
            );
        }
    }
}

#[tokio::test]
async fn binary_resources_are_bounded_verified_and_scoped() {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), TOKEN.into())
        .await
        .unwrap();
    let job = rip_core::Job {
        id: uuid::Uuid::new_v4(),
        manifest: serde_json::from_value(
            json!({"name":"source.pdf","format":"application/pdf","dpi":300,"color_mode":"rgb"}),
        )
        .unwrap(),
        state: rip_core::JobState::Spooling,
        source_sha256: "a".repeat(64),
        created_at: 0,
        updated_at: 0,
        revision: 0,
        error_code: None,
        selected_engine: None,
        worker_run_id: None,
    };
    app.repo.insert(&job).await.unwrap();
    let data: Vec<u8> = (0..70000).map(|i| (i % 251) as u8).collect();
    let path = app
        .root
        .join("jobs")
        .join(job.id.to_string())
        .join("raster");
    std::fs::create_dir_all(&path).unwrap();
    let file = path.join("page-000001.tiff");
    std::fs::write(&file, &data).unwrap();
    app.repo
        .complete_with_artifacts(
            job.clone(),
            &[rip_core::Artifact {
                page: 1,
                name: "page-000001.tiff".into(),
                media_type: "image/tiff".into(),
                bytes: data.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&data)),
            }],
        )
        .await
        .unwrap();
    let mut s = Session::new(jrip_server::router(app), TOKEN.into());
    initialize(&mut s).await;
    let listing = s
        .handle(req(8, "resources/templates/list", json!({})))
        .await
        .unwrap();
    assert!(listing.to_string().contains("/chunks/{offset}/{length}"));
    let prefix = format!("jrip://jobs/{}/artifacts/page-000001.tiff/chunks", job.id);
    let mut joined = Vec::new();
    let mut etag = None;
    for offset in [0, 32768, 65536] {
        let result = s
            .handle(req(
                2,
                "resources/read",
                json!({"uri":format!("{prefix}/{offset}/32768")}),
            ))
            .await
            .unwrap();
        let chunk = &result["result"]["contents"][0];
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(chunk["blob"].as_str().unwrap())
            .unwrap();
        assert!(bytes.len() <= 32768);
        let metadata = &chunk["_meta"]["org.jrip.artifact-chunk-v1"];
        if let Some(ref etag) = etag {
            assert_eq!(etag, &metadata["etag"]);
        }
        etag = Some(metadata["etag"].clone());
        assert!(
            metadata["contentRange"]
                .as_str()
                .unwrap()
                .ends_with("/70000")
        );
        joined.extend(bytes);
    }
    assert_eq!(joined, data);
    for suffix in [
        "0/0",
        "0/32769",
        "01/1",
        "-1/2",
        "18446744073709551615/1",
        "0/1?x=y",
    ] {
        let reply = s
            .handle(req(
                3,
                "resources/read",
                json!({"uri":format!("{prefix}/{suffix}")}),
            ))
            .await
            .unwrap();
        assert_eq!(reply["error"]["code"], -32602);
    }
    let reply = s
        .handle(req(
            4,
            "resources/read",
            json!({"uri":format!("{prefix}/70000/1")}),
        ))
        .await
        .unwrap();
    assert_eq!(reply["error"]["code"], -32002);
    std::fs::write(&file, vec![0; data.len()]).unwrap();
    let reply = s
        .handle(req(
            5,
            "resources/read",
            json!({"uri":format!("{prefix}/0/1")}),
        ))
        .await
        .unwrap();
    assert_eq!(reply["error"]["code"], -32002);
    let mut reader = s.with_access("reader".into(), vec!["jobs:read".into()], None);
    let reply = reader
        .handle(req(
            6,
            "resources/read",
            json!({"uri":format!("{prefix}/0/1")}),
        ))
        .await
        .unwrap();
    assert_eq!(reply["error"]["code"], -32001);
    let listing = reader
        .handle(req(7, "resources/templates/list", json!({})))
        .await
        .unwrap();
    assert!(!listing.to_string().contains("/chunks/"));
}

#[tokio::test]
async fn deletion_approval_is_reviewable_and_mcp_forwards_exact_grant() {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let mut app = jrip_server::open(dir.path().into(), String::new())
        .await
        .unwrap();
    app.require_delete_approval = true;
    app.require_settings_approval = true;
    let human = "human-admin-token-at-least-24-characters";
    app.credentials = vec![
        jrip_server::Credential {
            validity: None,
            token: TOKEN.into(),
            identity: "agent".into(),
            scopes: vec!["admin".into()],
        },
        jrip_server::Credential {
            validity: None,
            token: human.into(),
            identity: "human".into(),
            scopes: vec!["admin".into()],
        },
    ];
    let job = rip_core::Job {
        id: uuid::Uuid::new_v4(),
        manifest: serde_json::from_value(json!({"name":"test.pdf","format":"application/pdf"}))
            .unwrap(),
        state: rip_core::JobState::Completed,
        source_sha256: "a".repeat(64),
        created_at: 0,
        updated_at: 0,
        revision: 0,
        error_code: None,
        selected_engine: None,
        worker_run_id: None,
    };
    app.repo.insert(&job).await.unwrap();
    let key_doc =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let signer = rip_storage::receipts::ReceiptSigner::from_pkcs8(key_doc.as_ref()).unwrap();
    let key_repo = rip_storage::Repository::open_with_receipt_signer(
        &app.root.join("jobs.sqlite3"),
        Some(signer),
    )
    .await
    .unwrap();
    let keys = key_repo.receipt_keys("", 100).await.unwrap();
    let key = keys["keys"][0]["keyId"].as_str().unwrap();
    let router = jrip_server::router(app.clone());
    let mut session = Session::new(router.clone(), TOKEN.into());
    initialize(&mut session).await;
    let resources = session
        .handle(req(2, "resources/list", json!({})))
        .await
        .unwrap();
    assert_eq!(
        resources["result"]["resources"][0]["uri"],
        "jrip://approval-policy"
    );
    let policy = session
        .handle(req(
            3,
            "resources/read",
            json!({"uri":"jrip://approval-policy"}),
        ))
        .await
        .unwrap();
    let policy: Value =
        serde_json::from_str(policy["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(policy["deleteApprovalRequired"], true);
    let args = json!({"jobId":job.id,"expectedRevision":0});
    let challenge = session
        .handle(req(
            4,
            "tools/call",
            json!({"name":"rip_delete_job","arguments":args}),
        ))
        .await
        .unwrap();
    assert_eq!(challenge["result"]["isError"], true);
    let challenge: Value =
        serde_json::from_str(challenge["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(challenge["error"]["code"], "approval_required");
    let body = json!({"binding":challenge["error"]["approvalRequest"]["binding"],"reason":"human reviewed deletion"});
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/operation-approvals")
                .header("authorization", format!("Bearer {human}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let approval: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 32768).await.unwrap()).unwrap();
    let mut args = args;
    args["approvalId"] = approval["id"].clone();
    let result = session
        .handle(req(
            5,
            "tools/call",
            json!({"name":"rip_delete_job","arguments":args}),
        ))
        .await
        .unwrap();
    assert_eq!(result["result"]["isError"], false, "{result}");
    assert!(app.repo.get(job.id).await.is_err());
    let mut args = json!({"keyId":key,"reason":"retired"});
    let challenge = session
        .handle(req(
            6,
            "tools/call",
            json!({"name":"print_revoke_receipt_key","arguments":args}),
        ))
        .await
        .unwrap();
    assert_eq!(challenge["result"]["isError"], true);
    let challenge: Value =
        serde_json::from_str(challenge["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(challenge["error"]["code"], "approval_required");
    let body = json!({"binding":challenge["error"]["approvalRequest"]["binding"],"reason":"human reviewed key revocation"});
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/operation-approvals")
                .header("authorization", format!("Bearer {human}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let approval: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 32768).await.unwrap()).unwrap();
    args["approvalId"] = approval["id"].clone();
    let result = session
        .handle(req(
            7,
            "tools/call",
            json!({"name":"print_revoke_receipt_key","arguments":args}),
        ))
        .await
        .unwrap();
    assert_eq!(result["result"]["isError"], false, "{result}");
    assert!(key_repo.receipt_key_approval_state(key).await.unwrap()["revokedAt"].is_i64());
}

#[tokio::test]
async fn copy_count_is_validated_and_forwarded_for_delivery_preview_and_pipeline() {
    use axum::{Json, Router, routing::post};
    async fn echo(Json(value): Json<Value>) -> Json<Value> {
        Json(value)
    }
    let router = Router::new()
        .route("/api/v1/jobs/{id}/deliveries", post(echo))
        .route("/api/v1/jobs/{id}/delivery-preview", post(echo))
        .route("/api/v1/jobs/{id}/pipelines", post(echo));
    let mut session = Session::new(router, TOKEN.into());
    initialize(&mut session).await;
    for name in [
        "print_deliver_job",
        "print_preview_delivery",
        "print_create_pipeline",
    ] {
        let mut args = json!({"jobId":uuid::Uuid::new_v4(),"requestId":uuid::Uuid::new_v4(),"expectedRevision":0,"printerId":"proof","media":"iso_a4_210x297mm","copies":3});
        let result = session
            .handle(req(2, "tools/call", json!({"name":name,"arguments":args})))
            .await
            .unwrap();
        assert_eq!(result["result"]["isError"], false, "{result}");
        let payload: Value =
            serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(payload["copies"], 3);
        for count in [json!(0), json!(1000), json!(-1), json!(1.5), json!("2")] {
            args["copies"] = count;
            let reply = session
                .handle(req(3, "tools/call", json!({"name":name,"arguments":args})))
                .await
                .unwrap();
            assert_eq!(reply["error"]["code"], -32602, "{reply}");
        }
        args.as_object_mut().unwrap().remove("copies");
        let result = session
            .handle(req(4, "tools/call", json!({"name":name,"arguments":args})))
            .await
            .unwrap();
        let payload: Value =
            serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(payload["copies"], 1);
    }
}
