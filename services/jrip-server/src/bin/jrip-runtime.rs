//! Foreground single-host process supervisor. Rendering stays in isolated worker processes.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let mut check = false;
    let mut printers = false;
    let mut ipp = true;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--check" => check = true,
            "--with-printers" => printers = true,
            "--without-ipp" => ipp = false,
            "--help" => {
                println!(
                    "jrip-runtime [--check] [--with-printers] [--without-ipp]\nManaged local RIP stack / ローカルRIP一括起動 / 本地RIP统一启动"
                );
                return Ok(());
            }
            _ => {
                return Err(
                    "Unknown runtime option / 起動オプションが不正です / 启动选项无效".into(),
                );
            }
        }
    }
    let mut signals = jrip_server::shutdown::Signals::new()?;
    let config = jrip_server::runtime::Config::from_env(printers, ipp)?;
    let prepared = jrip_server::runtime::prepare(config).await?;
    if check {
        println!("{}", serde_json::to_string(&prepared.summary())?);
        return Ok(());
    }
    jrip_server::runtime::run(prepared, signals.wait()).await
}
