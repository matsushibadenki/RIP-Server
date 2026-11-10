#![cfg(unix)]
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
};
const TOKEN: &str = "runtime-private-token-at-least-24-characters";
fn port() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}
fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
struct Fixture {
    dir: tempfile::TempDir,
    bins: PathBuf,
    root: PathBuf,
    api: SocketAddr,
    ipp: SocketAddr,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bins = dir.path().join("bin");
        let root = dir.path().join("data");
        std::fs::create_dir(&bins).unwrap();
        executable(&bins.join("docker"), "#!/bin/sh\nexit 0\n");
        let exe = std::env::current_exe().unwrap();
        // Shell quoting preserves paths with spaces or apostrophes.
        let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        for name in [
            "jrip-server",
            "jrip-dispatcher",
            "jrip-worker",
            "jrip-color-worker",
            "jrip-inspection-worker",
            "jrip-ipp",
            "jrip-pipeline-worker",
            "jrip-delivery-worker",
            "jrip-printer-observer",
        ] {
            executable(
                &bins.join(name),
                &format!(
                    "#!/bin/sh\nexport JRIP_TEST_SERVICE={name}\nexport JRIP_TEST_JOB=\"$1\"\nexec {} --exact runtime_child --nocapture\n",
                    quote(exe.to_str().unwrap())
                ),
            );
        }
        let api = port();
        let mut ipp = port();
        while ipp == api {
            ipp = port();
        }
        Self {
            dir,
            bins,
            root,
            api,
            ipp,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_jrip-runtime"));
        command
            .env_clear()
            .env("JRIP_API_TOKEN", TOKEN)
            .env("JRIP_DATA_ROOT", &self.root)
            .env("JRIP_BIN_DIR", &self.bins)
            .env("PATH", &self.bins)
            .env("JRIP_LISTEN", self.api.to_string())
            .env("JRIP_IPP_LISTEN", self.ipp.to_string())
            .kill_on_drop(true);
        command
    }
    async fn start(&self) -> Child {
        let mut command = self.command();
        let mut child = command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let line = tokio::time::timeout(Duration::from_secs(10), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["status"], "ready");
        child
    }
    async fn stop(&self, child: &mut Child, signal: &str) {
        let status = Command::new("/bin/kill")
            .args([signal, &child.id().unwrap().to_string()])
            .status()
            .await
            .unwrap();
        assert!(status.success());
        assert!(
            tokio::time::timeout(Duration::from_secs(10), child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        assert!(TcpListener::bind(self.api).is_ok());
        assert!(TcpListener::bind(self.ipp).is_ok());
    }
}
#[test]
fn runtime_child() {
    let Ok(name) = std::env::var("JRIP_TEST_SERVICE") else {
        return;
    };
    let root = PathBuf::from(std::env::var("JRIP_DATA_ROOT").unwrap());
    std::fs::write(
        root.join(format!("{name}.pid")),
        std::process::id().to_string(),
    )
    .unwrap();
    if name == "jrip-worker" && std::env::var("JRIP_TEST_ACTIVE_WORKER").as_deref() == Ok("1") {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let repo = rip_storage::Repository::open(&root.join("jobs.sqlite3"))
                .await
                .unwrap();
            let id = uuid::Uuid::parse_str(&std::env::var("JRIP_TEST_JOB").unwrap()).unwrap();
            let run = uuid::Uuid::parse_str(&std::env::var("JRIP_WORKER_RUN_ID").unwrap()).unwrap();
            let job = repo.get(id).await.unwrap();
            let job = repo
                .claim_queued(job, run, rip_storage::now() + 30)
                .await
                .unwrap();
            std::fs::write(root.join("active.started"), b"started").unwrap();
            tokio::time::sleep(Duration::from_millis(1500)).await;
            repo.transition(
                job,
                rip_core::JobState::Failed,
                Some("FIXTURE_FINISHED".into()),
            )
            .await
            .unwrap();
            std::fs::write(root.join("active.finished"), b"finished").unwrap();
        });
        return;
    }
    if std::env::var("JRIP_TEST_FAIL_SERVICE").as_deref() == Ok(name.as_str()) {
        std::process::exit(7);
    }
    let key = match name.as_str() {
        "jrip-server" => Some("JRIP_LISTEN"),
        "jrip-ipp" => Some("JRIP_IPP_LISTEN"),
        _ => None,
    };
    if let Some(key) = key {
        let listener = TcpListener::bind(std::env::var(key).unwrap()).unwrap();
        for mut socket in listener.incoming().flatten() {
            let mut bytes = [0; 1024];
            let _ = socket.read(&mut bytes);
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
        }
    } else {
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}
#[tokio::test]
async fn preflight_checks_full_file_stack_and_rejects_incomplete_configuration() {
    let fixture = Fixture::new();
    let output = fixture.command().arg("--check").output().await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["physicalDispatch"], false);
    assert_eq!(value["services"].as_array().unwrap().len(), 5);
    assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
    let output = fixture
        .command()
        .args(["--check", "--without-ipp"])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["ippListen"].is_null()
    );
    assert!(
        !fixture
            .command()
            .args(["--check", "--with-printers"])
            .output()
            .await
            .unwrap()
            .status
            .success()
    );
    assert!(
        !fixture
            .command()
            .arg("--check")
            .env("JRIP_API_TOKEN", "short")
            .output()
            .await
            .unwrap()
            .status
            .success()
    );
    std::fs::remove_file(fixture.bins.join("jrip-color-worker")).unwrap();
    assert!(
        !fixture
            .command()
            .arg("--check")
            .output()
            .await
            .unwrap()
            .status
            .success()
    );
    assert!(!fixture.root.join("jrip-server.pid").exists());
    drop(fixture.dir);
}
#[tokio::test]
async fn runtime_lock_signal_shutdown_and_restart_release_children_and_ports() {
    let fixture = Fixture::new();
    let mut child = fixture.start().await;
    assert!(
        !fixture
            .command()
            .arg("--check")
            .output()
            .await
            .unwrap()
            .status
            .success()
    );
    fixture.stop(&mut child, "-TERM").await;
    let mut child = fixture.start().await;
    fixture.stop(&mut child, "-INT").await;
}
#[tokio::test]
async fn child_failure_stops_the_stack_instead_of_leaving_partial_service() {
    let fixture = Fixture::new();
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        fixture
            .command()
            .env("JRIP_TEST_FAIL_SERVICE", "jrip-inspection-worker")
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains(TOKEN));
    assert!(TcpListener::bind(fixture.api).is_ok());
    assert!(TcpListener::bind(fixture.ipp).is_ok());
    let mut child = fixture.start().await;
    fixture.stop(&mut child, "-TERM").await;
}

#[tokio::test]
#[ignore = "requires Docker images, all workspace binaries and Python 3"]
async fn managed_stack_accepts_documents_colors_prints_and_restarts_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("srgb.icc");
    std::fs::write(&profile, lcms2::Profile::new_srgb().icc().unwrap()).unwrap();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("python3")
        .arg("tests/runtime/accept.py")
        .arg("--runtime")
        .arg(env!("CARGO_BIN_EXE_jrip-runtime"))
        .arg("--icc")
        .arg(&profile)
        .current_dir(workspace)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}

#[tokio::test]
async fn supervisor_shutdown_drains_a_running_dispatcher_worker() {
    let fixture = Fixture::new();
    let app = jrip_server::open(fixture.root.clone(), TOKEN.into())
        .await
        .unwrap();
    let manifest = serde_json::from_value(serde_json::json!({"name":"active.pdf","format":"application/pdf","dpi":100,"color_mode":"rgb"})).unwrap();
    let job = rip_jobs::submit_bytes(&app.repo, &app.root, manifest, b"%PDF-1.4\n")
        .await
        .unwrap();
    let worker = fixture
        .bins
        .join("jrip-worker")
        .to_str()
        .unwrap()
        .replace('\'', "'\\''");
    let dispatcher = env!("CARGO_BIN_EXE_jrip-dispatcher").replace('\'', "'\\''");
    executable(
        &fixture.bins.join("jrip-dispatcher"),
        &format!(
            "#!/bin/sh\nexport JRIP_TEST_ACTIVE_WORKER=1\nexport JRIP_WORKER_BIN='{worker}'\nexec '{dispatcher}'\n"
        ),
    );
    let mut child = fixture.start().await;
    let started = tokio::time::timeout(Duration::from_secs(5), async {
        while !fixture.root.join("active.started").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if started.is_err() {
        fixture.stop(&mut child, "-TERM").await;
        panic!(
            "active worker did not start: {}",
            std::fs::read_to_string(fixture.root.join("logs/jrip-dispatcher.log"))
                .unwrap_or_default()
        );
    }
    assert!(!fixture.root.join("active.finished").exists());
    fixture.stop(&mut child, "-TERM").await;
    assert!(
        fixture.root.join("active.finished").exists(),
        "supervisor exited before its active worker finished"
    );
    let result = app.repo.get(job.id).await.unwrap();
    assert_eq!(result.error_code.as_deref(), Some("FIXTURE_FINISHED"));
}
