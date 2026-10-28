//! Common stage models, pure ticket validation and immutable publication records.
use super::*;
use rip_core::print::{
    Availability, CapabilityModel, DpiSupport, JobTicket, MediaCapability, Stage,
};
use rip_printer::PrinterBackend;
use serde_json::{Value, json};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityQuery {
    pub printer_id: Option<String>,
}
pub(crate) async fn capabilities(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<CapabilityQuery>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(models(&app, query.printer_id.as_deref()).await?))
}
pub async fn models(app: &App, printer_id: Option<&str>) -> Result<Value, ApiError> {
    use sha2::{Digest, Sha256};
    let mut models = vec![
        CapabilityModel::rip(),
        CapabilityModel::color(),
        CapabilityModel::queue(),
    ];
    if let Some(id) = printer_id {
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(bad("invalid_printer"));
        }
        let printer = app
            .printers
            .iter()
            .find(|printer| printer.id == id)
            .ok_or(bad("printer_not_configured"))?;
        let device = delivery::backend(app, printer)
            .map_err(|_| bad("invalid_printer"))?
            .capabilities()
            .await
            .map_err(|_| bad("printer_unavailable"))?;
        let mut model = CapabilityModel::rip();
        model.id = format!("org.jrip.printer.{id}");
        model.stage = Stage::Printer;
        model.input_media_types = device
            .document_formats
            .into_iter()
            .filter(|mime| mime == "image/pwg-raster")
            .collect();
        model.output_media_types = vec![];
        let mut resolutions: Vec<_> = device
            .pwg_resolutions
            .into_iter()
            .filter(|dpi| dpi[0] == dpi[1] && (72..=2400).contains(&dpi[0]))
            .collect();
        resolutions.sort();
        resolutions.dedup();
        model.dpi = Some(DpiSupport {
            minimum: resolutions.iter().map(|dpi| dpi[0]).min().unwrap_or(0),
            maximum: resolutions.iter().map(|dpi| dpi[0]).max().unwrap_or(0),
            discrete: Some(resolutions),
        });
        model.color_spaces = device
            .pwg_types
            .into_iter()
            .filter_map(|kind| match kind.as_str() {
                "rgb_8" => Some("rgb".into()),
                "cmyk_8" => Some("cmyk".into()),
                "black_8" => Some("gray".into()),
                _ => None,
            })
            .collect();
        model.color_spaces.sort();
        model.color_spaces.dedup();
        model.profiles = printer
            .color_target
            .iter()
            .flat_map(|target| {
                [
                    target.output_profile_sha256.clone(),
                    target.device_link_sha256.clone(),
                ]
            })
            .flatten()
            .collect();
        model.rendering_intents = vec![];
        model.media = printer
            .media
            .iter()
            .filter(|media| device.media.contains(&media.name))
            .map(|media| MediaCapability {
                name: media.name.clone(),
                width_microns: media.width_microns,
                height_microns: media.height_microns,
            })
            .collect();
        model.media.sort_by(|a, b| a.name.cmp(&b.name));
        model.sides = vec!["one-sided".into()];
        model.copies = match device.copies_supported {
            Some([min, max]) if min <= rip_core::print::MAX_DELIVERY_COPIES => {
                Some([min, max.min(rip_core::print::MAX_DELIVERY_COPIES)])
            }
            None => Some([1, 1]),
            _ => None,
        };
        model.finishings = vec!["none".into()];
        model.availability = if device.accepting_jobs
            && !model.input_media_types.is_empty()
            && !model.color_spaces.is_empty()
            && !model.media.is_empty()
            && model.copies.is_some()
            && model.dpi.as_ref().is_some_and(|dpi| dpi.minimum > 0)
        {
            Availability::Available
        } else {
            Availability::Unavailable
        };
        model.limits = json!({"artifactBytes":33554432,"maximumDimensionsMicrons":{"width":model.media.iter().map(|m|m.width_microns).max(),"height":model.media.iter().map(|m|m.height_microns).max()}});
        model.constraints = vec![
            "configured_media_and_exact_raster_geometry_required".into(),
            "explicit_color_attempt_required_when_color_target_configured".into(),
        ];
        model.extensions = json!({"org.jrip.printer":{"printerId":id,"completionMeaning":"device-confirmed","configuredColorTarget":printer.color_target,"state":device.state,"reasons":device.reasons}});
        models.push(model);
    }
    let value = serde_json::to_value(&models).map_err(internal)?;
    let revision = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&value).map_err(internal)?)
    );
    let now = rip_storage::now();
    let intersection = rip_core::print::delivery_intersection(&models);
    Ok(
        json!({"schemaVersion":1,"capabilityRevision":revision,"observedAt":now,"expiresAt":now+30,"stages":models.iter().map(|model|model.snapshot(now)).collect::<Vec<_>>(),"deliveryIntersection":intersection,"executionPaths":{"rasterExport":true,"explicitColorAttempt":true,"explicitDelivery":printer_id.is_some(),"automaticPhysicalPipeline":false},"authorizationGranted":false}),
    )
}
pub(crate) async fn validate_ticket(
    State(app): State<App>,
    headers: HeaderMap,
    Json(ticket): Json<JobTicket>,
) -> Result<Json<Value>, ApiError> {
    authorize(&app, &headers, "jobs:submit")?;
    let error = ticket.validate_raster().err();
    Ok(Json(
        json!({"valid":error.is_none(),"schemaVersion":1,"capabilityRevision":rip_core::print::RIP_REVISION,"conflicts":error.map(|error|vec![json!({"code":error.0,"retryable":false})]).unwrap_or_default(),"documentVerified":false,"authorizationGranted":false,"messages":{"en":"Pure ticket validation; no source download, structural preflight or execution","ja":"ticketの設定検証のみで、原稿取得・構造検証・実行を行いません","zh-CN":"仅验证ticket设置，不获取原稿、不做结构预检或执行"}}),
    ))
}
pub(crate) async fn ticket(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<JobTicket>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(app.repo.job_ticket(id).await?))
}
