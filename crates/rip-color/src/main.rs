use std::{io::Read, path::PathBuf};
fn bounded(path: &PathBuf, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::other("input limit exceeded"));
    }
    Ok(bytes)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if args.len() == 3 && args[0].as_os_str() == "register" {
        let registry = rip_color::registry::Registry::open(&args[1])?;
        let record = registry.register(&bounded(&args[2], 4 * 1024 * 1024)?)?;
        println!("{}", serde_json::to_string_pretty(&record)?);
        return Ok(());
    }
    if args.len() == 3 && args[0].as_os_str() == "describe" {
        let registry = rip_color::registry::Registry::open(&args[1])?;
        let record = registry.describe(args[2].to_str().ok_or("invalid digest")?)?;
        println!("{}", serde_json::to_string_pretty(&record)?);
        return Ok(());
    }
    if args.len() != 6 {
        return Err("Usage: rip-color <input.tiff> <input.icc> <output.icc|-> <policy.json> <dpi> <output.tiff>".into());
    }
    let source = bounded(&args[0], 64 * 1024 * 1024)?;
    let input = bounded(&args[1], 4 * 1024 * 1024)?;
    let output = if args[2].as_os_str() == "-" {
        None
    } else {
        Some(bounded(&args[2], 4 * 1024 * 1024)?)
    };
    let policy: rip_color::Policy = serde_json::from_slice(&bounded(&args[3], 16384)?)?;
    let dpi = args[4].to_str().ok_or("invalid dpi")?.parse()?;
    let parent = args[5]
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let mut staging = tempfile::NamedTempFile::new_in(parent)?;
    let report = rip_color::raster::convert_tiff(
        std::io::Cursor::new(source),
        staging.as_file_mut(),
        &input,
        output.as_deref(),
        &policy,
        dpi,
    )?;
    staging.as_file().sync_all()?;
    staging.persist_noclobber(&args[5])?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
