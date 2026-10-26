mod page_stream;
use page_stream::read_page_stream;
use rip_core::{ColorMode, DocumentFormat, EngineKind, Manifest};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

#[derive(Debug, thiserror::Error)]
pub enum InterpreterError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Rejected(&'static str),
}
pub struct RenderedDocument {
    pub pages: Vec<PathBuf>,
}
pub trait DocumentInterpreter: Send + Sync {
    fn render(
        &self,
        source: &Path,
        output: &Path,
        manifest: &Manifest,
    ) -> impl std::future::Future<Output = Result<RenderedDocument, InterpreterError>> + Send;
}
/// All interpretation runs in a resource-limited, networkless Linux container.
/// Docker access belongs only to the trusted worker, never the HTTP process.
pub struct SandboxConfig {
    pub image: String,
    pub timeout: Duration,
    pub max_pages: u32,
    pub max_spool_bytes: u64,
}
impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            image: "jrip-ghostscript:local".into(),
            timeout: Duration::from_secs(120),
            max_pages: 100,
            max_spool_bytes: 1024 * 1024 * 1024,
        }
    }
}
impl SandboxConfig {
    pub fn arguments(
        &self,
        engine: EngineKind,
        source: &Path,
        output: &Path,
        manifest: &Manifest,
        name: &str,
        user: &str,
    ) -> Result<Vec<String>, InterpreterError> {
        manifest
            .validate()
            .map_err(|_| InterpreterError::Rejected("invalid_manifest"))?;
        if self.max_pages == 0
            || self.max_pages > 10000
            || self.max_spool_bytes < 1024 * 1024
            || self.max_spool_bytes > u64::MAX - 1024 * 1024 - 2
        {
            return Err(InterpreterError::Rejected("invalid_limits"));
        }
        let source = source
            .to_str()
            .ok_or(InterpreterError::Rejected("invalid_path"))?;
        let output = output
            .to_str()
            .ok_or(InterpreterError::Rejected("invalid_path"))?;
        if source.contains([',', ':', '\n']) || output.contains([',', ':', '\n']) {
            return Err(InterpreterError::Rejected("invalid_mount_path"));
        }
        if engine == EngineKind::MuPdf && manifest.format != DocumentFormat::Pdf {
            return Err(InterpreterError::Rejected("engine_format_mismatch"));
        }
        let device = match manifest.color_mode {
            ColorMode::Rgb => "tiff24nc",
            ColorMode::Cmyk => "tiff32nc",
            ColorMode::Gray => "tiffgray",
        };
        let mut args = vec![
            "run".into(),
            "--pull=never".into(),
            "--name".into(),
            name.into(),
            "--label=org.jrip.interpreter=1".into(),
            format!("--label=org.jrip.source={source}"),
            "--network=none".into(),
            "--read-only".into(),
            "--cap-drop=ALL".into(),
            "--security-opt=no-new-privileges".into(),
            "--memory=512m".into(),
            "--memory-swap=512m".into(),
            "--cpus=1".into(),
            "--pids-limit=64".into(),
            "--ulimit".into(),
            format!(
                "fsize={0}:{0}",
                self.max_spool_bytes / u64::from(self.max_pages + 1)
            ),
            "--user".into(),
            user.into(),
            "--tmpfs".into(),
            "/tmp:rw,noexec,nosuid,size=67108864".into(),
            "--mount".into(),
            format!("type=bind,src={source},dst=/input/document,readonly"),
            "--tmpfs".into(),
            format!(
                "/output:rw,noexec,nosuid,size={},nr_inodes=4096,mode=1777",
                self.max_spool_bytes
            ),
            self.image.clone(),
        ];
        match engine {
            EngineKind::Ghostscript => args.extend([
                "-dSAFER".into(),
                "-dBATCH".into(),
                "-dNOPAUSE".into(),
                "-dQUIET".into(),
                "-sstdout=%stderr".into(),
                "-dNumRenderingThreads=1".into(),
                "-dMaxBitmap=16777216".into(),
                "-dBufferSpace=8388608".into(),
                format!("-sDEVICE={device}"),
                "-sCompression=lzw".into(),
                format!("-r{}", manifest.dpi),
                format!("-dLastPage={}", self.max_pages + 1),
                "-sOutputFile=/output/page-%06d.tiff".into(),
                "-f".into(),
                "/input/document".into(),
            ]),
            EngineKind::MuPdf => args.extend([
                manifest.dpi.to_string(),
                match manifest.color_mode {
                    ColorMode::Rgb => "rgb",
                    ColorMode::Cmyk => "cmyk",
                    ColorMode::Gray => "gray",
                }
                .into(),
                (self.max_pages + 1).to_string(),
            ]),
        }
        Ok(args)
    }
}

impl SandboxConfig {
    async fn render<F, Fut>(
        &self,
        engine: EngineKind,
        source: &Path,
        output: &Path,
        manifest: &Manifest,
        mut progress: F,
    ) -> Result<RenderedDocument, InterpreterError>
    where
        F: FnMut(u32) -> Fut + Send,
        Fut: std::future::Future<Output = Result<(), InterpreterError>> + Send,
    {
        let source = tokio::fs::canonicalize(source).await?;
        // Caller must provide a fresh directory. Refuse to mix old and new rasters.
        tokio::fs::create_dir(output).await?;
        let output = tokio::fs::canonicalize(output).await?;
        #[cfg(unix)]
        let user = {
            use std::os::unix::fs::MetadataExt;
            let meta = std::fs::metadata(&output)?;
            format!("{}:{}", meta.uid(), meta.gid())
        };
        #[cfg(not(unix))]
        let user = "1000:1000".to_string();
        let name = format!("jrip-{}", uuid::Uuid::new_v4());
        let args = self.arguments(engine, &source, &output, manifest, &name, &user)?;
        let mut child = Command::new("docker")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let result = tokio::time::timeout(self.timeout, async {
            let mut stdout = child
                .stdout
                .take()
                .ok_or(InterpreterError::Rejected("missing_output"))?;
            let pages = read_page_stream(
                &mut stdout,
                &output,
                self.max_pages,
                self.max_spool_bytes,
                &mut progress,
            )
            .await?;
            let status = child.wait().await?;
            if !status.success() {
                return Err(InterpreterError::Rejected("interpreter_failed"));
            }
            Ok::<_, InterpreterError>(RenderedDocument { pages })
        })
        .await;
        if !matches!(&result, Ok(Ok(_))) {
            let _ = child.kill().await;
        }
        // Explicitly remove the container: killing the Docker client does not stop the interpreter.
        let cleanup = tokio::time::timeout(
            Duration::from_secs(10),
            Command::new("docker")
                .args(["rm", "-f", &name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status(),
        )
        .await;
        if !matches!(cleanup,Ok(Ok(status)) if status.success()) {
            return Err(InterpreterError::Rejected("container_cleanup_failed"));
        }
        result.map_err(|_| InterpreterError::Rejected("resource_limit_timeout"))?
    }
}
#[derive(Default)]
pub struct GhostscriptInterpreter {
    pub sandbox: SandboxConfig,
}
pub struct MuPdfInterpreter {
    pub sandbox: SandboxConfig,
}
impl Default for MuPdfInterpreter {
    fn default() -> Self {
        Self {
            sandbox: SandboxConfig {
                image: "jrip-mupdf:local".into(),
                ..Default::default()
            },
        }
    }
}
impl DocumentInterpreter for GhostscriptInterpreter {
    async fn render(
        &self,
        source: &Path,
        output: &Path,
        manifest: &Manifest,
    ) -> Result<RenderedDocument, InterpreterError> {
        self.sandbox
            .render(
                EngineKind::Ghostscript,
                source,
                output,
                manifest,
                |_| async { Ok(()) },
            )
            .await
    }
}
impl DocumentInterpreter for MuPdfInterpreter {
    async fn render(
        &self,
        source: &Path,
        output: &Path,
        manifest: &Manifest,
    ) -> Result<RenderedDocument, InterpreterError> {
        self.sandbox
            .render(EngineKind::MuPdf, source, output, manifest, |_| async {
                Ok(())
            })
            .await
    }
}
#[derive(Default)]
pub struct HybridInterpreter {
    pub ghostscript: GhostscriptInterpreter,
    pub mupdf: MuPdfInterpreter,
}
impl HybridInterpreter {
    /// Observations of complete TIFF page frames, while the engine may still be running.
    /// Callback failure terminates the bounded render and removes its container.
    pub async fn render_with_progress<F, Fut>(
        &self,
        source: &Path,
        output: &Path,
        manifest: &Manifest,
        progress: F,
    ) -> Result<RenderedDocument, InterpreterError>
    where
        F: FnMut(u32) -> Fut + Send,
        Fut: std::future::Future<Output = Result<(), InterpreterError>> + Send,
    {
        let engine = manifest
            .engine
            .resolve(manifest.format)
            .map_err(|e| InterpreterError::Rejected(e.0))?;
        let sandbox = match engine {
            EngineKind::MuPdf => &self.mupdf.sandbox,
            EngineKind::Ghostscript => &self.ghostscript.sandbox,
        };
        sandbox
            .render(engine, source, output, manifest, progress)
            .await
    }
}
impl DocumentInterpreter for HybridInterpreter {
    async fn render(
        &self,
        source: &Path,
        output: &Path,
        manifest: &Manifest,
    ) -> Result<RenderedDocument, InterpreterError> {
        let engine = manifest
            .engine
            .resolve(manifest.format)
            .map_err(|e| InterpreterError::Rejected(e.0))?;
        match engine {
            EngineKind::MuPdf => self.mupdf.render(source, output, manifest).await,
            EngineKind::Ghostscript => self.ghostscript.render(source, output, manifest).await,
        }
    }
}
/// Remove only this interpreter's containers for a source whose worker lease has failed.
pub async fn cleanup_abandoned_source(source: &Path) -> Result<(), InterpreterError> {
    let source = tokio::fs::canonicalize(source).await?;
    cleanup_one_source(&source).await?;
    let snapshot = source
        .parent()
        .ok_or(InterpreterError::Rejected("invalid_path"))?
        .join("source-validation");
    match tokio::fs::symlink_metadata(&snapshot).await {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            let nested = snapshot.join("document");
            if tokio::fs::try_exists(&nested).await? {
                let metadata = tokio::fs::symlink_metadata(&nested).await?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(InterpreterError::Rejected("invalid_validation_snapshot"));
                }
                cleanup_one_source(&nested).await?;
            }
            tokio::fs::remove_dir_all(snapshot).await?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => return Err(InterpreterError::Rejected("invalid_validation_snapshot")),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
async fn cleanup_one_source(source: &Path) -> Result<(), InterpreterError> {
    let source = tokio::fs::canonicalize(source).await?;
    let source = source
        .to_str()
        .ok_or(InterpreterError::Rejected("invalid_path"))?;
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("docker")
            .args([
                "ps",
                "-aq",
                "--no-trunc",
                "--filter",
                "label=org.jrip.interpreter=1",
                "--filter",
                &format!("label=org.jrip.source={source}"),
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| InterpreterError::Rejected("cleanup_timeout"))??;
    if !output.status.success() || output.stdout.len() > 65536 {
        return Err(InterpreterError::Rejected("container_cleanup_failed"));
    }
    let ids = std::str::from_utf8(&output.stdout)
        .map_err(|_| InterpreterError::Rejected("invalid_container_ids"))?;
    for id in ids.lines() {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(InterpreterError::Rejected("invalid_container_ids"));
        }
        let status = tokio::time::timeout(
            Duration::from_secs(10),
            Command::new("docker")
                .args(["rm", "-f", id])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status(),
        )
        .await
        .map_err(|_| InterpreterError::Rejected("cleanup_timeout"))??;
        if !status.success() {
            return Err(InterpreterError::Rejected("container_cleanup_failed"));
        }
    }
    Ok(())
}

/// Trusted CLI/worker only; the HTTP server never calls this method.
pub async fn inspect_pdf(source: &Path) -> Result<serde_json::Value, InterpreterError> {
    inspect_pdf_attempt(source, uuid::Uuid::new_v4()).await
}
pub async fn inspect_pdf_attempt(
    source: &Path,
    id: uuid::Uuid,
) -> Result<serde_json::Value, InterpreterError> {
    inspect_pdf_attempt_with_image(source, id, "jrip-mupdf:local").await
}
pub async fn inspect_pdf_attempt_with_image(
    source: &Path,
    id: uuid::Uuid,
    image: &str,
) -> Result<serde_json::Value, InterpreterError> {
    inspect_script(source, id, image, "inspect.py", true).await
}
/// Passive PDF/PS declarations only; uses the same isolated read-only source mount.
pub async fn inspect_source_profiles(
    source: &Path,
    id: uuid::Uuid,
    image: &str,
) -> Result<serde_json::Value, InterpreterError> {
    inspect_script(source, id, image, "source_profiles.py", false).await
}
async fn inspect_script(
    source: &Path,
    id: uuid::Uuid,
    image: &str,
    script: &str,
    strict_pdf: bool,
) -> Result<serde_json::Value, InterpreterError> {
    let source = tokio::fs::canonicalize(source).await?;
    let config = SandboxConfig {
        image: image.into(),
        timeout: Duration::from_secs(120),
        max_pages: 1,
        max_spool_bytes: 1024 * 1024,
    };
    let manifest = Manifest {
        name: "inspection".into(),
        format: DocumentFormat::Pdf,
        dpi: 72,
        color_mode: ColorMode::Rgb,
        priority: 50,
        engine: rip_core::EnginePreference::MuPdf,
    };
    let name = format!("jrip-inspection-{id}");
    let mut args = config.arguments(
        EngineKind::MuPdf,
        &source,
        source
            .parent()
            .ok_or(InterpreterError::Rejected("invalid_path"))?,
        &manifest,
        &name,
        "1000:1000",
    )?;
    args.truncate(args.len() - 3);
    let index = args.len() - 1;
    args.splice(index..index, ["--entrypoint".into(), "python3".into()]);
    args.extend(["-I".into(), format!("/usr/local/lib/jrip/{script}")]);
    let mut child = Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let result = tokio::time::timeout(config.timeout, async {
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or(InterpreterError::Rejected("missing_output"))?
            .take(65537)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > 65536 {
            return Err(InterpreterError::Rejected("inspection_output_limit"));
        }
        if !child.wait().await?.success() {
            return Err(InterpreterError::Rejected(if strict_pdf {
                "pdf_inspection_failed"
            } else {
                "source_profile_analysis_failed"
            }));
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| InterpreterError::Rejected("invalid_inspection"))?;
        if strict_pdf {
            if value["schemaVersion"] != 1
                || value["contentParsed"] != true
                || value["repaired"] != false
                || value["encrypted"] != false
                || !value["pages"]
                    .as_array()
                    .is_some_and(|pages| !pages.is_empty() && pages.len() <= 100)
            {
                return Err(InterpreterError::Rejected("invalid_inspection"));
            }
        } else if value["schemaVersion"] != 1
            || value["profile"] != "org.jrip.source-color-profiles-v1"
            || value["automaticSelection"] != false
            || value["outputIntentIsInputProfile"] != false
            || !matches!(
                value["status"].as_str(),
                Some("DECLARATIONS_INSPECTED" | "INCOMPLETE")
            )
            || !value["profiles"]
                .as_array()
                .is_some_and(|profiles| profiles.len() <= 64)
        {
            return Err(InterpreterError::Rejected(
                "invalid_source_profile_analysis",
            ));
        }
        Ok(value)
    })
    .await;
    if !matches!(result, Ok(Ok(_))) {
        let _ = child.kill().await;
    }
    let cleanup = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("docker")
            .args(["rm", "-f", &name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await;
    if !matches!(cleanup,Ok(Ok(status)) if status.success()) {
        return Err(InterpreterError::Rejected("container_cleanup_failed"));
    }
    result.map_err(|_| InterpreterError::Rejected("inspection_timeout"))?
}

/// Distinguish a missing container from an unavailable Docker daemon before reclaiming source files.
pub async fn cleanup_inspection(id: uuid::Uuid) -> Result<(), InterpreterError> {
    let name = format!("jrip-inspection-{id}");
    let operation = async {
        let found = Command::new("docker")
            .args([
                "ps",
                "-a",
                "--filter",
                &format!("name=^/{name}$"),
                "--format",
                "{{.Names}}",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await?;
        if !found.status.success() {
            return Err(InterpreterError::Rejected("container_cleanup_failed"));
        }
        if !found.stdout.is_empty() {
            let status = Command::new("docker")
                .args(["rm", "-f", &name])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status()
                .await?;
            if !status.success() {
                return Err(InterpreterError::Rejected("container_cleanup_failed"));
            }
        }
        Ok(())
    };
    tokio::time::timeout(Duration::from_secs(10), operation)
        .await
        .map_err(|_| InterpreterError::Rejected("container_cleanup_failed"))?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolation_and_arguments() {
        let m = Manifest {
            name: "-sOutputFile=evil.ps".into(),
            format: rip_core::DocumentFormat::Postscript,
            dpi: 600,
            color_mode: ColorMode::Cmyk,
            priority: 50,
            engine: Default::default(),
        };
        let gs = SandboxConfig::default();
        let args = gs
            .arguments(
                EngineKind::Ghostscript,
                Path::new("/source/doc"),
                Path::new("/spool/job"),
                &m,
                "test",
                "1000:1000",
            )
            .unwrap();
        for option in [
            "--network=none",
            "--read-only",
            "--cap-drop=ALL",
            "-dSAFER",
            "-sDEVICE=tiff32nc",
            "-dLastPage=101",
        ] {
            assert!(args.iter().any(|arg| arg == option));
        }
        assert!(!args.iter().any(|arg| arg == &m.name));
        assert!(
            gs.arguments(
                EngineKind::Ghostscript,
                Path::new("/bad,path"),
                Path::new("/out"),
                &m,
                "test",
                "1:1"
            )
            .is_err()
        );
    }
}
