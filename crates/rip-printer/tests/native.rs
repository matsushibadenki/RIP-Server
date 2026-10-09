use axum::{Router, body::Bytes, extract::State, routing::post};
use rip_ipp::{self as ipp, Request, Response};
use rip_printer::{DeviceState, Error, NativeIpp, PrinterBackend};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Clone)]
struct Fake {
    state: Arc<Mutex<i32>>,
    prints: Arc<AtomicUsize>,
    mode: u8,
}
async fn handle(
    State(fake): State<Fake>,
    body: Bytes,
) -> ([(axum::http::header::HeaderName, &'static str); 1], Vec<u8>) {
    let request = Request::parse(&body).unwrap();
    assert_eq!(request.text(1, "attributes-charset", 0x47), Some("utf-8"));
    assert_eq!(
        request.text(1, "attributes-natural-language", 0x48),
        Some("en")
    );
    assert!(
        request
            .text(1, "printer-uri", 0x45)
            .unwrap()
            .ends_with("/printer")
    );
    let id = if fake.mode == 2 {
        request.request_id + 1
    } else {
        request.request_id
    };
    let status = if request.operation == ipp::PRINT_JOB {
        match fake.mode {
            4 => ipp::DOCUMENT_FORMAT_NOT_SUPPORTED,
            5 => 1,
            _ => ipp::OK,
        }
    } else {
        ipp::OK
    };
    let mut response = Response::new([1, 1], status, id);
    response.text(0x47, "attributes-charset", "utf-8");
    response.text(0x48, "attributes-natural-language", "en");
    match request.operation {
        ipp::GET_PRINTER_ATTRIBUTES => {
            response.group(4);
            response.text(0x49, "document-format-supported", "application/pdf");
            response.attribute(
                0x22,
                "printer-is-accepting-jobs",
                &[u8::from(fake.mode != 1)],
            );
            response.attribute(0x23, "printer-state", &3i32.to_be_bytes());
            response.text(0x44, "printer-state-reasons", "none");
            if fake.mode == 7 {
                response.text(0x49, "document-format-supported", "image/pwg-raster");
                response.resolution_dpi("pwg-raster-document-resolution-supported", 300);
                response.text(0x44, "pwg-raster-document-type-supported", "rgb_8");
                response.text(0x44, "media-supported", "iso_a4_210x297mm");
            }
        }
        ipp::PRINT_JOB => {
            assert_eq!(
                request.text(1, "document-format", 0x49),
                Some("application/pdf")
            );
            assert_eq!(request.one(1, "ipp-attribute-fidelity").unwrap().value, [1]);
            assert_eq!(request.document, b"%PDF-1.4\nmock\n");
            fake.prints.fetch_add(1, Ordering::SeqCst);
            response.group(2);
            response.integer("job-id", 42);
            response.attribute(0x23, "job-state", &3i32.to_be_bytes());
            if fake.mode == 6 {
                return (
                    [(axum::http::header::CONTENT_TYPE, "application/ipp")],
                    vec![0; 65537],
                );
            }
            if fake.mode == 3 {
                return (
                    [(axum::http::header::CONTENT_TYPE, "application/ipp")],
                    vec![0],
                );
            }
        }
        ipp::GET_JOB_ATTRIBUTES => {
            assert_eq!(request.integer(1, "job-id"), Some(42));
            response.group(2);
            response.integer("job-id", 42);
            response.attribute(0x23, "job-state", &fake.state.lock().unwrap().to_be_bytes());
            response.text(0x44, "job-state-reasons", "none");
        }
        ipp::CANCEL_JOB => {
            assert_eq!(request.integer(1, "job-id"), Some(42));
            *fake.state.lock().unwrap() = 7;
        }
        _ => panic!("Unexpected operation"),
    }
    (
        [(axum::http::header::CONTENT_TYPE, "application/ipp")],
        response.finish(),
    )
}
async fn printer(mode: u8) -> (NativeIpp, Fake, tokio::task::JoinHandle<()>) {
    let fake = Fake {
        state: Arc::new(Mutex::new(3)),
        prints: Arc::new(AtomicUsize::new(0)),
        mode,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/printer", post(handle))
        .with_state(fake.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (
        NativeIpp::new(&format!("ipp://{address}/printer")).unwrap(),
        fake,
        task,
    )
}
#[tokio::test]
async fn native_backend_distinguishes_acceptance_processing_completion_and_cancel() {
    let (printer, fake, task) = printer(0).await;
    let capabilities = printer.capabilities().await.unwrap();
    assert_eq!(capabilities.document_formats, ["application/pdf"]);
    assert!(capabilities.accepting_jobs);
    let accepted = printer
        .submit("proof.pdf", "application/pdf", b"%PDF-1.4\nmock\n".to_vec())
        .await
        .unwrap();
    assert_eq!(accepted.job_id, 42);
    assert_eq!(accepted.state, DeviceState::Pending);
    *fake.state.lock().unwrap() = 5;
    assert_eq!(
        printer.job_status(42).await.unwrap().state,
        DeviceState::Processing
    );
    *fake.state.lock().unwrap() = 9;
    assert_eq!(
        printer.job_status(42).await.unwrap().state,
        DeviceState::Completed
    );
    printer.cancel(42).await.unwrap();
    assert_eq!(
        printer.job_status(42).await.unwrap().state,
        DeviceState::Cancelled
    );
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    task.abort();
}
#[tokio::test]
async fn format_availability_and_invalid_input_fail_before_sending_a_job() {
    let (printer, fake, task) = printer(0).await;
    assert!(matches!(
        printer.submit("proof.tiff", "image/tiff", vec![1]).await,
        Err(Error::UnsupportedFormat)
    ));
    assert!(matches!(
        printer.submit("", "application/pdf", vec![1]).await,
        Err(Error::InvalidInput)
    ));
    assert!(matches!(
        printer.job_status(0).await,
        Err(Error::InvalidInput)
    ));
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
    let (printer, fake, task) = self::printer(1).await;
    assert!(matches!(
        printer
            .submit("proof.pdf", "application/pdf", vec![1])
            .await,
        Err(Error::NotAccepting)
    ));
    assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
    task.abort();
}
#[tokio::test]
async fn rejects_mismatched_reply_and_never_resends_uncertain_submission() {
    let (printer, _, task) = printer(2).await;
    assert!(matches!(
        printer.capabilities().await,
        Err(Error::InvalidResponse)
    ));
    task.abort();
    let (printer, fake, task) = self::printer(3).await;
    assert!(matches!(
        printer
            .submit("proof.pdf", "application/pdf", b"%PDF-1.4\nmock\n".to_vec())
            .await,
        Err(Error::OutcomeUnknown)
    ));
    assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
    task.abort();
}
#[test]
fn endpoints_are_validated_without_following_printer_returned_urls() {
    assert!(NativeIpp::new("ipps://example.com/printer").is_ok());
    for uri in [
        "https://example.com/printer",
        "ipp://user:password@example.com/printer",
        "ipp://example.com/printer?x=1",
        "ipp://example.com/printer#x",
    ] {
        assert!(NativeIpp::new(uri).is_err());
    }
}

#[tokio::test]
async fn ipp_rejection_substitution_and_oversized_replies_are_not_retried() {
    for mode in [4, 5, 6] {
        let (printer, fake, task) = printer(mode).await;
        let result = printer
            .submit("proof.pdf", "application/pdf", b"%PDF-1.4\nmock\n".to_vec())
            .await;
        if mode == 4 {
            assert!(matches!(
                result,
                Err(Error::Status(ipp::DOCUMENT_FORMAT_NOT_SUPPORTED))
            ));
        } else {
            assert!(matches!(result, Err(Error::OutcomeUnknown)));
        }
        assert_eq!(fake.prints.load(Ordering::SeqCst), 1);
        task.abort();
    }
}

#[tokio::test]
async fn pwg_capabilities_require_exact_dpi_type_and_media() {
    let (printer, _, task) = printer(7).await;
    let capabilities = printer.capabilities().await.unwrap();
    assert!(
        capabilities
            .check_pwg(300, "rgb_8", "iso_a4_210x297mm")
            .is_ok()
    );
    for (dpi, color, media) in [
        (600, "rgb_8", "iso_a4_210x297mm"),
        (300, "cmyk_8", "iso_a4_210x297mm"),
        (300, "rgb_8", "na_letter_8.5x11in"),
    ] {
        assert!(capabilities.check_pwg(dpi, color, media).is_err());
    }
    task.abort();
}
