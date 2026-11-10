//! Managed local stack. Optional physical dispatch must be enabled explicitly.
use crate::App;
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
};
type Error = Box<dyn std::error::Error>;
const BASE: &[&str] = &[
    "jrip-server",
    "jrip-dispatcher",
    "jrip-color-worker",
    "jrip-inspection-worker",
];
const PHYSICAL: &[&str] = &[
    "jrip-pipeline-worker",
    "jrip-delivery-worker",
    "jrip-printer-observer",
];
pub struct Config {
    pub root: PathBuf,
    pub binaries: PathBuf,
    pub api: SocketAddr,
    pub ipp: Option<SocketAddr>,
    pub physical: bool,
}
impl Config {
    pub fn from_env(physical: bool, ipp: bool) -> Result<Self, Error> {
        let binaries = std::env::var_os("JRIP_BIN_DIR")
            .map(PathBuf::from)
            .unwrap_or(
                std::env::current_exe()?
                    .parent()
                    .ok_or("missing executable directory")?
                    .into(),
            );
        let parse = |name: &str, default: &str| -> Result<SocketAddr, Error> {
            let value = std::env::var(name).unwrap_or_else(|_| default.to_owned());
            let address:SocketAddr=value.parse().map_err(|_|"Use an IP address and port / IPアドレスとポートを指定してください / 请指定IP地址及端口")?;
            if address.port() == 0 {
                return Err("Runtime needs fixed ports / 一括起動には固定ポートが必要です / 统一启动需要固定端口".into());
            }
            Ok(address)
        };
        let api = parse("JRIP_LISTEN", "127.0.0.1:8080")?;
        let ipp = if ipp {
            Some(parse("JRIP_IPP_LISTEN", "127.0.0.1:8631")?)
        } else {
            None
        };
        if ipp == Some(api) {
            return Err("API and IPP need distinct ports / APIとIPPは別ポートにしてください / API和IPP须使用不同端口".into());
        }
        Ok(Self {
            root: std::env::var_os("JRIP_DATA_ROOT")
                .map(PathBuf::from)
                .unwrap_or("data".into()),
            binaries,
            api,
            ipp,
            physical,
        })
    }
    pub fn services(&self) -> Vec<&'static str> {
        let mut names = BASE.to_vec();
        if self.ipp.is_some() {
            names.push("jrip-ipp");
        }
        if self.physical {
            names.extend(PHYSICAL);
        }
        names
    }
}
pub struct Prepared {
    config: Config,
    services: Vec<&'static str>,
    _lock: File,
}
impl Prepared {
    pub fn summary(&self) -> Value {
        json!({"profile":"org.jrip.local-runtime-v1","services":self.services,"physicalDispatch":self.config.physical,"apiListen":self.config.api.to_string(),"ippListen":self.config.ipp.map(|a|a.to_string()),"dataRoot":self.config.root,"status":"checked"})
    }
}
async fn docker(args: &[&str]) -> Result<(), Error> {
    let status = tokio::time::timeout(
        Duration::from_secs(15),
        Command::new("docker")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await;
    if matches!(status,Ok(Ok(status)) if status.success()) {
        Ok(())
    } else {
        Err("Docker or required images unavailable; run scripts/build-rip-server.sh / Dockerまたは必須イメージを確認しbuildスクリプトを実行してください / 请检查Docker及必需镜像并运行build脚本".into())
    }
}
pub async fn prepare(mut config: Config) -> Result<Prepared, Error> {
    if !cfg!(unix) {
        return Err("Managed runtime currently requires macOS/Linux / 一括起動は現在macOS・Linux向けです / 统一启动当前需要macOS或Linux".into());
    }
    let token = std::env::var("JRIP_API_TOKEN").map_err(
        |_| "Set JRIP_API_TOKEN / JRIP_API_TOKENを設定してください / 请设置JRIP_API_TOKEN",
    )?;
    if token.len() < 24 {
        return Err(
            "Token needs at least 24 characters / tokenは24文字以上 / token至少24个字符".into(),
        );
    }
    let app: App = crate::open(config.root.clone(), token).await?;
    config.root = app.root.clone();
    if config.physical && app.printers.is_empty() {
        return Err("Configure JRIP_PRINTER_CONFIG for physical dispatch / 実機出力にはJRIP_PRINTER_CONFIGが必要です / 实体输出需要JRIP_PRINTER_CONFIG".into());
    }
    if config.physical {
        if std::env::var("JRIP_REQUIRE_PRINT_APPROVAL").as_deref() != Ok("0")
            && app.credentials.is_empty()
        {
            return Err("Set JRIP_AUTH_POLICY for approved physical printing / 承認付き実機出力にはJRIP_AUTH_POLICYを設定してください / 审批实体输出需要JRIP_AUTH_POLICY".into());
        }
        crate::printer_observer::Config::from_env()?;
    }
    if config.ipp.is_some_and(|a| !a.ip().is_loopback())
        && std::env::var("JRIP_ALLOW_INSECURE_LAN").as_deref() != Ok("1")
    {
        return Err("LAN IPP requires explicit JRIP_ALLOW_INSECURE_LAN=1 / LAN IPPは明示許可が必要です / LAN IPP需要明确许可".into());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(config.root.join("runtime.lock"))?;
    lock.try_lock().map_err(|_|"Another runtime owns this root / 同じデータrootで起動済みです / 此数据目录已有运行实例")?;
    config.binaries = std::fs::canonicalize(&config.binaries)?;
    let services = config.services();
    for name in services
        .iter()
        .copied()
        .chain(std::iter::once("jrip-worker"))
    {
        let path = config
            .binaries
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        let metadata=std::fs::metadata(&path).map_err(|_|format!("Missing binary {name}; run scripts/build-rip-server.sh / 必須実行ファイルがありません / 缺少必需程序"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(format!(
                    "Binary {name} is not executable / 実行権限がありません / 程序无执行权限"
                )
                .into());
            }
        }
        if !metadata.is_file() {
            return Err(
                "Runtime binary is not a file / 実行ファイルが不正です / 程序文件无效".into(),
            );
        }
    }
    let mut ports = vec![std::net::TcpListener::bind(config.api)?];
    if let Some(address) = config.ipp {
        ports.push(std::net::TcpListener::bind(address)?);
    }
    docker(&["info"]).await?;
    docker(&[
        "image",
        "inspect",
        "jrip-mupdf:local",
        "jrip-ghostscript:local",
        "jrip-color:local",
    ])
    .await?;
    drop(ports);
    Ok(Prepared {
        config,
        services,
        _lock: lock,
    })
}
struct Service {
    name: &'static str,
    child: Child,
    exited: bool,
}
fn local(mut address: SocketAddr) -> SocketAddr {
    if address.ip().is_unspecified() {
        address.set_ip(match address.ip() {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        });
    }
    address
}
async fn ready(address: SocketAddr, path: &str) -> bool {
    let request = async {
        let mut socket = tokio::net::TcpStream::connect(local(address)).await?;
        socket
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await?;
        let mut bytes = [0; 12];
        socket.read_exact(&mut bytes).await?;
        Ok::<_, std::io::Error>(&bytes == b"HTTP/1.1 200")
    };
    matches!(
        tokio::time::timeout(Duration::from_secs(2), request).await,
        Ok(Ok(true))
    )
}
fn exited(services: &mut [Service]) -> Result<Option<&'static str>, Error> {
    for service in services {
        if let Some(status) = service.child.try_wait()? {
            service.exited = true;
            tracing::error!(service = service.name, ?status, "runtime child exited");
            return Ok(Some(service.name));
        }
    }
    Ok(None)
}
async fn stop(services: &mut [Service]) {
    // API SSE clients may keep HTTP graceful shutdown alive; workers retain time to
    // finish bounded active work. Never SIGKILL an active interpreter/color pass.
    for service in services.iter_mut() {
        if !service.exited
            && service.child.try_wait().ok().flatten().is_none()
            && let Some(id) = service.child.id()
        {
            let _ = Command::new("/bin/kill")
                .args(["-INT", &id.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
    }
    for service in services.iter_mut().filter(|s| s.name == "jrip-server") {
        if tokio::time::timeout(Duration::from_secs(5), service.child.wait())
            .await
            .is_err()
        {
            let _ = service.child.kill().await;
        }
    }
    for service in services {
        let _ = service.child.wait().await;
    }
}
pub async fn run(
    prepared: Prepared,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<(), Error> {
    tokio::pin!(shutdown);
    // Consume a stop requested during preflight before any processing service starts.
    tokio::select! {biased; _=&mut shutdown=>return Ok(()), _=std::future::ready(())=>{}}
    let logs = prepared.config.root.join("logs");
    std::fs::create_dir_all(&logs)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&logs, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut services = Vec::new();
    let started = async {
        for name in &prepared.services {
            let mut options = OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let log = options.open(logs.join(format!("{name}.log")))?;
            let mut command = Command::new(prepared.config.binaries.join(name));
            command
                .env("JRIP_DATA_ROOT", &prepared.config.root)
                .env("JRIP_LISTEN", prepared.config.api.to_string())
                .env(
                    "JRIP_REQUIRE_PRINT_APPROVAL",
                    std::env::var("JRIP_REQUIRE_PRINT_APPROVAL").unwrap_or_else(|_| "1".into()),
                )
                .stdin(Stdio::null())
                .stdout(log.try_clone()?)
                .stderr(log);
            if let Some(address) = prepared.config.ipp {
                command.env("JRIP_IPP_LISTEN", address.to_string()).env(
                    "JRIP_PRINTER_URI",
                    std::env::var("JRIP_PRINTER_URI")
                        .unwrap_or_else(|_| format!("ipp://{}/ipp/print", local(address))),
                );
            }
            #[cfg(unix)]
            command.process_group(0);
            let child = command.spawn()?;
            tracing::info!(service = *name, pid = child.id(), "runtime child started");
            services.push(Service {
                name,
                child,
                exited: false,
            });
        }
        Ok::<_, Error>(())
    }
    .await;
    if let Err(error) = started {
        stop(&mut services).await;
        return Err(error);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut announced = false;
    let result = loop {
        match exited(&mut services) {
            Ok(Some(name)) => break Err(format!("Runtime service {name} exited; inspect logs / プロセスが終了しました。ログを確認してください / 进程已退出，请检查日志").into()),
            Err(error) => break Err(error),
            Ok(None) => {}
        }
        if !announced {
            if ready(prepared.config.api, "/health").await
                && match prepared.config.ipp {
                    Some(address) => ready(address, "/").await,
                    None => true,
                }
            {
                announced = true;
                println!(
                    "{}",
                    json!({"status":"ready","profile":"org.jrip.local-runtime-v1","services":prepared.services,"physicalDispatch":prepared.config.physical,"apiListen":prepared.config.api.to_string(),"ippListen":prepared.config.ipp.map(|a|a.to_string())})
                );
            } else if tokio::time::Instant::now() >= deadline {
                break Err("Runtime readiness timed out; inspect logs / 起動確認がタイムアウトしました / 启动检查超时".into());
            }
        }
        tokio::select! {_= &mut shutdown=>break Ok(()),_=tokio::time::sleep(Duration::from_millis(250))=>{}}
    };
    stop(&mut services).await;
    tracing::info!("runtime stopped");
    result
}
