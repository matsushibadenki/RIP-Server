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
        if self.max_pages == 0 || self.max_pages > 10000 || self.max_spool_bytes < 1024 * 1024 {
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
    async fn render(
        &self,
        engine: EngineKind,
        source: &Path,
        output: &Path,
        manifest: &Manifest,
    ) -> Result<RenderedDocument, InterpreterError> {
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
        let archive_path = output.join("output.tar");
        let limit = self
            .max_spool_bytes
            .checked_add(1024 * 1024 + 1)
            .ok_or(InterpreterError::Rejected("invalid_limits"))?;
        let result = tokio::time::timeout(self.timeout, async {
            let mut stdout = child
                .stdout
                .take()
                .ok_or(InterpreterError::Rejected("missing_output"))?
                .take(limit + 1);
            let mut archive = tokio::fs::File::create(&archive_path).await?;
            let bytes = tokio::io::copy(&mut stdout, &mut archive).await?;
            archive.flush().await?;
            if bytes > limit {
                return Err(InterpreterError::Rejected("resource_limit"));
            }
            Ok::<_, InterpreterError>(child.wait().await?)
        })
        .await;
        if !matches!(&result, Ok(Ok(_))) {
            let _ = child.kill().await;
        }
        // Explicitly remove the container: killing the Docker client does not stop Ghostscript.
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
        let status = result.map_err(|_| InterpreterError::Rejected("resource_limit_timeout"))??;
        if !status.success() {
            return Err(InterpreterError::Rejected("interpreter_failed"));
        }
        let extraction_output = output.clone();
        let max_pages = self.max_pages;
        let max_bytes = self.max_spool_bytes;
        let mut pages = tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(&archive_path)?;
            let mut archive = tar::Archive::new(file);
            let mut pages = Vec::new();
            let mut total = 0u64;
            for entry in archive.entries()? {
                let mut entry = entry?;
                if entry.header().entry_type().is_dir() { continue; }
                let path = entry.path()?.into_owned();
                let filename = path.file_name().and_then(|s| s.to_str()).ok_or(InterpreterError::Rejected("invalid_output"))?;
                let number = filename.strip_prefix("page-").and_then(|s| s.strip_suffix(".tiff"));
                if !entry.header().entry_type().is_file() || !matches!(number, Some(n) if n.len()==6 && n.bytes().all(|b|b.is_ascii_digit())) {
                    return Err(InterpreterError::Rejected("invalid_output"));
                }
                let size = entry.size();
                total = total.checked_add(size).ok_or(InterpreterError::Rejected("resource_limit"))?;
                if size < 8 || total > max_bytes || pages.len() >= max_pages as usize { return Err(InterpreterError::Rejected("resource_limit")); }
                let destination = extraction_output.join(filename);
                let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(&destination)?;
                std::io::copy(&mut entry, &mut file)?;
                file.sync_all()?;
                let mut header = [0u8;4];
                use std::io::Read;
                std::fs::File::open(&destination)?.read_exact(&mut header)?;
                if header != *b"II\x2a\x00" && header != *b"MM\x00\x2a" { return Err(InterpreterError::Rejected("invalid_tiff")); }
                pages.push(destination);
            }
            if pages.is_empty() { return Err(InterpreterError::Rejected("empty_output")); }
            std::fs::remove_file(archive_path)?;
            Ok::<_,InterpreterError>(pages)
        }).await.map_err(|_| InterpreterError::Rejected("output_task_failed"))??;
        pages.sort();
        Ok(RenderedDocument { pages })
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
            .render(EngineKind::Ghostscript, source, output, manifest)
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
            .render(EngineKind::MuPdf, source, output, manifest)
            .await
    }
}
#[derive(Default)]
pub struct HybridInterpreter {
    pub ghostscript: GhostscriptInterpreter,
    pub mupdf: MuPdfInterpreter,
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
