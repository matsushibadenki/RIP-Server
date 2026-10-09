//! Shared execution vocabulary. Source integrity is distinct from structural preflight.
use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
pub const RIP_REVISION: &str = "jrip-rip-v1";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceDocument {
    pub uri: String,
    pub media_type: DocumentFormat,
    pub sha256: String,
    pub bytes: Option<u64>,
    pub page_count: Option<u32>,
    pub integrity_verified: bool,
    pub structure_verified: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    Artifact,
    Printer,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Output {
    pub destination: Destination,
    pub printer_id: Option<String>,
    pub media_type: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobTicket {
    pub schema_version: u32,
    pub version: String,
    pub job_id: Option<Uuid>,
    pub attempt_id: Option<Uuid>,
    pub capability_revision: String,
    pub document: SourceDocument,
    pub manifest: Manifest,
    pub selected_engine: EngineKind,
    pub copies: u32,
    pub sides: String,
    pub media: Option<String>,
    pub finishings: Vec<String>,
    pub page_selection: String,
    pub number_up: u32,
    pub imposition: String,
    pub quality: String,
    pub deadline: Option<String>,
    pub color_policy: Option<String>,
    pub output: Output,
    pub completion_meaning: String,
    pub document_verified: bool,
}
impl JobTicket {
    pub fn raster(document: SourceDocument, manifest: Manifest) -> Result<Self, ValidationError> {
        Self::build(document, manifest, None)
    }
    fn build(
        document: SourceDocument,
        manifest: Manifest,
        job_id: Option<Uuid>,
    ) -> Result<Self, ValidationError> {
        let selected_engine = manifest.engine.resolve(manifest.format)?;
        let ticket = Self {
            schema_version: 1,
            version: "0.1".into(),
            job_id,
            attempt_id: job_id,
            capability_revision: RIP_REVISION.into(),
            document,
            manifest,
            selected_engine,
            copies: 1,
            sides: "one-sided".into(),
            media: None,
            finishings: vec![],
            page_selection: "all".into(),
            number_up: 1,
            imposition: "none".into(),
            quality: "standard".into(),
            deadline: None,
            color_policy: None,
            output: Output {
                destination: Destination::Artifact,
                printer_id: None,
                media_type: "image/tiff".into(),
            },
            completion_meaning: "raster_export".into(),
            document_verified: false,
        };
        ticket.validate_raster()?;
        Ok(ticket)
    }
    pub fn published(job: &Job, bytes: u64) -> Result<Self, ValidationError> {
        Self::build(
            SourceDocument {
                uri: format!("jrip://jobs/{}/source", job.id),
                media_type: job.manifest.format,
                sha256: job.source_sha256.clone(),
                bytes: Some(bytes),
                page_count: None,
                integrity_verified: true,
                structure_verified: false,
            },
            job.manifest.clone(),
            Some(job.id),
        )
    }
    /// Validate implemented raster semantics; no caller-supplied verification flag grants authority.
    pub fn validate_raster(&self) -> Result<(), ValidationError> {
        self.manifest.validate()?;
        if self.schema_version != 1
            || self.version != "0.1"
            || self.capability_revision != RIP_REVISION
        {
            return Err(ValidationError("unsupported_ticket_version"));
        }
        if self.document.media_type != self.manifest.format
            || self.selected_engine != self.manifest.engine.resolve(self.manifest.format)?
        {
            return Err(ValidationError("ticket_manifest_mismatch"));
        }
        if !valid_digest(&self.document.sha256)
            || self
                .document
                .bytes
                .is_some_and(|bytes| bytes == 0 || bytes > 32 * 1024 * 1024)
            || self
                .document
                .page_count
                .is_some_and(|count| count == 0 || count > 100)
        {
            return Err(ValidationError("invalid_source_reference"));
        }
        let uri = &self.document.uri;
        let published_uri = self.job_id.map(|id| format!("jrip://jobs/{id}/source"));
        if uri.len() > 2048
            || uri.chars().any(char::is_control)
            || (!valid_https_reference(uri) && published_uri.as_ref() != Some(uri))
            || self.attempt_id != self.job_id
        {
            return Err(ValidationError("invalid_source_reference"));
        }
        if (self.document.integrity_verified && self.document.bytes.is_none())
            || self.document.structure_verified
            || self.document_verified
        {
            return Err(ValidationError("invalid_source_evidence"));
        }
        if self.copies != 1
            || self.sides != "one-sided"
            || self.media.is_some()
            || !self.finishings.is_empty()
            || self.page_selection != "all"
            || self.number_up != 1
            || self.imposition != "none"
            || self.quality != "standard"
            || self.deadline.is_some()
            || self.color_policy.is_some()
        {
            return Err(ValidationError("unsupported_ticket_setting"));
        }
        if self.output.destination != Destination::Artifact
            || self.output.printer_id.is_some()
            || self.output.media_type != "image/tiff"
            || self.completion_meaning != "raster_export"
        {
            return Err(ValidationError("unsupported_ticket_output"));
        }
        Ok(())
    }
}
pub fn valid_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn valid_https_reference(uri: &str) -> bool {
    if uri.len() > 2048 || uri.chars().any(char::is_control) {
        return false;
    }
    url::Url::parse(uri).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Rip,
    Color,
    Queue,
    Printer,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    Unavailable,
    NotProbed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DpiSupport {
    pub minimum: u32,
    pub maximum: u32,
    pub discrete: Option<Vec<[u32; 2]>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaCapability {
    pub name: String,
    pub width_microns: u32,
    pub height_microns: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityModel {
    pub schema_version: u32,
    pub id: String,
    pub stage: Stage,
    pub input_media_types: Vec<String>,
    pub output_media_types: Vec<String>,
    pub dpi: Option<DpiSupport>,
    pub color_spaces: Vec<String>,
    pub bit_depths: Vec<u8>,
    pub profiles: Vec<String>,
    pub rendering_intents: Vec<String>,
    pub media: Vec<MediaCapability>,
    pub sides: Vec<String>,
    pub copies: Option<[u32; 2]>,
    pub finishings: Vec<String>,
    pub availability: Availability,
    pub limits: Value,
    pub constraints: Vec<String>,
    pub extensions: Value,
}
impl CapabilityModel {
    pub fn rip() -> Self {
        Self {
            schema_version: 1,
            id: "org.jrip.rip.export_tiff".into(),
            stage: Stage::Rip,
            input_media_types: vec![
                "application/pdf".into(),
                "application/postscript".into(),
                "application/eps".into(),
            ],
            output_media_types: vec!["image/tiff".into()],
            dpi: Some(DpiSupport {
                minimum: 72,
                maximum: 2400,
                discrete: None,
            }),
            color_spaces: vec!["rgb".into(), "cmyk".into(), "gray".into()],
            bit_depths: vec![8],
            profiles: vec![],
            rendering_intents: vec![],
            media: vec![],
            sides: vec![],
            copies: None,
            finishings: vec![],
            availability: Availability::NotProbed,
            limits: json!({"sourceBytes":33554432,"artifactBytes":1073741824,"pages":100,"maximumDimensions":null}),
            constraints: vec![
                "source_integrity_is_not_document_structure_preflight".into(),
                "completion_is_tiff_export".into(),
            ],
            extensions: json!({"org.jrip.engines":{"mupdf":["application/pdf"],"ghostscript":["application/pdf","application/postscript","application/eps"]}}),
        }
    }
    pub fn color() -> Self {
        let mut model = Self::rip();
        model.id = "org.jrip.color.icc-transform".into();
        model.stage = Stage::Color;
        model.input_media_types = vec!["image/tiff".into()];
        model.rendering_intents = vec![
            "perceptual".into(),
            "relative_colorimetric".into(),
            "absolute_colorimetric".into(),
            "saturation".into(),
        ];
        model.limits = json!({"profileBytes":4194304,"pageSampleBytes":67108864,"artifactBytes":1073741824,"pages":100,"maximumDimensions":null});
        model.constraints = vec![
            "explicit_registered_profile_required".into(),
            "device_link_header_intent_required".into(),
            "black_preservation_cmyk_only_excludes_absolute_intent".into(),
        ];
        model.extensions = json!({"org.jrip.color":{"blackPointCompensation":true,"blackPreservation":["none","k_only","k_plane"],"tacPercent":[100,400]}});
        model
    }
    pub fn queue() -> Self {
        let mut model = Self::rip();
        model.id = "org.jrip.queue.delivery".into();
        model.stage = Stage::Queue;
        model.input_media_types = vec!["image/pwg-raster".into()];
        model.output_media_types = model.input_media_types.clone();
        model.dpi = None;
        model.color_spaces = vec![];
        model.bit_depths = vec![];
        model.limits = json!({"artifactBytes":33554432});
        model.constraints = vec![
            "ambiguous_device_submission_never_automatically_retried".into(),
            "acceptance_is_not_device_completion".into(),
        ];
        model.extensions = json!({"org.jrip.queue":{"deliveryIdempotency":true,"leases":true,"boundedAutomaticRetry":false,"deadlines":false,"fairness":false}});
        model
    }
    pub fn snapshot(&self, now: i64) -> Value {
        let digest = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&serde_json::to_value(self).expect("serializable capability"))
                    .expect("serializable capability")
            )
        );
        json!({"capability":self,"capabilityRevision":digest,"observedAt":now,"expiresAt":now+30})
    }
}

/// Settings intersection for the implemented TIFF → PWG bridge. This is not
/// source/geometry validation or permission to submit to a physical device.
pub fn delivery_intersection(models: &[CapabilityModel]) -> Value {
    let find = |stage| {
        models
            .iter()
            .find(|model| std::mem::discriminant(&model.stage) == stage)
    };
    let rip = find(std::mem::discriminant(&Stage::Rip));
    let queue = find(std::mem::discriminant(&Stage::Queue));
    let printer = find(std::mem::discriminant(&Stage::Printer));
    let (Some(rip), Some(queue), Some(printer)) = (rip, queue, printer) else {
        return json!({"settingsCompatible":false,"conflicts":[{"code":"printer_capability_required","retryable":false}],"authorizationGranted":false});
    };
    let mut conflicts = vec![];
    let bridge = rip.output_media_types.iter().any(|m| m == "image/tiff")
        && queue
            .input_media_types
            .iter()
            .any(|m| m == "image/pwg-raster")
        && queue
            .output_media_types
            .iter()
            .any(|m| printer.input_media_types.contains(m));
    if !bridge {
        conflicts.push(json!({"code":"pipeline_format_mismatch","retryable":false}));
    }
    let dpi: Vec<_> = printer
        .dpi
        .as_ref()
        .and_then(|d| d.discrete.as_ref())
        .into_iter()
        .flatten()
        .filter(|d| {
            rip.dpi
                .as_ref()
                .is_some_and(|r| d[0] == d[1] && (r.minimum..=r.maximum).contains(&d[0]))
        })
        .copied()
        .collect();
    if dpi.is_empty() {
        conflicts.push(json!({"code":"pipeline_dpi_mismatch","retryable":false}));
    }
    let colors: Vec<_> = rip
        .color_spaces
        .iter()
        .filter(|c| printer.color_spaces.contains(c))
        .collect();
    if colors.is_empty() && printer.profiles.is_empty() {
        conflicts.push(json!({"code":"pipeline_color_mismatch","retryable":false}));
    }
    if !rip.bit_depths.contains(&8) || !printer.bit_depths.contains(&8) {
        conflicts.push(json!({"code":"pipeline_bit_depth_mismatch","retryable":false}));
    }
    if printer.media.is_empty() {
        conflicts.push(json!({"code":"pipeline_media_required","retryable":false}));
    }
    if matches!(printer.availability, Availability::Unavailable) {
        conflicts.push(json!({"code":"printer_unavailable","retryable":true}));
    }
    let max_bytes = [rip, queue, printer]
        .into_iter()
        .filter_map(|m| m.limits["artifactBytes"].as_u64())
        .min();
    let requires_color = !printer.profiles.is_empty();
    let color = find(std::mem::discriminant(&Stage::Color));
    let color_outputs: Vec<_> = color
        .into_iter()
        .flat_map(|c| c.color_spaces.iter())
        .filter(|space| printer.color_spaces.contains(space))
        .collect();
    let color_path = color.is_some_and(|c| {
        c.input_media_types.contains(&"image/tiff".into())
            && c.output_media_types.contains(&"image/tiff".into())
            && rip.color_spaces.iter().any(|s| c.color_spaces.contains(s))
            && printer
                .color_spaces
                .iter()
                .any(|s| c.color_spaces.contains(s))
            && c.bit_depths.contains(&8)
    });
    if requires_color && !color_path {
        conflicts.push(json!({"code":"pipeline_color_stage_required","retryable":false}));
    }
    json!({"settingsCompatible":conflicts.is_empty(),"conflicts":conflicts,
        "dpi":dpi,"directColorSpaces":colors,"media":printer.media,"bitDepths":[8],
        "artifactBytes":max_bytes,"copies":[1,1],"sides":["one-sided"],"finishings":["none"],
        "bridge":{"inputMediaType":"image/tiff","outputMediaType":"image/pwg-raster","preservesGeometry":true},
        "explicitColorRequired":requires_color,"colorPathSupported":color_path,"colorOutputSpaces":color_outputs,
        "requiredProfiles":printer.profiles,"runtimeProbed":false,"documentVerified":false,
        "authorizationGranted":false,"automaticSubmissionSupported":false,
        "remainingChecks":["registered_profile_types_and_policy","artifact_integrity_and_size","exact_media_geometry","current_capability_revision","scoped_authorization_and_approval"],
        "messages":{"en":"Settings intersection only; explicit color and delivery remain separate attempts","ja":"設定の交差照合のみです。明示的な色処理と実機への送信は別attemptです","zh-CN":"仅交叉匹配设置；显式颜色处理和投递仍为独立尝试"}})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intersection_checks_bridge_and_discrete_dpi_without_granting_authority() {
        let mut printer = CapabilityModel::rip();
        printer.stage = Stage::Printer;
        printer.input_media_types = vec!["image/pwg-raster".into()];
        printer.dpi = Some(DpiSupport {
            minimum: 300,
            maximum: 300,
            discrete: Some(vec![[300, 300]]),
        });
        printer.media = vec![MediaCapability {
            name: "a4".into(),
            width_microns: 210000,
            height_microns: 297000,
        }];
        printer.availability = Availability::Available;
        let mut models = vec![
            CapabilityModel::rip(),
            CapabilityModel::color(),
            CapabilityModel::queue(),
            printer,
        ];
        let result = delivery_intersection(&models);
        assert_eq!(result["settingsCompatible"], true);
        assert_eq!(result["artifactBytes"], 33554432);
        assert_eq!(result["authorizationGranted"], false);
        models[3].profiles = vec!["a".repeat(64)];
        assert_eq!(
            delivery_intersection(&models)["explicitColorRequired"],
            true
        );
        models[1].output_media_types.clear();
        assert_eq!(
            delivery_intersection(&models)["conflicts"][0]["code"],
            "pipeline_color_stage_required"
        );
        models[1] = CapabilityModel::color();
        models[3].profiles.clear();
        models[3].dpi.as_mut().unwrap().discrete = Some(vec![[300, 600]]);
        assert_eq!(
            delivery_intersection(&models)["conflicts"][0]["code"],
            "pipeline_dpi_mismatch"
        );
        models[2].output_media_types.clear();
        assert_eq!(
            delivery_intersection(&models)["conflicts"][0]["code"],
            "pipeline_format_mismatch"
        );
        assert_eq!(
            delivery_intersection(&models[..3])["settingsCompatible"],
            false
        );
    }
    #[test]
    fn ticket_rejects_unimplemented_settings_and_inconsistent_engine() {
        let manifest: Manifest =
            serde_json::from_value(json!({"name":"x.pdf","format":"application/pdf"})).unwrap();
        let mut ticket = JobTicket::raster(
            SourceDocument {
                uri: "https://example.org/x.pdf".into(),
                media_type: DocumentFormat::Pdf,
                sha256: "a".repeat(64),
                bytes: None,
                page_count: None,
                integrity_verified: false,
                structure_verified: false,
            },
            manifest,
        )
        .unwrap();
        ticket.selected_engine = EngineKind::Ghostscript;
        assert!(ticket.validate_raster().is_err());
        ticket.selected_engine = EngineKind::MuPdf;
        ticket.copies = 2;
        assert!(ticket.validate_raster().is_err());
        ticket.copies = 1;
        ticket.deadline = Some("2030-01-01T00:00:00Z".into());
        assert!(ticket.validate_raster().is_err());
    }
    #[test]
    fn capability_revision_excludes_observation_time_and_binds_constraints() {
        let mut model = CapabilityModel::rip();
        assert_eq!(
            model.snapshot(1)["capabilityRevision"],
            model.snapshot(2)["capabilityRevision"]
        );
        let before = model.snapshot(2)["capabilityRevision"].clone();
        model.color_spaces = vec!["gray".into()];
        assert_ne!(before, model.snapshot(2)["capabilityRevision"]);
    }
}
