//! Explicit single-device routing for newly submitted local IPP jobs.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IppForward {
    pub printer_id: String,
    pub printer_uri: String,
    pub media: String,
    pub dpi: u32,
}
