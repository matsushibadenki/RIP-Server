//! Stateless capability negotiation. This does not authorize or execute a job.
use rip_core::{ColorMode, DocumentFormat, EnginePreference, Manifest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrintIntent {
    pub document: Document,
    pub copies: Requirement<u32>,
    pub output: Output,
    pub dpi: Option<Requirement<u32>>,
    pub color_mode: Option<Requirement<String>>,
    pub media: Option<Requirement<String>>,
    pub sides: Option<Requirement<String>>,
    pub finishings: Option<Requirement<Vec<String>>>,
    pub color_policy: Option<String>,
    pub priority: Option<u8>,
    pub deadline: Option<String>,
    pub idempotency_key: Option<String>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Document {
    pub uri: String,
    pub media_type: String,
    pub sha256: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement<T> {
    pub mode: Mode,
    pub value: T,
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Required,
    Preferred,
    Automatic,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Output {
    pub destination: Destination,
    pub printer_id: Option<String>,
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    Artifact,
    Printer,
}

pub fn resolve(intent: PrintIntent) -> Value {
    let mut conflicts = Vec::new();
    let mut adjustments = Vec::new();
    let mut issue = |field: &str, code: &str| conflicts.push(json!({"field":field,"code":code}));
    // References are metadata only: the resolver never downloads caller-provided URLs.
    if !intent.document.uri.starts_with("https://")
        || intent.document.uri.len() > 2048
        || intent.document.uri.chars().any(char::is_control)
    {
        issue("document.uri", "https_reference_required");
    }
    if intent.document.sha256.len() != 64
        || !intent
            .document
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        issue("document.sha256", "invalid_sha256");
    }
    let format = match intent.document.media_type.as_str() {
        "application/pdf" => DocumentFormat::Pdf,
        "application/postscript" => DocumentFormat::Postscript,
        "application/eps" => DocumentFormat::Eps,
        _ => {
            issue("document.mediaType", "unsupported_format");
            DocumentFormat::Pdf
        }
    };
    if intent.output.destination != Destination::Artifact || intent.output.printer_id.is_some() {
        issue("output", "physical_output_unavailable");
    }
    if intent.copies.value == 0 {
        issue("copies", "invalid_copies");
    }
    if intent.copies.value != 1 {
        if intent.copies.mode == Mode::Required {
            issue("copies", "copies_unavailable");
        } else {
            adjustments
                .push(json!({"field":"copies","requested":intent.copies.value,"resolved":1}));
        }
    }
    // These options have no meaningful implementation yet. Explicit conflicts keep intent visible.
    for (field, present) in [
        ("media", intent.media.is_some()),
        ("sides", intent.sides.is_some()),
        ("finishings", intent.finishings.is_some()),
        ("colorPolicy", intent.color_policy.is_some()),
        ("deadline", intent.deadline.is_some()),
    ] {
        if present {
            issue(field, "unsupported_setting");
        }
    }
    if intent
        .idempotency_key
        .as_ref()
        .is_some_and(|s| !(16..=255).contains(&s.len()))
    {
        issue("idempotencyKey", "invalid_idempotency_key");
    }
    let mut dpi = 600;
    if let Some(r) = intent.dpi {
        if r.value == 0 {
            issue("dpi", "invalid_dpi");
        } else if r.mode == Mode::Automatic {
            adjustments.push(json!({"field":"dpi","requested":r.value,"resolved":dpi}));
        } else if (72..=2400).contains(&r.value) {
            dpi = r.value;
        } else if r.mode == Mode::Required {
            issue("dpi", "unsupported_dpi");
        } else {
            adjustments.push(json!({"field":"dpi","requested":r.value,"resolved":dpi}));
        }
    }
    let mut color = ColorMode::Cmyk;
    if let Some(r) = intent.color_mode {
        let selected = match r.value.as_str() {
            "rgb" => Some(ColorMode::Rgb),
            "cmyk" => Some(ColorMode::Cmyk),
            "gray" => Some(ColorMode::Gray),
            _ => None,
        };
        if r.mode == Mode::Automatic {
            adjustments.push(json!({"field":"colorMode","requested":r.value,"resolved":"cmyk"}));
        } else if let Some(v) = selected {
            color = v;
        } else if r.mode == Mode::Required {
            issue("colorMode", "unsupported_color_mode");
        } else {
            adjustments.push(json!({"field":"colorMode","requested":r.value,"resolved":"cmyk"}));
        }
    }
    let manifest = Manifest {
        name: "Agent RIP export".into(),
        format,
        dpi,
        color_mode: color,
        priority: intent.priority.unwrap_or(50),
        engine: EnginePreference::Auto,
    };
    if let Err(e) = manifest.validate() {
        issue("manifest", e.0);
    }
    let ticket = if conflicts.is_empty() {
        Some(
            json!({"version":"0.1", "capabilityRevision":"jrip-rip-v1", "document":intent.document,"manifest":manifest,"selectedEngine":manifest.engine.resolve(format).unwrap(),"output":{"destination":"artifact","mediaType":"image/tiff"},"copies":1,"completionMeaning":"raster_export","documentVerified":false}),
        )
    } else {
        None
    };
    json!({"executable":ticket.is_some(),"ticket":ticket,"conflicts":conflicts,"adjustments":adjustments,"authorizationGranted":false,"messages":{"en":"Settings negotiation only; source verification and authorization are required before execution.","ja":"設定の能力照合結果です。実行には原稿検証と認可が必要です。","zh-CN":"仅为设置协商结果；执行前需要验证原稿并授权。"}})
}
