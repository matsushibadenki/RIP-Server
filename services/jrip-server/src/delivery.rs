//! Explicit outbound delivery. Device addresses are administrator configuration only.
use super::*;
use rip_printer::{DeviceState, NativeIpp, PrinterBackend};
use rip_storage::delivery::{Delivery, DeliveryState, DeliveryTicket};
use serde::Serialize;
use sha2::{Digest, Sha256};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Printer {
    pub id: String,
    pub uri: String,
    pub media: Vec<Media>,
    #[serde(default)]
    pub color_target: Option<ColorTarget>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColorTarget {
    pub output_profile_sha256: Option<String>,
    pub device_link_sha256: Option<String>,
}
impl ColorTarget {
    fn valid(&self) -> bool {
        let digests = [&self.output_profile_sha256, &self.device_link_sha256];
        digests.iter().filter(|digest| digest.is_some()).count() == 1
            && digests.into_iter().flatten().all(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Media {
    pub name: String,
    pub width_microns: u32,
    pub height_microns: u32,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrinterSecurity {
    pub printer_id: String,
    pub bearer_token: Option<String>,
    pub ca_pem: Option<String>,
}
pub fn load_security() -> Result<Vec<PrinterSecurity>, Box<dyn std::error::Error>> {
    let Ok(path) = std::env::var("JRIP_PRINTER_SECURITY") else {
        return Ok(vec![]);
    };
    let file = std::fs::read(path)?;
    if file.len() > 256 * 1024 {
        return Err("printer security file too large".into());
    }
    let values: Vec<PrinterSecurity> = serde_json::from_slice(&file)?;
    let printers = load()?;
    let mut ids = std::collections::HashSet::new();
    for value in &values {
        if !ids.insert(&value.printer_id)
            || !printers
                .iter()
                .any(|printer| printer.id == value.printer_id)
        {
            return Err("invalid printer security destination".into());
        }
        let printer = printers
            .iter()
            .find(|printer| printer.id == value.printer_id)
            .unwrap();
        configured_backend(printer, Some(value))
            .map_err(|_| "invalid printer security settings")?;
    }
    Ok(values)
}
fn configured_backend(
    printer: &Printer,
    security: Option<&PrinterSecurity>,
) -> Result<NativeIpp, rip_printer::Error> {
    let mut backend = NativeIpp::new(&printer.uri)?;
    if let Some(security) = security {
        if let Some(pem) = &security.ca_pem {
            if pem.len() > 64 * 1024 {
                return Err(rip_printer::Error::InvalidInput);
            }
            let certificates = rustls_pemfile::certs(&mut std::io::Cursor::new(pem.as_bytes()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| rip_printer::Error::InvalidInput)?;
            backend = backend
                .with_trust_anchors(certificates.into_iter().map(|cert| cert.to_vec()).collect())?;
        }
        if let Some(token) = &security.bearer_token {
            backend = backend.with_bearer(token.clone())?;
        }
    }
    Ok(backend)
}
pub(crate) fn backend(app: &App, printer: &Printer) -> Result<NativeIpp, rip_printer::Error> {
    configured_backend(
        printer,
        app.printer_security
            .iter()
            .find(|security| security.printer_id == printer.id),
    )
}
pub(crate) async fn list(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(
        serde_json::json!({"schemaVersion":1,"printers":app.printers.iter().map(|p|
        serde_json::json!({"id":p.id,"media":p.media,"backend":"native-ipp","liveCapabilitiesChecked":false}))
        .collect::<Vec<_>>()}),
    ))
}
pub(crate) async fn capabilities(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    let printer = app
        .printers
        .iter()
        .find(|printer| printer.id == id)
        .ok_or(ApiError(StatusCode::NOT_FOUND, "not_found"))?;
    let capabilities = backend(&app, printer)
        .map_err(|_| bad("invalid_printer"))?
        .capabilities()
        .await
        .map_err(|_| bad("printer_unavailable"))?;
    let model = serde_json::json!({"schemaVersion":1,"id":printer.id,"backend":"native-ipp","capabilities":capabilities,"configuredMedia":printer.media,"configuredColorTarget":printer.color_target});
    let revision = format!(
        "sha256:{:x}",
        Sha256::digest(
            serde_json::to_vec(&serde_json::json!({"model":model,"uri":printer.uri}))
                .map_err(internal)?
        )
    );
    Ok(Json(
        serde_json::json!({"model":model,"capabilityRevision":revision,"observedAt":rip_storage::now(),"expiresAt":rip_storage::now()+30}),
    ))
}
pub fn load() -> Result<Vec<Printer>, Box<dyn std::error::Error>> {
    let Ok(path) = std::env::var("JRIP_PRINTER_CONFIG") else {
        return Ok(vec![]);
    };
    let printers: Vec<Printer> = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut ids = std::collections::HashSet::new();
    for p in &printers {
        NativeIpp::new(&p.uri)?;
        if p.color_target
            .as_ref()
            .is_some_and(|target| !target.valid())
        {
            return Err("invalid printer color target".into());
        }
        if p.id.is_empty()
            || p.id.len() > 128
            || !p
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !ids.insert(&p.id)
            || p.media.is_empty()
        {
            return Err("invalid printer configuration".into());
        }
        let mut names = std::collections::HashSet::new();
        for m in &p.media {
            if m.name.is_empty()
                || m.name.len() > 63
                || !m.name.bytes().all(|b| b.is_ascii_graphic())
                || !names.insert(&m.name)
                || !(1..=1_000_000).contains(&m.width_microns)
                || !(1..=1_000_000).contains(&m.height_microns)
            {
                return Err("invalid media configuration".into());
            }
        }
    }
    Ok(printers)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    request_id: Uuid,
    printer_id: String,
    media: String,
    expected_revision: i64,
    color_attempt_id: Option<Uuid>,
    expected_ticket_sha256: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Registration {
    ticket: DeliveryTicket,
    printer_uri: String,
}
fn profile<'a>(
    app: &'a App,
    ticket: &DeliveryTicket,
) -> Result<(&'a Printer, &'a Media), ApiError> {
    let p = app
        .printers
        .iter()
        .find(|p| p.id == ticket.printer_id)
        .ok_or(bad("printer_not_configured"))?;
    let m = p
        .media
        .iter()
        .find(|m| m.name == ticket.media)
        .ok_or(bad("media_not_configured"))?;
    Ok((p, m))
}
fn validate(bytes: &[u8], ticket: &DeliveryTicket, media: &Media) -> Result<(), ApiError> {
    if bytes.len() as u64 != ticket.artifact_bytes
        || format!("{:x}", Sha256::digest(bytes)) != ticket.artifact_sha256
    {
        return Err(bad("delivery_artifact_mismatch"));
    }
    let pages = rip_raster::inspect(bytes).map_err(|_| bad("invalid_pwg_raster"))?;
    for p in pages {
        let pixels =
            |microns: u32| ((u64::from(microns) * u64::from(ticket.dpi) + 12700) / 25400) as u32;
        let name = &bytes[p.offset + 1732..p.offset + 1796];
        let end = name.iter().position(|b| *b == 0).unwrap_or(64);
        if p.width.abs_diff(pixels(media.width_microns)) > 1
            || p.height.abs_diff(pixels(media.height_microns)) > 1
            || p.dpi != ticket.dpi
            || p.document_type != ticket.document_type
            || &name[..end] != ticket.media.as_bytes()
        {
            return Err(bad("delivery_media_mismatch"));
        }
    }
    Ok(())
}
fn publish(path: &std::path::Path, bytes: &[u8]) -> Result<(), ApiError> {
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap()).map_err(internal)?;
    tmp.write_all(bytes).map_err(internal)?;
    tmp.as_file().sync_all().map_err(internal)?;
    match tmp.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(path).map_err(internal)? == bytes {
                Ok(())
            } else {
                Err(ApiError(StatusCode::CONFLICT, "delivery_request_conflict"))
            }
        }
        Err(e) => Err(internal(e.error)),
    }
}
pub(crate) async fn prepare(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<Request>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    prepare_impl(app, headers, id, req, false).await
}
pub(crate) async fn preview(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<Request>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    prepare_impl(app, headers, id, req, true).await
}
async fn prepare_impl(
    app: App,
    headers: HeaderMap,
    id: Uuid,
    req: Request,
    preview: bool,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    authorize(
        &app,
        &headers,
        if preview {
            "jobs:read"
        } else {
            "printers:print"
        },
    )?;
    authorize(&app, &headers, "artifacts:read")?;
    if req
        .expected_ticket_sha256
        .as_ref()
        .is_some_and(|digest| !rip_core::print::valid_digest(digest))
    {
        return Err(bad("invalid_ticket_digest"));
    }
    let job = app.repo.get(id).await?;
    if job.state != JobState::Completed || job.revision != req.expected_revision {
        return Err(ApiError(StatusCode::CONFLICT, "revision_conflict"));
    }
    let color_result = if let Some(color_id) = req.color_attempt_id {
        let attempt = app.repo.color_attempt(color_id).await?;
        if attempt["state"] != "COMPLETED"
            || attempt["sourceJob"] != id.to_string()
            || attempt["ticket"]["sourceRevision"] != req.expected_revision
        {
            return Err(bad("invalid_color_attempt"));
        }
        Some(attempt["result"].clone())
    } else {
        None
    };
    let selected_color = if let Some(result) = &color_result {
        match result["colorSpace"].as_str() {
            Some("rgb") => rip_core::ColorMode::Rgb,
            Some("cmyk") => rip_core::ColorMode::Cmyk,
            Some("gray") => rip_core::ColorMode::Gray,
            _ => return Err(bad("invalid_color_attempt")),
        }
    } else {
        job.manifest.color_mode
    };
    let (color, kind) = match selected_color {
        rip_core::ColorMode::Rgb => (rip_raster::Color::DeviceRgb, "rgb_8"),
        rip_core::ColorMode::Cmyk => (rip_raster::Color::DeviceCmyk, "cmyk_8"),
        rip_core::ColorMode::Gray => (rip_raster::Color::Black, "black_8"),
    };
    let mut ticket = DeliveryTicket {
        source_job: id,
        color_attempt_id: req.color_attempt_id,
        printer_id: req.printer_id,
        artifact_sha256: String::new(),
        artifact_bytes: 0,
        dpi: job.manifest.dpi,
        document_type: kind.into(),
        media: req.media,
    };
    let (printer, media) = profile(&app, &ticket)?;
    let uri = printer.uri.clone();
    let media = media.clone();
    if printer
        .color_target
        .as_ref()
        .is_some_and(|target| !target.valid())
    {
        return Err(bad("printer_color_profile_mismatch"));
    }
    if color_result.is_none() && printer.color_target.is_some() {
        return Err(bad("color_management_required"));
    }
    let expected_icc = if let Some(result) = &color_result {
        let target = printer
            .color_target
            .as_ref()
            .ok_or(bad("printer_color_target_required"))?;
        if result["outputProfileSha256"] != serde_json::json!(target.output_profile_sha256)
            || (target.output_profile_sha256.is_none()
                && result["inputProfileSha256"] != serde_json::json!(target.device_link_sha256))
        {
            return Err(bad("printer_color_profile_mismatch"));
        }
        target.output_profile_sha256.clone()
    } else {
        None
    };
    if !preview && let Ok(existing) = app.repo.delivery(req.request_id).await {
        if existing.ticket.source_job == id
            && existing.ticket.printer_id == ticket.printer_id
            && existing.ticket.media == ticket.media
            && existing.ticket.color_attempt_id == ticket.color_attempt_id
        {
            if let Some(expected) = &req.expected_ticket_sha256 {
                let contract = app.repo.delivery_contract(req.request_id).await?;
                if contract["ticketSha256"] != *expected {
                    return Err(ApiError(StatusCode::CONFLICT, "delivery_ticket_conflict"));
                }
            }
            return Ok((
                StatusCode::ACCEPTED,
                Json(serde_json::to_value(existing).map_err(internal)?),
            ));
        }
        return Err(ApiError(StatusCode::CONFLICT, "delivery_request_conflict"));
    }
    let approval = if preview {
        None
    } else {
        approvals::check_delivery(
            &app,
            &headers,
            id,
            req.expected_revision,
            req.request_id,
            &ticket.printer_id,
            (&ticket.media, ticket.color_attempt_id),
        )
        .await?
    };
    backend(&app, printer)
        .map_err(|_| bad("invalid_printer"))?
        .capabilities()
        .await
        .map_err(|_| bad("printer_unavailable"))?
        .check_pwg(ticket.dpi, kind, &ticket.media)
        .map_err(|_| bad("unsupported_printer_profile"))?;
    let artifacts: Vec<rip_core::Artifact> =
        serde_json::from_value(if let Some(result) = &color_result {
            result["artifacts"].clone()
        } else {
            app.repo.artifacts(id).await?
        })
        .map_err(internal)?;
    if artifacts.is_empty() || artifacts.len() > 100 {
        return Err(bad("invalid_artifacts"));
    }
    if artifacts
        .iter()
        .try_fold(0u64, |sum, a| sum.checked_add(a.bytes))
        .is_none_or(|sum| sum > 32 * 1024 * 1024)
    {
        return Err(bad("delivery_input_too_large"));
    }
    let preparation = app
        .delivery_preparation
        .clone()
        .acquire_owned()
        .await
        .map_err(internal)?;
    let mut snapshots = Vec::new();
    for (index, a) in artifacts.iter().enumerate() {
        if a.page != index as u32 + 1 {
            return Err(bad("invalid_artifacts"));
        }
        snapshots.push(if let Some(color_id) = ticket.color_attempt_id {
            color::verified_page(&app, color_id, &a.name).await?
        } else {
            artifacts::verified_snapshot(&app, id, &a.name).await?
        });
    }
    let dpi = ticket.dpi;
    let media_name = ticket.media.clone();
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let _preparation = preparation;
        let paths: Vec<_> = snapshots
            .iter()
            .map(|(_, tmp, _, _)| tmp.path().to_path_buf())
            .collect();
        let mut bytes = Vec::new();
        rip_raster::convert_registered(
            &paths,
            &rip_raster::Options { dpi, color },
            expected_icc.as_deref(),
            &mut bytes,
        )
        .map_err(|e| e.to_string())?;
        for p in rip_raster::inspect(&bytes).map_err(|e| e.to_string())? {
            bytes[p.offset + 1732..p.offset + 1732 + media_name.len()]
                .copy_from_slice(media_name.as_bytes());
        }
        Ok(bytes)
    })
    .await
    .map_err(internal)?
    .map_err(|_| bad("raster_conversion_failed"))?;
    ticket.artifact_bytes = bytes.len() as u64;
    ticket.artifact_sha256 = format!("{:x}", Sha256::digest(&bytes));
    validate(&bytes, &ticket, &media)?;
    let physical = serde_json::json!({"schemaVersion":1,"version":"0.1","jobId":id,"attemptId":req.request_id,
        "sourceRevision":req.expected_revision,"sourceSha256":job.source_sha256,"manifest":job.manifest,"selectedEngine":job.selected_engine,
        "delivery":ticket,"printerConfigSha256":format!("{:x}",Sha256::digest(serde_json::to_vec(printer).map_err(internal)?)),
        "copies":1,"sides":"one-sided","pageSelection":"all","numberUp":1,"imposition":"none",
        "completionMeaning":"device-confirmed","approvalRequired":app.require_print_approval});
    let contract = serde_json::json!({"ticket":physical,"ticketSha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&physical).map_err(internal)?))});
    if req
        .expected_ticket_sha256
        .as_ref()
        .is_some_and(|expected| contract["ticketSha256"] != *expected)
    {
        return Err(ApiError(StatusCode::CONFLICT, "delivery_ticket_conflict"));
    }
    if preview {
        return Ok((
            StatusCode::OK,
            Json(
                serde_json::json!({"contract":contract,"settingsVerified":true,"artifactVerified":true,"geometryVerified":true,"documentStructureVerified":false,"authorizationGranted":false,"queued":false,
            "messages":{"en":"Verified delivery preview; commit rechecks capabilities and approval","ja":"送信設定・成果物を確認しました。投入時に能力と承認を再検査します","zh-CN":"已验证投递设置和产物；提交时重新检查能力与审批"}}),
            ),
        ));
    }
    let dir = app.root.join("deliveries");
    tokio::fs::create_dir_all(&dir).await.map_err(internal)?;
    let registration = serde_json::to_vec(&Registration {
        ticket: ticket.clone(),
        printer_uri: uri,
    })
    .map_err(internal)?;
    let request_id = req.request_id;
    tokio::task::spawn_blocking(move || {
        publish(&dir.join(format!("{request_id}.pwg")), &bytes)?;
        publish(&dir.join(format!("{request_id}.json")), &registration)
    })
    .await
    .map_err(internal)??;
    Ok((
        StatusCode::ACCEPTED,
        Json(
            serde_json::to_value(
                app.repo
                    .enqueue_contract_delivery(req.request_id, ticket, approval, Some(contract))
                    .await?,
            )
            .map_err(internal)?,
        ),
    ))
}
pub(crate) async fn contract(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(app.repo.delivery_contract(id).await?))
}
pub(crate) async fn detail(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Delivery>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(app.repo.delivery(id).await?))
}
async fn read(app: &App, id: Uuid, ext: &str, max: u64) -> Result<Vec<u8>, ApiError> {
    use tokio::io::AsyncReadExt;
    let path = app.root.join("deliveries").join(format!("{id}.{ext}"));
    if tokio::fs::canonicalize(&path).await.map_err(internal)? != path {
        return Err(bad("invalid_delivery_path"));
    }
    let file = tokio::fs::File::open(path).await.map_err(internal)?;
    let meta = file.metadata().await.map_err(internal)?;
    if !meta.is_file() || meta.len() > max {
        return Err(bad("invalid_delivery_file"));
    }
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(internal)?;
    if bytes.len() as u64 > max {
        return Err(bad("invalid_delivery_file"));
    }
    Ok(bytes)
}
async fn registered(app: &App, d: &Delivery) -> Result<(NativeIpp, Media), ApiError> {
    let r: Registration =
        serde_json::from_slice(&read(app, d.id, "json", 16384).await?).map_err(internal)?;
    let (p, m) = profile(app, &d.ticket)?;
    if r.ticket != d.ticket || r.printer_uri != p.uri {
        return Err(bad("delivery_registration_mismatch"));
    }
    Ok((
        backend(app, p).map_err(|_| bad("invalid_printer"))?,
        m.clone(),
    ))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CancelRequest {
    expected_revision: i64,
}
pub(crate) async fn cancel(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<CancelRequest>,
) -> Result<(StatusCode, Json<Delivery>), ApiError> {
    authorize(&app, &headers, "printers:print")?;
    let d = app
        .repo
        .request_delivery_cancel(id, req.expected_revision)
        .await?;
    Ok((
        if d.state == DeliveryState::Cancelled {
            StatusCode::OK
        } else {
            StatusCode::ACCEPTED
        },
        Json(d),
    ))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReconcileRequest {
    expected_revision: i64,
    device_job_id: i32,
}
pub(crate) async fn reconcile(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<ReconcileRequest>,
) -> Result<Json<Delivery>, ApiError> {
    authorize(&app, &headers, "admin")?;
    let delivery = app.repo.delivery(id).await?;
    if delivery.state != DeliveryState::Unknown
        || delivery.revision != req.expected_revision
        || req.device_job_id <= 0
    {
        return Err(ApiError(StatusCode::CONFLICT, "state_conflict"));
    }
    let (printer, _) = registered(&app, &delivery).await?;
    let status = printer
        .job_status(req.device_job_id)
        .await
        .map_err(|_| bad("remote_job_unavailable"))?;
    if status.job_name.as_deref() != Some(format!("J-RIP {id}").as_str()) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "remote_job_identity_mismatch",
        ));
    }
    let next = match status.state {
        DeviceState::Completed => DeliveryState::Completed,
        DeviceState::Cancelled => DeliveryState::Cancelled,
        DeviceState::Aborted => DeliveryState::Aborted,
        _ => DeliveryState::Accepted,
    };
    Ok(Json(
        app.repo
            .reconcile_delivery(id, req.expected_revision, req.device_job_id, next)
            .await?,
    ))
}
/// Run one bounded dispatch pass. Unknown outcomes are never automatically retried.
pub async fn tick(app: &App) -> Result<(), StorageError> {
    app.repo.recover_delivery_sends().await?;
    if let Some(d) = app.repo.next_delivery().await? {
        let run = Uuid::new_v4();
        if app.repo.claim_delivery(d.id, d.revision, run).await.is_ok() {
            let result = async {
                let (printer, media) = registered(app, &d)
                    .await
                    .map_err(|_| rip_printer::Error::InvalidInput)?;
                let bytes = read(app, d.id, "pwg", 32 * 1024 * 1024)
                    .await
                    .map_err(|_| rip_printer::Error::InvalidInput)?;
                validate(&bytes, &d.ticket, &media)
                    .map_err(|_| rip_printer::Error::InvalidInput)?;
                printer
                    .submit_pwg(
                        &format!("J-RIP {}", d.id),
                        bytes,
                        d.ticket.dpi,
                        &d.ticket.document_type,
                        &d.ticket.media,
                    )
                    .await
            }
            .await;
            let (state, device, reason) = match result {
                Ok(job) => (DeliveryState::Accepted, Some(job.job_id), None),
                Err(rip_printer::Error::OutcomeUnknown) => {
                    (DeliveryState::Unknown, None, Some("OUTCOME_UNKNOWN".into()))
                }
                Err(e) => (DeliveryState::Rejected, None, Some(e.to_string())),
            };
            app.repo
                .finish_delivery(d.id, run, state, device, reason)
                .await?;
        }
    }
    for mut d in app.repo.accepted_deliveries().await? {
        let Ok((printer, _)) = registered(app, &d).await else {
            continue;
        };
        if d.reason.as_deref() == Some("CANCEL_REQUESTED") {
            match app.repo.claim_delivery_cancel(d.id, d.revision).await {
                Ok(claimed) => {
                    d = claimed;
                    if let Err(error) = printer.cancel(d.device_job_id.unwrap()).await {
                        tracing::warn!(delivery_id=%d.id,%error,"cancel result uncertain; continue device polling");
                    }
                }
                Err(StorageError::Conflict) => continue,
                Err(e) => return Err(e),
            }
        }
        let Ok(status) = printer.job_status(d.device_job_id.unwrap()).await else {
            continue;
        };
        let next = match status.state {
            DeviceState::Completed => DeliveryState::Completed,
            DeviceState::Cancelled => DeliveryState::Cancelled,
            DeviceState::Aborted => DeliveryState::Aborted,
            _ => continue,
        };
        match app.repo.observe_delivery(d.id, d.revision, next).await {
            Ok(_) | Err(StorageError::Conflict) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub(crate) async fn result_receipt(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    Ok(Json(app.repo.delivery_result_receipt(id).await?))
}
