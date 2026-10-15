//! One worker gate for every transport; no Docker privileges are added to frontends.
use rip_core::{DocumentFormat, Job};
use rip_interpreter::{DocumentInterpreter, HybridInterpreter, InterpreterError, RenderedDocument};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Snapshot {
    directory: PathBuf,
    guard: Option<std::fs::File>,
    _budget: Vec<std::fs::File>,
    keep: bool,
    bytes: u64,
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        drop(self.guard.take());
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}
impl Snapshot {
    async fn open(root: &Path, directory: &Path, job: &Job) -> Result<Self, InterpreterError> {
        let budget = crate::artifacts::reserve_shared(root, rip_jobs::MAX_DOCUMENT_BYTES)?;
        let source = directory.join("document");
        if !tokio::fs::symlink_metadata(&source).await?.is_file()
            || tokio::fs::canonicalize(&source).await? != source
        {
            return Err(InterpreterError::Rejected("invalid_source"));
        }
        let path = directory.join("source-validation");
        tokio::fs::create_dir(&path).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).await?;
        }
        let mut snapshot = Self {
            directory: path,
            guard: None,
            _budget: budget,
            keep: false,
            bytes: 0,
        };
        snapshot.guard = Some(rip_jobs::upload_guard(&snapshot.directory)?);
        let mut input = tokio::fs::File::open(source)
            .await?
            .take(rip_jobs::MAX_DOCUMENT_BYTES + 1);
        let mut output = tokio::fs::File::create(snapshot.directory.join("source.part")).await?;
        snapshot.bytes = tokio::io::copy(&mut input, &mut output).await?;
        output.flush().await?;
        output.sync_all().await?;
        drop(output);
        rip_jobs::prepare_staged(
            &snapshot.directory,
            job.id,
            job.manifest.clone(),
            Some(&job.source_sha256),
        )
        .await
        .map_err(|_| InterpreterError::Rejected("source_integrity_mismatch"))?;
        Ok(snapshot)
    }
}
pub struct ValidatedRender {
    pub rendered: RenderedDocument,
    pub source_bytes: u64,
    pub strict_pdf: bool,
}
pub async fn render(
    root: &Path,
    directory: &Path,
    job: &Job,
    engine: &HybridInterpreter,
    output: &Path,
) -> Result<ValidatedRender, InterpreterError> {
    let mut snapshot = Snapshot::open(root, directory, job).await?;
    let source = snapshot.directory.join("document");
    let strict_pdf = job.manifest.format == DocumentFormat::Pdf;
    let result = async {
        let inspected = if strict_pdf {
            let report = rip_interpreter::inspect_pdf_attempt_with_image(
                &source,
                uuid::Uuid::new_v4(),
                &engine.mupdf.sandbox.image,
            )
            .await?;
            Some(
                report["pages"]
                    .as_array()
                    .ok_or(InterpreterError::Rejected("invalid_inspection"))?
                    .len(),
            )
        } else {
            None
        };
        let rendered = engine.render(&source, output, &job.manifest).await?;
        if inspected.is_some_and(|count| count != rendered.pages.len()) {
            return Err(InterpreterError::Rejected("source_page_count_mismatch"));
        }
        Ok(ValidatedRender {
            rendered,
            source_bytes: snapshot.bytes,
            strict_pdf,
        })
    }
    .await;
    if matches!(
        &result,
        Err(InterpreterError::Rejected("container_cleanup_failed"))
    ) {
        snapshot.keep = true;
    }
    result
}
