//! Run explicitly alongside the API/MCP process with the same data root and printer configuration.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] {
        println!(
            "jrip-printer-observer [--once]\nObserve configured printers without printing / 設定済み機器を印刷せず観測 / 观察已配置设备，不执行打印"
        );
        return Ok(());
    }
    if !(args.is_empty() || args == ["--once"]) {
        return Err(
            "Use --once or no arguments / 引数は--onceまたは省略 / 参数须为--once或省略".into(),
        );
    }
    let config = jrip_server::printer_observer::Config::from_env()?;
    let token = std::env::var("JRIP_API_TOKEN").map_err(
        |_| "Set JRIP_API_TOKEN / JRIP_API_TOKENを設定してください / 请设置JRIP_API_TOKEN",
    )?;
    if token.len() < 24 {
        return Err(
            "JRIP_API_TOKEN needs 24 characters / tokenは24文字以上 / token至少24个字符".into(),
        );
    }
    let root = std::env::var("JRIP_DATA_ROOT").unwrap_or_else(|_| "data".into());
    let app = jrip_server::open(root.into(), token).await?;
    if app.printers.is_empty() {
        return Err("Configure JRIP_PRINTER_CONFIG first / JRIP_PRINTER_CONFIGで機器を設定してください / 请先通过JRIP_PRINTER_CONFIG配置设备".into());
    }
    if args == ["--once"] {
        let report = jrip_server::printer_observer::observe_once(&app, config).await;
        println!("{}", serde_json::to_string(&report)?);
        if report.failed > 0 || report.storage_errors > 0 || report.lease_lost > 0 {
            return Err("Observation incomplete / 観測が完了しませんでした / 观察未完成".into());
        }
    } else {
        tracing::info!(printers=app.printers.len(),configuration=?config,"printer observer started");
        jrip_server::printer_observer::run_until(&app, config, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    }
    Ok(())
}
