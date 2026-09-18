use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ValidationError(pub &'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobState {
    Received,
    Validating,
    Preflight,
    Queued,
    Ripping,
    ColorProcessing,
    Halftoning,
    Spooling,
    Printing,
    Completed,
    Held,
    Cancelled,
    Failed,
}
impl JobState {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
    pub fn transition(self, next: Self) -> Result<(), ValidationError> {
        use JobState::*;
        let allowed = matches!(
            (self, next),
            (Received, Validating)
                | (Validating, Preflight | Queued | Held)
                | (Preflight, Queued | Held)
                | (Queued, Ripping | Held)
                | (Held, Queued)
                | (Ripping, ColorProcessing | Spooling)
                | (ColorProcessing, Halftoning | Spooling)
                | (Halftoning, Spooling)
                | (Spooling, Printing | Completed)
                | (Printing, Completed)
        ) || (!self.terminal() && matches!(next, Cancelled | Failed));
        if allowed {
            Ok(())
        } else {
            Err(ValidationError("invalid_state_transition"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentFormat {
    #[serde(rename = "application/pdf")]
    Pdf,
    #[serde(rename = "application/postscript")]
    Postscript,
    #[serde(rename = "application/eps")]
    Eps,
}
impl DocumentFormat {
    pub fn detect(data: &[u8]) -> Result<Self, ValidationError> {
        if data.starts_with(b"%PDF-") {
            Ok(Self::Pdf)
        } else if data.starts_with(b"%!PS-Adobe-")
            && data[..data.len().min(100)].windows(4).any(|s| s == b"EPSF")
        {
            Ok(Self::Eps)
        } else if data.starts_with(b"%!") {
            Ok(Self::Postscript)
        } else {
            Err(ValidationError("unsupported_document"))
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ColorMode {
    Rgb,
    Cmyk,
    Gray,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnginePreference {
    #[default]
    Auto,
    #[serde(rename = "mupdf")]
    MuPdf,
    Ghostscript,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    #[serde(rename = "mupdf")]
    MuPdf,
    Ghostscript,
}
impl EnginePreference {
    pub fn resolve(self, format: DocumentFormat) -> Result<EngineKind, ValidationError> {
        match (self, format) {
            (Self::Auto | Self::MuPdf, DocumentFormat::Pdf) => Ok(EngineKind::MuPdf),
            (Self::MuPdf, _) => Err(ValidationError("engine_format_mismatch")),
            _ => Ok(EngineKind::Ghostscript),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub format: DocumentFormat,
    #[serde(default = "default_dpi")]
    pub dpi: u32,
    #[serde(default = "default_color")]
    pub color_mode: ColorMode,
    #[serde(default = "default_priority")]
    pub priority: u8,
    #[serde(default)]
    pub engine: EnginePreference,
}
fn default_dpi() -> u32 {
    600
}
fn default_color() -> ColorMode {
    ColorMode::Cmyk
}
fn default_priority() -> u8 {
    50
}
impl Manifest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.engine.resolve(self.format)?;
        if self.name.trim().is_empty()
            || self.name.len() > 255
            || self.name.chars().any(char::is_control)
        {
            return Err(ValidationError("invalid_name"));
        }
        if !(72..=2400).contains(&self.dpi) {
            return Err(ValidationError("invalid_dpi"));
        }
        if self.priority > 100 {
            return Err(ValidationError("invalid_priority"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub manifest: Manifest,
    pub state: JobState,
    pub source_sha256: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub revision: i64,
    pub error_code: Option<String>,
    #[serde(default)]
    pub selected_engine: Option<EngineKind>,
}

/// Canonical interleaved raster layout; all dimensions and allocations are checked.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RasterLayout {
    pub width: u64,
    pub height: u64,
    pub channels: u8,
    pub bits_per_channel: u8,
    pub xdpi: u32,
    pub ydpi: u32,
}
impl RasterLayout {
    pub fn byte_len(self, budget: u64) -> Result<u64, ValidationError> {
        if self.width == 0
            || self.height == 0
            || self.channels == 0
            || !matches!(self.bits_per_channel, 8 | 16)
            || self.xdpi == 0
            || self.ydpi == 0
        {
            return Err(ValidationError("invalid_raster_layout"));
        }
        self.width
            .checked_mul(self.height)
            .and_then(|n| n.checked_mul(u64::from(self.channels)))
            .and_then(|n| n.checked_mul(u64::from(self.bits_per_channel / 8)))
            .filter(|n| *n <= budget)
            .ok_or(ValidationError("resource_limit"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn engine_routing_is_explicit() {
        assert_eq!(
            EnginePreference::Auto.resolve(DocumentFormat::Pdf).unwrap(),
            EngineKind::MuPdf
        );
        for format in [DocumentFormat::Postscript, DocumentFormat::Eps] {
            assert_eq!(
                EnginePreference::Auto.resolve(format).unwrap(),
                EngineKind::Ghostscript
            );
            assert!(EnginePreference::MuPdf.resolve(format).is_err());
        }
        assert_eq!(
            EnginePreference::Ghostscript
                .resolve(DocumentFormat::Pdf)
                .unwrap(),
            EngineKind::Ghostscript
        );
    }
    #[test]
    fn states_are_guarded() {
        assert!(JobState::Queued.transition(JobState::Held).is_ok());
        assert!(JobState::Held.transition(JobState::Queued).is_ok());
        for state in [JobState::Completed, JobState::Cancelled, JobState::Failed] {
            assert!(state.transition(JobState::Queued).is_err());
            assert!(state.transition(JobState::Cancelled).is_err());
        }
        assert!(JobState::Received.transition(JobState::Completed).is_err());
    }
    #[test]
    fn detects_signatures() {
        assert_eq!(
            DocumentFormat::detect(b"%!PS-Adobe-3.0 EPSF-3.0").unwrap(),
            DocumentFormat::Eps
        );
        assert!(DocumentFormat::detect(b"random.pdf").is_err());
    }
    #[test]
    fn raster_overflow_and_budget() {
        let mut r = RasterLayout {
            width: 1024,
            height: 1024,
            channels: 4,
            bits_per_channel: 16,
            xdpi: 600,
            ydpi: 600,
        };
        assert_eq!(r.byte_len(8388608).unwrap(), 8388608);
        assert!(r.byte_len(100).is_err());
        r.width = u64::MAX;
        assert!(r.byte_len(u64::MAX).is_err());
    }
}
