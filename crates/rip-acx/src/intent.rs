//! Stateless negotiation of the shared ticket vocabulary. Never authorizes execution.
use rip_core::{
    ColorMode, DocumentFormat, EnginePreference, Manifest,
    print::{JobTicket, SourceDocument, valid_digest, valid_https_reference},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    pub color_policy: Option<Preference<String>>,
    pub priority: Option<Preference<u32>>,
    pub deadline: Option<Preference<String>>,
    pub idempotency_key: Option<String>,
    pub engine: Option<Requirement<String>>,
    pub page_ranges: Option<Requirement<Vec<PageRange>>>,
    pub number_up: Option<Requirement<u32>>,
    pub imposition: Option<Requirement<String>>,
    pub quality: Option<Requirement<String>>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageRange {
    pub first: u32,
    pub last: u32,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Document {
    pub uri: String,
    pub media_type: String,
    pub sha256: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement<T> {
    pub mode: Mode,
    pub value: T,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Required,
    Preferred,
    Automatic,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Preference<T> {
    Requirement(Requirement<T>),
    Legacy(T),
}
impl<T> Preference<T> {
    fn requirement(self) -> Requirement<T> {
        match self {
            Self::Requirement(r) => r,
            Self::Legacy(value) => Requirement {
                mode: Mode::Required,
                value,
            },
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Output {
    pub destination: Destination,
    pub printer_id: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    Artifact,
    Printer,
}
fn conflict(field: &str, code: &str, requested: Value, alternatives: Value) -> Value {
    json!({"field":field,"code":code,"requested":requested,"alternatives":alternatives,"retryable":false,"messages":{"en":format!("Unsupported or invalid requirement: {field}"),"ja":format!("未対応または不正な指定: {field}"),"zh-CN":format!("不支持或无效要求: {field}")}})
}
#[derive(Default)]
struct Negotiation {
    conflicts: Vec<Value>,
    adjustments: Vec<Value>,
}
impl Negotiation {
    fn select<T: Clone + Serialize + PartialEq>(
        &mut self,
        field: &str,
        requirement: Option<Requirement<T>>,
        default: T,
        supported: impl Fn(&T) -> bool,
        alternatives: Value,
    ) -> T {
        let Some(r) = requirement else {
            return default;
        };
        if r.mode != Mode::Automatic && supported(&r.value) {
            return r.value;
        }
        if r.mode == Mode::Required {
            self.conflicts.push(conflict(
                field,
                &format!("unsupported_{}", snake(field)),
                json!(r.value),
                alternatives,
            ));
        } else if r.value != default {
            self.adjustments.push(json!({"field":field,"requested":r.value,"resolved":default,"reasonCode":"PREFERENCE_NOT_SUPPORTED","alternatives":alternatives}));
        }
        default
    }
    fn unavailable<T: Serialize>(&mut self, field: &str, requirement: Option<Requirement<T>>) {
        if let Some(r) = requirement {
            if r.mode == Mode::Required {
                self.conflicts.push(conflict(
                    field,
                    "unsupported_setting",
                    json!(r.value),
                    json!([null]),
                ));
            } else {
                self.adjustments.push(json!({"field":field,"requested":r.value,"resolved":null,"reasonCode":"SETTING_NOT_IMPLEMENTED","alternatives":[null]}));
            }
        }
    }
}
fn snake(value: &str) -> String {
    let mut result = String::new();
    for c in value.chars() {
        if c.is_ascii_uppercase() {
            result.push('_');
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

pub fn resolve(intent: PrintIntent) -> Value {
    let mut n = Negotiation::default();
    if !valid_https_reference(&intent.document.uri) {
        n.conflicts.push(conflict(
            "document.uri",
            "https_reference_required",
            json!(intent.document.uri),
            json!([]),
        ));
    }
    if !valid_digest(&intent.document.sha256) {
        n.conflicts.push(conflict(
            "document.sha256",
            "invalid_sha256",
            json!(intent.document.sha256),
            json!([]),
        ));
    }
    let format = match intent.document.media_type.as_str() {
        "application/pdf" => DocumentFormat::Pdf,
        "application/postscript" => DocumentFormat::Postscript,
        "application/eps" => DocumentFormat::Eps,
        _ => {
            n.conflicts.push(conflict(
                "document.mediaType",
                "unsupported_format",
                json!(intent.document.media_type),
                json!([
                    "application/pdf",
                    "application/postscript",
                    "application/eps"
                ]),
            ));
            DocumentFormat::Pdf
        }
    };
    if intent.output.destination != Destination::Artifact || intent.output.printer_id.is_some() {
        n.conflicts.push(conflict(
            "output",
            "physical_output_unavailable",
            json!(intent.output),
            json!([{"destination":"artifact","nextAction":"explicit_delivery_after_rip"}]),
        ));
    }
    if intent.copies.value == 0 {
        n.conflicts
            .push(conflict("copies", "invalid_copies", json!(0), json!([1])));
    }
    n.select("copies", Some(intent.copies), 1, |v| *v == 1, json!([1]));
    let dpi = n.select(
        "dpi",
        intent.dpi,
        600,
        |v| (72..=2400).contains(v),
        json!({"minimum":72,"maximum":2400,"default":600}),
    );
    let color = n.select(
        "colorMode",
        intent.color_mode,
        "cmyk".to_string(),
        |v| matches!(v.as_str(), "rgb" | "cmyk" | "gray"),
        json!(["rgb", "cmyk", "gray"]),
    );
    n.select(
        "media",
        intent.media,
        "document-size".to_string(),
        |v| v == "document-size",
        json!(["document-size"]),
    );
    n.select(
        "sides",
        intent.sides,
        "one-sided".to_string(),
        |v| v == "one-sided",
        json!(["one-sided"]),
    );
    n.select(
        "finishings",
        intent.finishings,
        Vec::<String>::new(),
        |v| v.is_empty(),
        json!([[]]),
    );
    n.unavailable(
        "colorPolicy",
        intent.color_policy.map(Preference::requirement),
    );
    n.unavailable("deadline", intent.deadline.map(Preference::requirement));
    let priority = n.select(
        "priority",
        intent.priority.map(Preference::requirement),
        50,
        |v| *v <= 100,
        json!({"minimum":0,"maximum":100,"default":50}),
    );
    let engine = n.select(
        "engine",
        intent.engine,
        "auto".to_string(),
        |v| {
            matches!(v.as_str(), "auto" | "ghostscript")
                || (v == "mupdf" && format == DocumentFormat::Pdf)
        },
        if format == DocumentFormat::Pdf {
            json!(["auto", "mupdf", "ghostscript"])
        } else {
            json!(["auto", "ghostscript"])
        },
    );
    if intent.page_ranges.as_ref().is_some_and(|r| {
        r.value.len() > 100 || r.value.iter().any(|v| v.first == 0 || v.last < v.first)
    }) {
        n.conflicts.push(conflict(
            "pageRanges",
            "invalid_page_ranges",
            json!(intent.page_ranges),
            json!([]),
        ));
    }
    n.unavailable(
        "pageRanges",
        intent.page_ranges.filter(|r| !r.value.is_empty()),
    );
    n.select("numberUp", intent.number_up, 1, |v| *v == 1, json!([1]));
    n.select(
        "imposition",
        intent.imposition,
        "none".to_string(),
        |v| v == "none",
        json!(["none"]),
    );
    n.select(
        "quality",
        intent.quality,
        "standard".to_string(),
        |v| v == "standard",
        json!(["standard"]),
    );
    if intent
        .idempotency_key
        .as_ref()
        .is_some_and(|key| !(16..=255).contains(&key.len()) || key.chars().any(char::is_control))
    {
        n.conflicts.push(conflict(
            "idempotencyKey",
            "invalid_idempotency_key",
            json!(intent.idempotency_key),
            json!([]),
        ));
    }
    let manifest = Manifest {
        name: "Agent RIP export".into(),
        format,
        dpi,
        color_mode: match color.as_str() {
            "rgb" => ColorMode::Rgb,
            "gray" => ColorMode::Gray,
            _ => ColorMode::Cmyk,
        },
        priority: priority as u8,
        engine: match engine.as_str() {
            "mupdf" => EnginePreference::MuPdf,
            "ghostscript" => EnginePreference::Ghostscript,
            _ => EnginePreference::Auto,
        },
    };
    let ticket = if n.conflicts.is_empty() {
        JobTicket::raster(
            SourceDocument {
                uri: intent.document.uri,
                media_type: format,
                sha256: intent.document.sha256,
                bytes: None,
                page_count: None,
                integrity_verified: false,
                structure_verified: false,
            },
            manifest,
        )
        .ok()
        .map(|ticket| serde_json::to_value(ticket).expect("serializable ticket"))
    } else {
        None
    };
    json!({"executable":ticket.is_some(),"ticket":ticket,"conflicts":n.conflicts,"adjustments":n.adjustments,"authorizationGranted":false,"messages":{"en":"Settings negotiation only; source verification and authorization are required before execution.","ja":"設定の能力照合結果です。実行には原稿検証と認可が必要です。","zh-CN":"仅为设置协商结果；执行前需要验证原稿并授权。"}})
}
