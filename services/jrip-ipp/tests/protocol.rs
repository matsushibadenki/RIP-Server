use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use rip_ipp::{self as ipp, Request as IppRequest};
use tower::ServiceExt;
const URI: &str = "ipp://127.0.0.1:8631/ipp/print";
#[test]
fn discovery_matches_reachable_endpoint_and_pdf_capability() {
    let listen = "0.0.0.0:8631".parse().unwrap();
    let info = jrip_ipp::discovery_info(
        listen,
        "ipp://192.0.2.15:8631/ipp/print",
        "jrip-proof.local.",
    )
    .unwrap();
    assert_eq!(info.get_type(), "_ipp._tcp.local.");
    assert_eq!(info.get_port(), 8631);
    assert_eq!(info.get_property_val_str("rp"), Some("ipp/print"));
    assert_eq!(info.get_property_val_str("pdl"), Some("application/pdf"));
    for uri in [
        URI,
        "ipps://192.0.2.15:8631/ipp/print",
        "ipp://192.0.2.15:8632/ipp/print",
        "ipp://192.0.2.15:8631/ipp/other",
        "ipp://192.0.2.15:8631/ipp/print?x=1",
    ] {
        assert!(jrip_ipp::discovery_info(listen, uri, "jrip-proof.local.").is_err());
    }
    assert!(
        jrip_ipp::discovery_info(
            "192.0.2.16:8631".parse().unwrap(),
            "ipp://192.0.2.15:8631/ipp/print",
            "jrip-proof.local.",
        )
        .is_err()
    );
}
fn attribute(bytes: &mut Vec<u8>, tag: u8, name: &str, value: &[u8]) {
    bytes.push(tag);
    bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
    bytes.extend_from_slice(name.as_bytes());
    bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
    bytes.extend_from_slice(value);
}
fn message(
    operation: u16,
    document: &[u8],
    format: Option<&str>,
    id: Option<i32>,
    unsupported: bool,
) -> Vec<u8> {
    let mut msg = vec![1, 1];
    msg.extend_from_slice(&operation.to_be_bytes());
    msg.extend_from_slice(&17u32.to_be_bytes());
    msg.push(1);
    attribute(&mut msg, 0x47, "attributes-charset", b"utf-8");
    attribute(&mut msg, 0x48, "attributes-natural-language", b"en");
    attribute(&mut msg, 0x45, "printer-uri", URI.as_bytes());
    if let Some(format) = format {
        attribute(&mut msg, 0x49, "document-format", format.as_bytes());
    }
    if let Some(id) = id {
        attribute(&mut msg, 0x21, "job-id", &id.to_be_bytes());
    }
    if unsupported {
        msg.push(2);
        attribute(&mut msg, 0x21, "copies", &2i32.to_be_bytes());
    }
    msg.push(3);
    msg.extend_from_slice(document);
    msg
}
async fn send(router: &axum::Router, bytes: Vec<u8>) -> (StatusCode, Vec<u8>) {
    send_path(router, "/ipp/print", bytes).await
}
async fn send_path(router: &axum::Router, path: &str, bytes: Vec<u8>) -> (StatusCode, Vec<u8>) {
    let request = Request::post(path)
        .header("content-type", "application/ipp")
        .body(Body::from(bytes))
        .unwrap();
    let result = router.clone().oneshot(request).await.unwrap();
    let status = result.status();
    let bytes = result
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, bytes)
}
fn status(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[2], bytes[3]])
}
#[tokio::test]
async fn pdf_job_round_trip_and_persistent_id() {
    let dir = tempfile::tempdir().unwrap();
    let state = jrip_ipp::open(dir.path().into(), URI.into()).await.unwrap();
    let router = jrip_ipp::router(state.clone());
    let (http, attrs) = send(
        &router,
        message(ipp::GET_PRINTER_ATTRIBUTES, &[], None, None, false),
    )
    .await;
    assert_eq!(http, StatusCode::OK);
    assert_eq!(status(&attrs), ipp::OK);
    let parsed = IppRequest::parse(&attrs).unwrap();
    assert_eq!(
        parsed.text(4, "document-format-supported", 0x49),
        Some("application/pdf")
    );
    assert_eq!(
        parsed.resolution_dpi(4, "printer-resolution-default"),
        Some(600)
    );
    assert_eq!(
        parsed.text(4, "print-color-mode-default", 0x44),
        Some("color")
    );
    let mut selected = message(ipp::GET_PRINTER_ATTRIBUTES, &[], None, None, false);
    selected.pop();
    attribute(
        &mut selected,
        0x44,
        "requested-attributes",
        b"document-format-supported",
    );
    selected.push(3);
    let (_, selected_response) = send(&router, selected).await;
    let selected = IppRequest::parse(&selected_response).unwrap();
    assert_eq!(
        selected.text(4, "document-format-supported", 0x49),
        Some("application/pdf")
    );
    assert!(selected.one(4, "printer-name").is_none());
    let mut version_two = message(ipp::GET_PRINTER_ATTRIBUTES, &[], None, None, false);
    version_two[..2].copy_from_slice(&[2, 0]);
    let (_, version_two_response) = send(&router, version_two).await;
    assert_eq!(&version_two_response[..2], &[2, 0]);
    assert_eq!(status(&version_two_response), ipp::OK);
    let pdf = include_bytes!("../../../tests/corpus/colors.pdf");
    let (http, response) = send(
        &router,
        message(ipp::PRINT_JOB, pdf, Some("application/pdf"), None, false),
    )
    .await;
    assert_eq!(http, StatusCode::OK);
    assert_eq!(status(&response), ipp::OK);
    assert_eq!(i32::from_be_bytes(response[4..8].try_into().unwrap()), 17);
    let parsed = IppRequest::parse(&response).unwrap();
    let id = parsed.integer(2, "job-id").unwrap();
    assert!(id > 0);
    let job_uri = parsed.text(2, "job-uri", 0x45).unwrap();
    let mut by_uri = vec![1, 1];
    by_uri.extend_from_slice(&ipp::GET_JOB_ATTRIBUTES.to_be_bytes());
    by_uri.extend_from_slice(&18u32.to_be_bytes());
    by_uri.push(1);
    attribute(&mut by_uri, 0x47, "attributes-charset", b"utf-8");
    attribute(&mut by_uri, 0x48, "attributes-natural-language", b"en");
    attribute(&mut by_uri, 0x45, "job-uri", job_uri.as_bytes());
    by_uri.push(3);
    let (_, by_uri_response) = send_path(&router, &format!("/ipp/print/jobs/{id}"), by_uri).await;
    assert_eq!(
        IppRequest::parse(&by_uri_response)
            .unwrap()
            .integer(2, "job-id"),
        Some(id)
    );
    let job = state.repo.get_ipp_job(id).await.unwrap();
    assert_eq!(job.manifest.format, rip_core::DocumentFormat::Pdf);
    assert!(
        dir.path()
            .join("jobs")
            .join(job.id.to_string())
            .join("document")
            .exists()
    );
    let reopened = jrip_ipp::open(dir.path().into(), URI.into()).await.unwrap();
    assert_eq!(reopened.repo.get_ipp_job(id).await.unwrap().id, job.id);
    let (_, query) = send(
        &router,
        message(ipp::GET_JOB_ATTRIBUTES, &[], None, Some(id), false),
    )
    .await;
    assert_eq!(
        IppRequest::parse(&query).unwrap().integer(2, "job-state"),
        Some(3)
    );
    let (_, cancel) = send(
        &router,
        message(ipp::CANCEL_JOB, &[], None, Some(id), false),
    )
    .await;
    assert_eq!(status(&cancel), ipp::OK);
    assert_eq!(
        reopened.repo.get_ipp_job(id).await.unwrap().state,
        rip_core::JobState::Cancelled
    );
    let (_, again) = send(
        &router,
        message(ipp::CANCEL_JOB, &[], None, Some(id), false),
    )
    .await;
    assert_eq!(status(&again), 0x0405);
}
#[tokio::test]
async fn supported_raster_settings_reach_manifest_and_other_values_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let state = jrip_ipp::open(dir.path().into(), URI.into()).await.unwrap();
    let app = jrip_ipp::router(state.clone());
    let pdf = include_bytes!("../../../tests/corpus/colors.pdf");
    for (dpi, mode, expected) in [
        (300u32, "monochrome", ipp::OK),
        (1200u32, "color", ipp::ATTRIBUTES_NOT_SUPPORTED),
        (600u32, "auto", ipp::ATTRIBUTES_NOT_SUPPORTED),
    ] {
        let mut request = message(ipp::PRINT_JOB, pdf, Some("application/pdf"), None, false);
        let insert = request.len() - pdf.len() - 1;
        let mut settings = vec![2];
        let mut resolution = [0u8; 9];
        resolution[0..4].copy_from_slice(&dpi.to_be_bytes());
        resolution[4..8].copy_from_slice(&dpi.to_be_bytes());
        resolution[8] = 3;
        attribute(&mut settings, 0x32, "printer-resolution", &resolution);
        attribute(&mut settings, 0x44, "print-color-mode", mode.as_bytes());
        request.splice(insert..insert, settings);
        let (_, response) = send(&app, request).await;
        assert_eq!(status(&response), expected);
        if expected == ipp::OK {
            let parsed = IppRequest::parse(&response).unwrap();
            let id = parsed.integer(2, "job-id").unwrap();
            assert_eq!(parsed.resolution_dpi(2, "printer-resolution"), Some(300));
            assert_eq!(parsed.text(2, "print-color-mode", 0x44), Some("monochrome"));
            let job = state.repo.get_ipp_job(id).await.unwrap();
            assert_eq!(job.manifest.dpi, 300);
            assert_eq!(job.manifest.color_mode, rip_core::ColorMode::Gray);
        }
    }
}
#[tokio::test]
async fn get_jobs_filters_completed_and_limits_results() {
    let dir = tempfile::tempdir().unwrap();
    let state = jrip_ipp::open(dir.path().into(), URI.into()).await.unwrap();
    let app = jrip_ipp::router(state);
    let pdf = include_bytes!("../../../tests/corpus/colors.pdf");
    let mut ids = Vec::new();
    for _ in 0..3 {
        let (_, response) = send(&app, message(ipp::PRINT_JOB, pdf, None, None, false)).await;
        ids.push(
            IppRequest::parse(&response)
                .unwrap()
                .integer(2, "job-id")
                .unwrap(),
        );
    }
    let (_, response) = send(
        &app,
        message(ipp::CANCEL_JOB, &[], None, Some(ids[1]), false),
    )
    .await;
    assert_eq!(status(&response), ipp::OK);
    let (_, response) = send(&app, message(ipp::GET_JOBS, &[], None, None, false)).await;
    let listed = IppRequest::parse(&response).unwrap();
    let pending: Vec<_> = listed
        .attributes
        .iter()
        .filter(|a| a.name == "job-id")
        .collect();
    assert_eq!(pending.len(), 2);
    assert!(listed.attributes.iter().all(|a| matches!(
        a.name.as_str(),
        "attributes-charset" | "attributes-natural-language" | "job-id" | "job-uri"
    )));

    let mut request = message(ipp::GET_JOBS, &[], None, None, false);
    request.pop();
    attribute(&mut request, 0x44, "which-jobs", b"completed");
    attribute(&mut request, 0x21, "limit", &1i32.to_be_bytes());
    attribute(&mut request, 0x44, "requested-attributes", b"job-state");
    request.push(3);
    let (_, response) = send(&app, request).await;
    let listed = IppRequest::parse(&response).unwrap();
    assert_eq!(listed.integer(2, "job-state"), Some(7));
    assert!(listed.one(2, "job-id").is_none());
}
#[tokio::test]
async fn validate_job_uses_print_job_rules_without_persisting() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_ipp::router(jrip_ipp::open(dir.path().into(), URI.into()).await.unwrap());
    for (request, expected) in [
        (
            message(ipp::VALIDATE_JOB, &[], Some("application/pdf"), None, false),
            ipp::OK,
        ),
        (
            message(ipp::VALIDATE_JOB, &[], Some("image/urf"), None, false),
            ipp::DOCUMENT_FORMAT_NOT_SUPPORTED,
        ),
        (
            message(ipp::VALIDATE_JOB, &[], None, None, true),
            ipp::ATTRIBUTES_NOT_SUPPORTED,
        ),
        (
            message(ipp::VALIDATE_JOB, b"%PDF-1.4", None, None, false),
            ipp::BAD_REQUEST,
        ),
    ] {
        let (_, response) = send(&app, request).await;
        assert_eq!(status(&response), expected);
    }
    assert_eq!(
        std::fs::read_dir(dir.path().join("jobs")).unwrap().count(),
        0
    );
}
#[tokio::test]
async fn unsupported_settings_and_malformed_documents_do_not_create_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_ipp::router(jrip_ipp::open(dir.path().into(), URI.into()).await.unwrap());
    for (message, expected) in [
        (
            message(ipp::PRINT_JOB, b"%PDF-1.4", Some("image/urf"), None, false),
            ipp::DOCUMENT_FORMAT_NOT_SUPPORTED,
        ),
        (
            message(
                ipp::PRINT_JOB,
                b"%PDF-1.4",
                Some("application/pdf"),
                None,
                true,
            ),
            ipp::ATTRIBUTES_NOT_SUPPORTED,
        ),
        (
            message(
                ipp::PRINT_JOB,
                b"not a pdf",
                Some("application/pdf"),
                None,
                false,
            ),
            ipp::FORMAT_ERROR,
        ),
        (
            message(ipp::GET_JOB_ATTRIBUTES, &[], None, Some(99), false),
            ipp::NOT_FOUND,
        ),
    ] {
        let (http, response) = send(&app, message).await;
        assert_eq!(http, StatusCode::OK);
        assert_eq!(status(&response), expected);
    }
    assert_eq!(
        std::fs::read_dir(dir.path().join("jobs")).unwrap().count(),
        0
    );
    let (http, _) = send(&app, vec![1, 1, 0]).await;
    assert_eq!(http, StatusCode::BAD_REQUEST);

    let pdf = include_bytes!("../../../tests/corpus/colors.pdf");
    let base = message(ipp::PRINT_JOB, pdf, Some("application/pdf"), None, false);
    let insert = base.len() - pdf.len() - 1;
    for (tag, name, value) in [
        (0x44, "job-name", b"ignored".as_slice()),
        (0x42, "job-name", b"first".as_slice()),
        (0x21, "copies", &2i32.to_be_bytes()[..]),
    ] {
        let mut malformed = base.clone();
        let mut extra = Vec::new();
        attribute(&mut extra, tag, name, value);
        if name == "job-name" && tag == 0x42 {
            attribute(&mut extra, 0x42, "job-name", b"second");
        }
        malformed.splice(insert..insert, extra);
        let (_, response) = send(&app, malformed).await;
        assert_ne!(status(&response), ipp::OK);
    }
    assert_eq!(
        std::fs::read_dir(dir.path().join("jobs")).unwrap().count(),
        0
    );
}
