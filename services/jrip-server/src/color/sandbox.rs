use serde_json::Value;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};
pub struct Output {
    pub raster: PathBuf,
    pub report: Value,
}
pub fn arguments(
    input: &Path,
    name: &str,
    attempt: uuid::Uuid,
    dpi: u32,
    user: &str,
) -> Result<Vec<String>, String> {
    let input = input.to_str().ok_or("invalid_color_path")?;
    if input.contains([',', ':', '\n']) || !(72..=2400).contains(&dpi) {
        return Err("invalid_color_path".into());
    }
    Ok(vec![
        "run".into(),
        "--pull=never".into(),
        "--name".into(),
        name.into(),
        "--label=org.jrip.color=1".into(),
        format!("--label=org.jrip.color-attempt={attempt}"),
        "--network=none".into(),
        "--read-only".into(),
        "--cap-drop=ALL".into(),
        "--security-opt=no-new-privileges".into(),
        "--memory=384m".into(),
        "--memory-swap=384m".into(),
        "--cpus=1".into(),
        "--pids-limit=32".into(),
        "--ulimit".into(),
        "fsize=73400320:73400320".into(),
        "--user".into(),
        user.into(),
        "--tmpfs".into(),
        "/tmp:rw,noexec,nosuid,size=8388608,mode=1777".into(),
        "--tmpfs".into(),
        "/output:rw,noexec,nosuid,size=83886080,nr_inodes=64,mode=1777".into(),
        "--mount".into(),
        format!("type=bind,src={input},dst=/input,readonly"),
        "jrip-color:local".into(),
        dpi.to_string(),
    ])
}
async fn remove(name: &str) -> Result<(), String> {
    let status = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("docker")
            .args(["rm", "-f", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await
    .map_err(|_| "color_cleanup_timeout")?
    .map_err(|_| "color_cleanup_failed")?;
    if status.success() {
        Ok(())
    } else {
        Err("color_cleanup_failed".into())
    }
}
pub async fn cleanup_attempt(attempt: uuid::Uuid) -> Result<(), String> {
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("docker")
            .args([
                "ps",
                "-aq",
                "--no-trunc",
                "--filter",
                "label=org.jrip.color=1",
                "--filter",
                &format!("label=org.jrip.color-attempt={attempt}"),
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "color_cleanup_timeout")?
    .map_err(|_| "color_cleanup_failed")?;
    if !output.status.success() || output.stdout.len() > 65536 {
        return Err("color_cleanup_failed".into());
    }
    let ids = std::str::from_utf8(&output.stdout).map_err(|_| "color_cleanup_failed")?;
    for id in ids.lines() {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("color_cleanup_failed".into());
        }
        remove(id).await?;
    }
    Ok(())
}
pub async fn convert(
    input: &Path,
    output: &Path,
    attempt: uuid::Uuid,
    dpi: u32,
) -> Result<Output, String> {
    let input = std::fs::canonicalize(input).map_err(|_| "invalid_color_path")?;
    #[cfg(unix)]
    let user = {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(&input).map_err(|_| "invalid_color_path")?;
        format!("{}:{}", m.uid(), m.gid())
    };
    #[cfg(not(unix))]
    let user = "1000:1000".to_string();
    let name = format!("jrip-color-{}", uuid::Uuid::new_v4());
    let mut child = Command::new("docker")
        .args(arguments(&input, &name, attempt, dpi, &user)?)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "color_worker_unavailable")?;
    let archive = output.join("color.tar");
    let result = tokio::time::timeout(Duration::from_secs(120), async {
        let mut stdout = child
            .stdout
            .take()
            .ok_or("color_worker_output")?
            .take(67 * 1024 * 1024 + 1);
        let mut file = tokio::fs::File::create(&archive)
            .await
            .map_err(|_| "color_worker_output")?;
        let bytes = tokio::io::copy(&mut stdout, &mut file)
            .await
            .map_err(|_| "color_worker_output")?;
        file.flush().await.map_err(|_| "color_worker_output")?;
        if bytes > 67 * 1024 * 1024 {
            return Err("color_output_limit");
        }
        let status = child.wait().await.map_err(|_| "color_worker_failed")?;
        if !status.success() {
            return Err("color_transform_failed");
        }
        Ok(())
    })
    .await;
    if !matches!(&result, Ok(Ok(()))) {
        let _ = child.kill().await;
    }
    remove(&name).await?;
    result
        .map_err(|_| "color_worker_timeout")?
        .map_err(str::to_string)?;
    let output = output.to_path_buf();
    tokio::task::spawn_blocking(move || extract(&archive, &output))
        .await
        .map_err(|_| "color_output_invalid".to_string())?
}
fn extract(archive: &Path, output: &Path) -> Result<Output, String> {
    let mut tar =
        tar::Archive::new(std::fs::File::open(archive).map_err(|_| "color_output_invalid")?);
    let mut report = None;
    let mut raster = None;
    for entry in tar.entries().map_err(|_| "color_output_invalid")? {
        let mut entry = entry.map_err(|_| "color_output_invalid")?;
        if !entry.header().entry_type().is_file() {
            return Err("color_output_invalid".into());
        }
        let path = entry
            .path()
            .map_err(|_| "color_output_invalid")?
            .into_owned();
        if path == Path::new("report.json") && report.is_none() && entry.size() <= 16384 {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|_| "color_output_invalid")?;
            report = Some(serde_json::from_slice(&bytes).map_err(|_| "color_output_invalid")?);
        } else if path == Path::new("result.tiff")
            && raster.is_none()
            && (8..=65 * 1024 * 1024).contains(&entry.size())
        {
            let destination = output.join("result.tiff");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
                .map_err(|_| "color_output_invalid")?;
            std::io::copy(&mut entry, &mut file).map_err(|_| "color_output_invalid")?;
            file.flush().map_err(|_| "color_output_invalid")?;
            file.sync_all().map_err(|_| "color_output_invalid")?;
            raster = Some(destination);
        } else {
            return Err("color_output_invalid".into());
        }
    }
    std::fs::remove_file(archive).map_err(|_| "color_output_invalid")?;
    Ok(Output {
        raster: raster.ok_or("color_output_invalid")?,
        report: report.ok_or("color_output_invalid")?,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sandbox_has_no_network_or_host_output_mount() {
        let args = arguments(
            Path::new("/tmp/input"),
            "test",
            uuid::Uuid::nil(),
            300,
            "1000:1000",
        )
        .unwrap();
        for required in [
            "--network=none",
            "--read-only",
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--memory=384m",
        ] {
            assert!(args.iter().any(|a| a == required));
        }
        assert!(args.iter().any(|a| a.contains("dst=/input,readonly")));
        assert!(arguments(Path::new("/tmp/a,b"), "test", uuid::Uuid::nil(), 300, "1:1").is_err());
    }
}
