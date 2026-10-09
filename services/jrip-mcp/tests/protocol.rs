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
    assert_eq!(listing["result"]["tools"].as_array().unwrap().len(), 23);
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
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 11);
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
        let response = session
            .handle(req(
                2,
                "tools/call",
                json!({"name":"print_get_printer","arguments":{"printerId":printer}}),
            ))
            .await
            .unwrap();
        assert_eq!(response["error"]["code"], -32602);
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
