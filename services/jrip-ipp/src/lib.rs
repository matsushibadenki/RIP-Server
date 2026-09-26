//! PDF-only IPP proof-printer frontend. It does not claim IPP Everywhere conformance.
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use rip_core::{ColorMode, DocumentFormat, EnginePreference, Job, JobState, Manifest};
use rip_ipp::{self as ipp, Request, Response};
use rip_storage::{Repository, StorageError};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct App {
    pub repo: Repository,
    pub root: PathBuf,
    pub printer_uri: Arc<str>,
}
pub async fn open(root: PathBuf, printer_uri: String) -> Result<App, Box<dyn std::error::Error>> {
    if !printer_uri.starts_with("ipp://") || !printer_uri.ends_with("/ipp/print") {
        return Err("Invalid JRIP_PRINTER_URI".into());
    }
    tokio::fs::create_dir_all(root.join("jobs")).await?;
    let root = tokio::fs::canonicalize(root).await?;
    let repo = Repository::open(&root.join("jobs.sqlite3")).await?;
    Ok(App {
        repo,
        root,
        printer_uri: printer_uri.into(),
    })
}
/// Build only the generic IPP service. PDF-only proof output is not AirPrint or IPP Everywhere.
pub fn discovery_info(
    listen: SocketAddr,
    printer_uri: &str,
    hostname: &str,
) -> Result<mdns_sd::ServiceInfo, Box<dyn std::error::Error>> {
    let uri = url::Url::parse(printer_uri)?;
    let host = uri.host_str().ok_or("JRIP_PRINTER_URI needs a host")?;
    let host_ip: std::net::IpAddr = host
        .parse()
        .map_err(|_| "JRIP_PRINTER_URI must use a LAN IP address while DNS-SD is enabled")?;
    if uri.scheme() != "ipp"
        || uri.path() != "/ipp/print"
        || uri.query().is_some()
        || uri.fragment().is_some()
        || !uri.username().is_empty()
        || uri.password().is_some()
        || host_ip.is_loopback()
        || host_ip.is_unspecified()
        || listen.ip().is_loopback()
        || (!listen.ip().is_unspecified() && listen.ip() != host_ip)
        || uri.port_or_known_default() != Some(listen.port())
    {
        return Err("DNS-SD requires a reachable IPP URI matching the LAN listener".into());
    }
    if !hostname.ends_with(".local.") {
        return Err("JRIP_MDNS_HOSTNAME must end in .local.".into());
    }
    let properties = [
        ("txtvers", "1"),
        ("rp", "ipp/print"),
        ("pdl", "application/pdf"),
        ("ty", "J-RIP PDF Proof"),
        ("qtotal", "1"),
    ];
    Ok(mdns_sd::ServiceInfo::new(
        "_ipp._tcp.local.",
        "J-RIP PDF Proof",
        hostname,
        host_ip,
        listen.port(),
        &properties[..],
    )?)
}
pub fn router(app: App) -> Router {
    Router::new()
        .route("/ipp/print", post(handle))
        .route("/ipp/print/jobs/{id}", post(handle_job_uri))
        .layer(DefaultBodyLimit::max(
            rip_jobs::MAX_DOCUMENT_BYTES as usize + 16384,
        ))
        .with_state(app)
}
type IppReply = (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    Vec<u8>,
);
async fn handle(State(app): State<App>, headers: HeaderMap, body: Bytes) -> IppReply {
    handle_request(app, headers, body, None).await
}
async fn handle_job_uri(
    State(app): State<App>,
    Path(id): Path<i32>,
    headers: HeaderMap,
    body: Bytes,
) -> IppReply {
    handle_request(app, headers, body, Some(id)).await
}
async fn handle_request(
    app: App,
    headers: HeaderMap,
    body: Bytes,
    path_id: Option<i32>,
) -> IppReply {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type != "application/ipp" {
        tracing::warn!(content_type, "unsupported IPP content type");
        return reply_http(StatusCode::UNSUPPORTED_MEDIA_TYPE, Vec::new());
    }
    let req = match Request::parse(&body) {
        Ok(req) => req,
        Err(_) => {
            tracing::warn!(length = body.len(), "invalid IPP wire message");
            return reply_http(StatusCode::BAD_REQUEST, Vec::new());
        }
    };
    let mut status = req.validate_operation();
    let target_id = if let Some(id) = path_id {
        let expected = format!("{}/jobs/{id}", app.printer_uri);
        if !matches!(req.operation, ipp::GET_JOB_ATTRIBUTES | ipp::CANCEL_JOB)
            || req.text(1, "job-uri", 0x45) != Some(expected.as_str())
        {
            status = Err(ipp::BAD_REQUEST);
        }
        Some(id)
    } else {
        if req.text(1, "printer-uri", 0x45) != Some(app.printer_uri.as_ref()) {
            status = Err(ipp::BAD_REQUEST);
        }
        None
    };
    let mut response = Response::new(req.version, status.err().unwrap_or(ipp::OK), req.request_id);
    response.text(0x47, "attributes-charset", "utf-8");
    response.text(0x48, "attributes-natural-language", "en");
    if status.is_ok() {
        let result = match req.operation {
            ipp::GET_PRINTER_ATTRIBUTES => {
                printer_attributes(&mut response, &app, &req);
                Ok(())
            }
            ipp::PRINT_JOB => print_job(&app, &req, &mut response).await,
            ipp::VALIDATE_JOB => validate_job(&req),
            ipp::GET_JOB_ATTRIBUTES => job_info(&app, &req, target_id, &mut response).await,
            ipp::GET_JOBS => list_jobs(&app, &req, &mut response).await,
            ipp::CANCEL_JOB => cancel(&app, &req, target_id).await,
            _ => Ok(()),
        };
        if let Err(code) = result {
            response = Response::new(req.version, code, req.request_id);
            response.text(0x47, "attributes-charset", "utf-8");
            response.text(0x48, "attributes-natural-language", "en");
        }
    }
    reply_http(StatusCode::OK, response.finish())
}
fn reply_http(
    status: StatusCode,
    body: Vec<u8>,
) -> (
    StatusCode,
    [(axum::http::header::HeaderName, &'static str); 1],
    Vec<u8>,
) {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/ipp")],
        body,
    )
}
fn printer_attributes(response: &mut Response, app: &App, req: &Request<'_>) {
    response.group(4);
    macro_rules! attr {
        ($name:literal, $emit:expr) => {
            if req.wants($name, "printer-description") {
                $emit;
            }
        };
    }
    macro_rules! template_attr {
        ($name:literal, $emit:expr) => {
            if req.wants($name, "job-template") {
                $emit;
            }
        };
    }
    attr!(
        "printer-uri-supported",
        response.text(0x45, "printer-uri-supported", &app.printer_uri)
    );
    attr!(
        "printer-name",
        response.text(0x42, "printer-name", "J-RIP PDF Proof")
    );
    attr!(
        "document-format-supported",
        response.text(0x49, "document-format-supported", "application/pdf")
    );
    attr!(
        "document-format-default",
        response.text(0x49, "document-format-default", "application/pdf")
    );
    attr!(
        "charset-configured",
        response.text(0x47, "charset-configured", "utf-8")
    );
    attr!(
        "charset-supported",
        response.text(0x47, "charset-supported", "utf-8")
    );
    attr!(
        "natural-language-configured",
        response.text(0x48, "natural-language-configured", "en")
    );
    attr!(
        "generated-natural-language-supported",
        response.text(0x48, "generated-natural-language-supported", "en")
    );
    attr!(
        "compression-supported",
        response.text(0x44, "compression-supported", "none")
    );
    attr!(
        "uri-authentication-supported",
        response.text(0x44, "uri-authentication-supported", "none")
    );
    attr!(
        "uri-security-supported",
        response.text(0x44, "uri-security-supported", "none")
    );
    attr!(
        "printer-info",
        response.text(0x41, "printer-info", "PDF proof to TIFF file")
    );
    attr!(
        "printer-make-and-model",
        response.text(0x41, "printer-make-and-model", "J-RIP PDF Proof")
    );
    attr!(
        "ipp-versions-supported",
        response.text(0x44, "ipp-versions-supported", "1.1")
    );
    if req.wants("operations-supported", "printer-description") {
        response.integer("operations-supported", i32::from(ipp::PRINT_JOB));
        for operation in [
            ipp::GET_PRINTER_ATTRIBUTES,
            ipp::VALIDATE_JOB,
            ipp::GET_JOB_ATTRIBUTES,
            ipp::GET_JOBS,
            ipp::CANCEL_JOB,
        ] {
            response.additional(0x21, &i32::from(operation).to_be_bytes());
        }
    }
    attr!("printer-state", response.integer("printer-state", 3));
    attr!(
        "printer-is-accepting-jobs",
        response.attribute(0x22, "printer-is-accepting-jobs", &[1])
    );
    attr!(
        "printer-state-reasons",
        response.text(0x44, "printer-state-reasons", "none")
    );
    template_attr!(
        "printer-resolution-default",
        response.resolution_dpi("printer-resolution-default", 600)
    );
    if req.wants("printer-resolution-supported", "job-template") {
        response.resolution_dpi("printer-resolution-supported", 300);
        let mut value = [0u8; 9];
        value[0..4].copy_from_slice(&600u32.to_be_bytes());
        value[4..8].copy_from_slice(&600u32.to_be_bytes());
        value[8] = 3;
        response.additional(0x32, &value);
    }
    template_attr!(
        "print-color-mode-default",
        response.text(0x44, "print-color-mode-default", "color")
    );
    if req.wants("print-color-mode-supported", "job-template") {
        response.text(0x44, "print-color-mode-supported", "color");
        response.additional(0x44, b"monochrome");
    }
}
async fn print_job(app: &App, req: &Request<'_>, response: &mut Response) -> Result<(), u16> {
    if req.document.len() as u64 > rip_jobs::MAX_DOCUMENT_BYTES {
        return Err(0x0408);
    }
    let manifest = manifest_for(req);
    manifest.validate().map_err(|_| ipp::BAD_REQUEST)?;
    let job = rip_jobs::submit_bytes(&app.repo, &app.root, manifest, req.document)
        .await
        .map_err(|e| match e {
            rip_jobs::SubmitError::Invalid("document_too_large") => 0x0408,
            rip_jobs::SubmitError::Invalid(_) => ipp::FORMAT_ERROR,
            e => {
                tracing::error!(error=%e,"IPP job publication failed");
                ipp::INTERNAL_ERROR
            }
        })?;
    let ipp_id = app.repo.register_ipp_job(job.id).await.map_err(|e| {
        tracing::error!(error=%e,"IPP job mapping failed");
        ipp::INTERNAL_ERROR
    })?;
    response.group(2);
    job_attributes(response, &job, ipp_id, &app.printer_uri, None, false);
    Ok(())
}
fn validate_job(req: &Request<'_>) -> Result<(), u16> {
    manifest_for(req).validate().map_err(|_| ipp::BAD_REQUEST)
}
fn manifest_for(req: &Request<'_>) -> Manifest {
    Manifest {
        name: req
            .text(1, "job-name", 0x42)
            .unwrap_or("document.pdf")
            .to_string(),
        format: DocumentFormat::Pdf,
        dpi: req.resolution_dpi(2, "printer-resolution").unwrap_or(600),
        color_mode: if req.text(2, "print-color-mode", 0x44) == Some("monochrome") {
            ColorMode::Gray
        } else {
            ColorMode::Cmyk
        },
        priority: 50,
        engine: EnginePreference::Auto,
    }
}
async fn job_info(
    app: &App,
    req: &Request<'_>,
    target_id: Option<i32>,
    response: &mut Response,
) -> Result<(), u16> {
    let id = target_id
        .or_else(|| req.integer(1, "job-id"))
        .ok_or(ipp::BAD_REQUEST)?;
    let job = app.repo.get_ipp_job(id).await.map_err(map_storage)?;
    response.group(2);
    job_attributes(response, &job, id, &app.printer_uri, Some(req), false);
    Ok(())
}
async fn list_jobs(app: &App, req: &Request<'_>, response: &mut Response) -> Result<(), u16> {
    let completed = req.text(1, "which-jobs", 0x44) == Some("completed");
    let mut jobs = app.repo.list_ipp_jobs().await.map_err(map_storage)?;
    jobs.retain(|(_, job)| job.state.terminal() == completed);
    if completed {
        jobs.sort_by_key(|(id, job)| (std::cmp::Reverse(job.updated_at), std::cmp::Reverse(*id)));
    } else {
        jobs.sort_by_key(|(id, job)| {
            (
                std::cmp::Reverse(job.manifest.priority),
                job.created_at,
                *id,
            )
        });
    }
    let limit = req.integer(1, "limit").map_or(usize::MAX, |n| n as usize);
    let minimal = !req
        .attributes
        .iter()
        .any(|a| a.name == "requested-attributes");
    for (id, job) in jobs.into_iter().take(limit) {
        response.group(2);
        job_attributes(response, &job, id, &app.printer_uri, Some(req), minimal);
    }
    Ok(())
}
async fn cancel(app: &App, req: &Request<'_>, target_id: Option<i32>) -> Result<(), u16> {
    let id = target_id
        .or_else(|| req.integer(1, "job-id"))
        .ok_or(ipp::BAD_REQUEST)?;
    let job = app.repo.get_ipp_job(id).await.map_err(map_storage)?;
    app.repo
        .transition(job, JobState::Cancelled, None)
        .await
        .map_err(map_storage)?;
    Ok(())
}
fn map_storage(e: StorageError) -> u16 {
    match e {
        StorageError::NotFound => ipp::NOT_FOUND,
        StorageError::Conflict => 0x0405,
        e => {
            tracing::error!(error=%e,"IPP storage error");
            ipp::INTERNAL_ERROR
        }
    }
}
fn job_attributes(
    response: &mut Response,
    job: &Job,
    id: i32,
    uri: &str,
    req: Option<&Request<'_>>,
    minimal: bool,
) {
    macro_rules! attr {
        ($name:literal, $emit:expr) => {
            if (!minimal || matches!($name, "job-id" | "job-uri"))
                && req.is_none_or(|request| request.wants($name, "job-description"))
            {
                $emit;
            }
        };
    }
    attr!("job-id", response.integer("job-id", id));
    attr!(
        "job-uri",
        response.text(0x45, "job-uri", &format!("{uri}/jobs/{id}"))
    );
    attr!(
        "job-printer-uri",
        response.text(0x45, "job-printer-uri", uri)
    );
    attr!(
        "job-name",
        response.text(0x42, "job-name", &job.manifest.name)
    );
    let state = match job.state {
        JobState::Queued | JobState::Received | JobState::Validating | JobState::Preflight => 3,
        JobState::Held => 4,
        JobState::Ripping
        | JobState::ColorProcessing
        | JobState::Halftoning
        | JobState::Spooling
        | JobState::Printing => 5,
        JobState::Cancelled => 7,
        JobState::Failed => 8,
        JobState::Completed => 9,
    };
    attr!("job-state", response.integer("job-state", state));
    attr!(
        "job-state-reasons",
        response.text(
            0x44,
            "job-state-reasons",
            match job.state {
                JobState::Cancelled => "job-canceled-by-user",
                JobState::Failed => "aborted-by-system",
                _ => "none",
            },
        )
    );
    if !minimal && req.is_none_or(|request| request.wants("printer-resolution", "job-template")) {
        response.resolution_dpi("printer-resolution", job.manifest.dpi);
    }
    if !minimal && req.is_none_or(|request| request.wants("print-color-mode", "job-template")) {
        response.text(
            0x44,
            "print-color-mode",
            if job.manifest.color_mode == ColorMode::Gray {
                "monochrome"
            } else {
                "color"
            },
        );
    }
}
