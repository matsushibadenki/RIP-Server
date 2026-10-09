#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var("JRIP_API_TOKEN").map_err(|_| "Set JRIP_API_TOKEN")?;
    if token.len() < 24 {
        return Err("JRIP_API_TOKEN must have at least 24 characters".into());
    }
    let root = std::env::var("JRIP_DATA_ROOT").unwrap_or_else(|_| "./data".into());
    let app = jrip_server::open(root.into(), token.clone()).await?;
    let (identity, scopes) = app
        .access(&token)
        .ok_or("MCP token is not in the authorization policy")?;
    let input_root = std::env::var("JRIP_INPUT_ROOT")
        .ok()
        .map(std::path::PathBuf::from);
    let audit = app.repo.clone();
    jrip_mcp::serve(
        tokio::io::BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
        jrip_mcp::Session::new(jrip_server::router(app), token)
            .with_access(identity, scopes, input_root)
            .with_audit(audit),
    )
    .await?;
    Ok(())
}
