//! Native IPP outbound boundary. Acceptance is distinct from physical completion.
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use rip_ipp::{self as ipp, Request as Message, Response as Encoder};
use serde::Serialize;
use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};
use url::Url;
const MAX_DOCUMENT: usize = 32 * 1024 * 1024;
const MAX_REPLY: usize = 64 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid_printer_uri")]
    InvalidUri,
    #[error("invalid_document_or_job")]
    InvalidInput,
    #[error("unsupported_document_format")]
    UnsupportedFormat,
    #[error("printer_not_accepting_jobs")]
    NotAccepting,
    #[error("printer_transport_failure")]
    Transport,
    #[error("delivery_outcome_unknown_do_not_automatically_resend")]
    OutcomeUnknown,
    #[error("invalid_printer_response")]
    InvalidResponse,
    #[error("printer_ipp_status_{0:04x}")]
    Status(u16),
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub document_formats: Vec<String>,
    pub accepting_jobs: bool,
    pub state: i32,
    pub reasons: Vec<String>,
    pub pwg_resolutions: Vec<[u32; 2]>,
    pub pwg_types: Vec<String>,
    pub media: Vec<String>,
}
impl Capabilities {
    /// Require exact matches. Never substitute DPI, color, or media silently.
    pub fn check_pwg(&self, dpi: u32, document_type: &str, media: &str) -> Result<(), Error> {
        if !self.accepting_jobs {
            return Err(Error::NotAccepting);
        }
        if !self
            .document_formats
            .iter()
            .any(|v| v == "image/pwg-raster")
            || !self.pwg_resolutions.contains(&[dpi, dpi])
            || !self.pwg_types.iter().any(|v| v == document_type)
            || !self.media.iter().any(|v| v == media)
        {
            return Err(Error::UnsupportedFormat);
        }
        Ok(())
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedJob {
    pub job_id: i32,
    pub state: DeviceState,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Pending,
    Held,
    Processing,
    Stopped,
    Cancelled,
    Aborted,
    Completed,
}
impl DeviceState {
    fn from_code(code: i32) -> Result<Self, Error> {
        match code {
            3 => Ok(Self::Pending),
            4 => Ok(Self::Held),
            5 => Ok(Self::Processing),
            6 => Ok(Self::Stopped),
            7 => Ok(Self::Cancelled),
            8 => Ok(Self::Aborted),
            9 => Ok(Self::Completed),
            _ => Err(Error::InvalidResponse),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub job_id: i32,
    pub job_name: Option<String>,
    pub state: DeviceState,
    pub reasons: Vec<String>,
}
pub trait PrinterBackend {
    fn capabilities(&self)
    -> impl std::future::Future<Output = Result<Capabilities, Error>> + Send;
    fn submit(
        &self,
        name: &str,
        mime: &str,
        document: Vec<u8>,
    ) -> impl std::future::Future<Output = Result<AcceptedJob, Error>> + Send;
    fn job_status(
        &self,
        job_id: i32,
    ) -> impl std::future::Future<Output = Result<JobStatus, Error>> + Send;
    fn cancel(&self, job_id: i32) -> impl std::future::Future<Output = Result<(), Error>> + Send;
}
pub struct NativeIpp {
    uri: Url,
    ids: AtomicU32,
    timeout: Duration,
    tls: Option<std::sync::Arc<tokio_rustls::rustls::ClientConfig>>,
    bearer: Option<String>,
}
impl NativeIpp {
    /// Endpoints come from administrator configuration, never from Agent-supplied job input.
    /// IPPS verifies both the certificate chain and server name. Credentials require TLS.
    pub fn new(uri: &str) -> Result<Self, Error> {
        let uri = Url::parse(uri).map_err(|_| Error::InvalidUri)?;
        if !matches!(uri.scheme(), "ipp" | "ipps")
            || uri.host_str().is_none()
            || uri.path().is_empty()
            || uri.query().is_some()
            || uri.fragment().is_some()
            || !uri.username().is_empty()
            || uri.password().is_some()
            || uri.as_str().len() > 2048
        {
            return Err(Error::InvalidUri);
        }
        let tls = if uri.scheme() == "ipps" {
            let mut roots = tokio_rustls::rustls::RootCertStore::empty();
            let certificates = rustls_native_certs::load_native_certs();
            for certificate in certificates.certs {
                let _ = roots.add(certificate);
            }
            Some(std::sync::Arc::new(
                tokio_rustls::rustls::ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            ))
        } else {
            None
        };
        Ok(Self {
            tls,
            bearer: None,
            uri,
            ids: AtomicU32::new(1),
            timeout: Duration::from_secs(20),
        })
    }
    fn message(&self, operation: u16) -> (u32, Encoder) {
        let id = loop {
            let id = self.ids.fetch_add(1, Ordering::Relaxed);
            if id != 0 {
                break id;
            }
        };
        let mut message = Encoder::new([1, 1], operation, id);
        message.text(0x47, "attributes-charset", "utf-8");
        message.text(0x48, "attributes-natural-language", "en");
        message.text(0x45, "printer-uri", self.uri.as_str());
        message.text(0x42, "requesting-user-name", "jrip-server");
        (id, message)
    }
    async fn exchange(&self, operation: u16, id: u32, body: Vec<u8>) -> Result<Vec<u8>, Error> {
        let result = tokio::time::timeout(self.timeout, self.http(body)).await;
        let bytes = match result {
            Ok(Ok(bytes)) => bytes,
            _ if operation == ipp::PRINT_JOB => return Err(Error::OutcomeUnknown),
            _ => return Err(Error::Transport),
        };
        let message = Message::parse(&bytes).map_err(|_| {
            if operation == ipp::PRINT_JOB {
                Error::OutcomeUnknown
            } else {
                Error::InvalidResponse
            }
        })?;
        if message.request_id != id || !message.document.is_empty() {
            return Err(if operation == ipp::PRINT_JOB {
                Error::OutcomeUnknown
            } else {
                Error::InvalidResponse
            });
        }
        // Never accept 'successful-ok-ignored-or-substituted-attributes' as fidelity success.
        if message.operation != ipp::OK {
            return Err(
                if operation == ipp::PRINT_JOB && message.operation < 0x0400 {
                    Error::OutcomeUnknown
                } else {
                    Error::Status(message.operation)
                },
            );
        }
        if message.text(1, "attributes-charset", 0x47) != Some("utf-8") {
            return Err(if operation == ipp::PRINT_JOB {
                Error::OutcomeUnknown
            } else {
                Error::InvalidResponse
            });
        }
        Ok(bytes)
    }
    /// Administrator-provided DER trust anchors; never disables certificate/name validation.
    pub fn with_trust_anchors(mut self, certificates: Vec<Vec<u8>>) -> Result<Self, Error> {
        if self.uri.scheme() != "ipps" || certificates.is_empty() || certificates.len() > 32 {
            return Err(Error::InvalidInput);
        }
        let mut roots = tokio_rustls::rustls::RootCertStore::empty();
        for certificate in certificates {
            if certificate.len() > 64 * 1024 {
                return Err(Error::InvalidInput);
            }
            roots
                .add(tokio_rustls::rustls::pki_types::CertificateDer::from(
                    certificate,
                ))
                .map_err(|_| Error::InvalidInput)?;
        }
        self.tls = Some(std::sync::Arc::new(
            tokio_rustls::rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        ));
        Ok(self)
    }
    pub fn with_bearer(mut self, token: String) -> Result<Self, Error> {
        if self.uri.scheme() != "ipps"
            || token.is_empty()
            || token.len() > 4096
            || token.bytes().any(|b| !b.is_ascii_graphic())
        {
            return Err(Error::InvalidInput);
        }
        self.bearer = Some(token);
        Ok(self)
    }
    async fn http(&self, body: Vec<u8>) -> Result<Vec<u8>, Error> {
        let host = self
            .uri
            .host_str()
            .ok_or(Error::InvalidUri)?
            .trim_start_matches('[')
            .trim_end_matches(']');
        let port = self.uri.port().unwrap_or(631);
        let stream = tokio::net::TcpStream::connect((host, port))
            .await
            .map_err(|_| Error::Transport)?;
        trait Transport: tokio::io::AsyncRead + tokio::io::AsyncWrite {}
        impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite> Transport for T {}
        let stream: Box<dyn Transport + Unpin + Send> = if let Some(config) = &self.tls {
            let name = tokio_rustls::rustls::pki_types::ServerName::try_from(host.to_string())
                .map_err(|_| Error::InvalidUri)?;
            Box::new(
                tokio_rustls::TlsConnector::from(config.clone())
                    .connect(name, stream)
                    .await
                    .map_err(|_| Error::Transport)?,
            )
        } else {
            Box::new(stream)
        };
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|_| Error::Transport)?;
        let task = tokio::spawn(async move {
            let _ = connection.await;
        });
        struct ConnectionTask(tokio::task::JoinHandle<()>);
        impl Drop for ConnectionTask {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _connection = ConnectionTask(task);
        let authority = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let mut request = hyper::Request::post(self.uri.path());
        if let Some(token) = &self.bearer {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let request = request
            .header("host", authority)
            .header("content-type", "application/ipp")
            .header("connection", "close")
            .body(Full::new(Bytes::from(body)))
            .map_err(|_| Error::Transport)?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|_| Error::Transport)?;
        if response.status() != hyper::StatusCode::OK
            || response
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .is_none_or(|s| s.split(';').next() != Some("application/ipp"))
        {
            return Err(Error::InvalidResponse);
        }
        let mut body = response.into_body();
        let mut reply = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|_| Error::Transport)?;
            if let Some(bytes) = frame.data_ref() {
                if reply.len() + bytes.len() > MAX_REPLY {
                    return Err(Error::InvalidResponse);
                }
                reply.extend_from_slice(bytes);
            }
        }
        Ok(reply)
    }
}
impl NativeIpp {
    pub async fn submit_pwg(
        &self,
        name: &str,
        document: Vec<u8>,
        dpi: u32,
        document_type: &str,
        media: &str,
    ) -> Result<AcceptedJob, Error> {
        if name.is_empty()
            || name.len() > 255
            || name.chars().any(char::is_control)
            || media.is_empty()
            || media.len() > 63
        {
            return Err(Error::InvalidInput);
        }
        let pages = rip_raster::inspect(&document).map_err(|_| Error::InvalidInput)?;
        for page in pages {
            let media_name = &document[page.offset + 1732..page.offset + 1796];
            let end = media_name
                .iter()
                .position(|b| *b == 0)
                .ok_or(Error::InvalidInput)?;
            if page.dpi != dpi
                || page.document_type != document_type
                || media_name[..end] != *media.as_bytes()
            {
                return Err(Error::InvalidInput);
            }
        }
        self.capabilities()
            .await?
            .check_pwg(dpi, document_type, media)?;
        self.send_document(name, "image/pwg-raster", document, Some(media))
            .await
    }
    async fn send_document(
        &self,
        name: &str,
        mime: &str,
        document: Vec<u8>,
        media: Option<&str>,
    ) -> Result<AcceptedJob, Error> {
        let (id, mut message) = self.message(ipp::PRINT_JOB);
        message.text(0x49, "document-format", mime);
        message.text(0x42, "job-name", name);
        message.attribute(0x22, "ipp-attribute-fidelity", &[1]);
        if let Some(media) = media {
            message.group(2);
            message.text(0x44, "media", media);
        }
        let mut body = message.finish();
        body.extend_from_slice(&document);
        drop(document);
        let bytes = self.exchange(ipp::PRINT_JOB, id, body).await?;
        let message = Message::parse(&bytes).map_err(|_| Error::OutcomeUnknown)?;
        let job_id = message
            .integer(2, "job-id")
            .filter(|id| *id > 0)
            .ok_or(Error::OutcomeUnknown)?;
        let device_state = DeviceState::from_code(
            state(&message, 2, "job-state").map_err(|_| Error::OutcomeUnknown)?,
        )
        .map_err(|_| Error::OutcomeUnknown)?;
        Ok(AcceptedJob {
            job_id,
            state: device_state,
        })
    }
}

fn values(message: &Message<'_>, group: u8, name: &str, tag: u8) -> Result<Vec<String>, Error> {
    message
        .attributes
        .iter()
        .filter(|a| a.group == group && a.name == name)
        .map(|a| {
            if a.tag != tag {
                return Err(Error::InvalidResponse);
            }
            std::str::from_utf8(&a.value)
                .map(str::to_string)
                .map_err(|_| Error::InvalidResponse)
        })
        .collect()
}
fn state(message: &Message<'_>, group: u8, name: &str) -> Result<i32, Error> {
    let attribute = message.one(group, name).ok_or(Error::InvalidResponse)?;
    if attribute.tag != 0x23 || attribute.value.len() != 4 {
        return Err(Error::InvalidResponse);
    }
    Ok(i32::from_be_bytes(
        attribute.value.as_slice().try_into().unwrap(),
    ))
}
impl PrinterBackend for NativeIpp {
    async fn capabilities(&self) -> Result<Capabilities, Error> {
        let (id, mut message) = self.message(ipp::GET_PRINTER_ATTRIBUTES);
        message.text(0x44, "requested-attributes", "document-format-supported");
        for name in [
            "printer-is-accepting-jobs",
            "printer-state",
            "printer-state-reasons",
            "pwg-raster-document-resolution-supported",
            "pwg-raster-document-type-supported",
            "media-supported",
        ] {
            message.additional(0x44, name.as_bytes());
        }
        let bytes = self
            .exchange(ipp::GET_PRINTER_ATTRIBUTES, id, message.finish())
            .await?;
        let message = Message::parse(&bytes).map_err(|_| Error::InvalidResponse)?;
        let formats = values(&message, 4, "document-format-supported", 0x49)?;
        if formats.is_empty() {
            return Err(Error::InvalidResponse);
        }
        let accepting = message
            .one(4, "printer-is-accepting-jobs")
            .ok_or(Error::InvalidResponse)?;
        if accepting.tag != 0x22 || !matches!(accepting.value.as_slice(), [0] | [1]) {
            return Err(Error::InvalidResponse);
        }
        let printer_state = state(&message, 4, "printer-state")?;
        if !(3..=5).contains(&printer_state) {
            return Err(Error::InvalidResponse);
        }
        let mut pwg_resolutions = Vec::new();
        for attribute in message
            .attributes
            .iter()
            .filter(|a| a.group == 4 && a.name == "pwg-raster-document-resolution-supported")
        {
            if attribute.tag != 0x32 || attribute.value.len() != 9 || attribute.value[8] != 3 {
                return Err(Error::InvalidResponse);
            }
            let x = i32::from_be_bytes(attribute.value[..4].try_into().unwrap());
            let y = i32::from_be_bytes(attribute.value[4..8].try_into().unwrap());
            if x <= 0 || y <= 0 {
                return Err(Error::InvalidResponse);
            }
            pwg_resolutions.push([x as u32, y as u32]);
        }
        Ok(Capabilities {
            document_formats: formats,
            accepting_jobs: accepting.value == [1],
            state: printer_state,
            reasons: values(&message, 4, "printer-state-reasons", 0x44)?,
            pwg_resolutions,
            pwg_types: values(&message, 4, "pwg-raster-document-type-supported", 0x44)?,
            media: values(&message, 4, "media-supported", 0x44)?,
        })
    }
    async fn submit(
        &self,
        name: &str,
        mime: &str,
        document: Vec<u8>,
    ) -> Result<AcceptedJob, Error> {
        if name.is_empty()
            || name.len() > 255
            || name.chars().any(char::is_control)
            || mime.len() > 255
            || document.is_empty()
            || document.len() > MAX_DOCUMENT
        {
            return Err(Error::InvalidInput);
        }
        if mime == "image/pwg-raster" {
            return Err(Error::InvalidInput);
        }
        let capabilities = self.capabilities().await?;
        if !capabilities.accepting_jobs {
            return Err(Error::NotAccepting);
        }
        if !capabilities.document_formats.iter().any(|f| f == mime) {
            return Err(Error::UnsupportedFormat);
        }
        self.send_document(name, mime, document, None).await
    }
    async fn job_status(&self, job_id: i32) -> Result<JobStatus, Error> {
        if job_id <= 0 {
            return Err(Error::InvalidInput);
        }
        let (id, mut message) = self.message(ipp::GET_JOB_ATTRIBUTES);
        message.integer("job-id", job_id);
        let bytes = self
            .exchange(ipp::GET_JOB_ATTRIBUTES, id, message.finish())
            .await?;
        let message = Message::parse(&bytes).map_err(|_| Error::InvalidResponse)?;
        if message.integer(2, "job-id") != Some(job_id) {
            return Err(Error::InvalidResponse);
        }
        let names = values(&message, 2, "job-name", 0x42)?;
        if names.len() > 1 {
            return Err(Error::InvalidResponse);
        }
        Ok(JobStatus {
            job_name: names.into_iter().next(),
            job_id,
            state: DeviceState::from_code(state(&message, 2, "job-state")?)?,
            reasons: values(&message, 2, "job-state-reasons", 0x44)?,
        })
    }
    async fn cancel(&self, job_id: i32) -> Result<(), Error> {
        if job_id <= 0 {
            return Err(Error::InvalidInput);
        }
        let (id, mut message) = self.message(ipp::CANCEL_JOB);
        message.integer("job-id", job_id);
        self.exchange(ipp::CANCEL_JOB, id, message.finish()).await?;
        Ok(())
    }
}
