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
            assert!(request.attributes.iter().any(|a| a.group == 1
                && a.name == "requested-attributes"
                && a.value == b"queued-job-count"));
            match fake.mode {
                23 => response.integer("queued-job-count", 0),
                24 => response.integer("queued-job-count", 7),
                25 => response.integer("queued-job-count", -1),
                26 => response.text(0x44, "queued-job-count", "7"),
                27 => {
                    response.integer("queued-job-count", 1);
                    response.additional(0x21, &2i32.to_be_bytes());
                }
                28 => response.attribute(0x12, "queued-job-count", &[]),
                29 => response.attribute(0x21, "queued-job-count", &[0]),
                30 => response.integer("queued-job-count", i32::MAX),
                _ => {}
            }
            for name in [
                "marker-names",
                "marker-types",
                "marker-colors",
                "marker-levels",
                "marker-low-levels",
                "marker-high-levels",
            ] {
                assert!(request.attributes.iter().any(|a| a.group == 1
                    && a.name == "requested-attributes"
                    && a.value == name.as_bytes()));
            }
            match fake.mode {
                40 => {
                    response.integer("marker-levels", 0);
                    response.additional(0x21, &(-2i32).to_be_bytes());
                    response.text(0x42, "marker-names", "Black toner");
                    response.additional(0x42, "シアン".as_bytes());
                    response.text(0x44, "marker-types", "toner");
                    response.additional(0x44, b"ink");
                    response.text(0x42, "marker-colors", "#000000");
                    response.additional(0x42, b"#00FFFF");
                }
                41 => {
                    response.integer("marker-levels", -1);
                    response.additional(0x21, &(-2i32).to_be_bytes());
                    response.additional(0x21, &(-3i32).to_be_bytes());
                }
                42 => {
                    response.integer("marker-levels", 1);
                    response.additional(0x21, &2i32.to_be_bytes());
                    response.text(0x42, "marker-names", "Only one");
                }
                43 => response.text(0x44, "marker-levels", "25"),
                44 => response.attribute(0x12, "marker-levels", &[]),
                45 => {
                    response.integer("marker-levels", 90);
                    response.text(0x44, "marker-types", "waste-toner");
                    response.integer("marker-high-levels", 0);
                    response.integer("marker-low-levels", 95);
                }
                46 => {
                    response.integer("marker-levels", 10);
                    for _ in 0..64 {
                        response.additional(0x21, &10i32.to_be_bytes());
                    }
                }
                47 => response.text(0x42, "marker-names", "bad\nname"),
                48 => response.integer("marker-levels", 101),
                _ => {}
            }
            for name in ["printer-supply", "printer-supply-description"] {
                assert!(request.attributes.iter().any(|a| a.group == 1
                    && a.name == "requested-attributes"
                    && a.value == name.as_bytes()));
            }
            match fake.mode {
                50 => {
                    response.attribute(0x30,"printer-supply",b"type=tonerCartridge;maxcapacity=100;level=0;colorantname=black;unit=percent;class=supplyThatIsConsumed;");
                    let text = "黒トナー".as_bytes();
                    let mut d = 2u16.to_be_bytes().to_vec();
                    d.extend_from_slice(b"ja");
                    d.extend_from_slice(&(text.len() as u16).to_be_bytes());
                    d.extend_from_slice(text);
                    response.attribute(0x35, "printer-supply-description", &d);
                }
                51 => response.attribute(
                    0x30,
                    "printer-supply",
                    b"type=staples;maxcapacity=5000;level=2000;colorantname=no-color;unit=items;",
                ),
                52 => {
                    response.attribute(
                        0x30,
                        "printer-supply",
                        b"type=ink;maxcapacity=-2;level=-1;colorantname=cyan;",
                    );
                    response.additional(
                        0x30,
                        b"type=ink;maxcapacity=-2;level=-2;colorantname=magenta;",
                    );
                    response.additional(
                        0x30,
                        b"type=ink;maxcapacity=-2;level=-3;colorantname=yellow;",
                    );
                }
                53 => response.attribute(
                    0x30,
                    "printer-supply",
                    b"type=toner;maxcapacity=100;level=5;level=6;colorantname=black;",
                ),
                54 => {
                    response.attribute(
                        0x30,
                        "printer-supply",
                        b"type=toner;maxcapacity=100;level=5;colorantname=black;",
                    );
                    response.text(0x41, "printer-supply-description", "One");
                    response.additional(0x41, b"Extra");
                }
                55 => response.attribute(0x12, "printer-supply", &[]),
                56 => response.attribute(0x10, "printer-supply", &[]),
                57 => response.text(
                    0x41,
                    "printer-supply",
                    "type=toner;maxcapacity=100;level=5;colorantname=black;",
                ),
                58 => {
                    response.attribute(0x30,"printer-supply",b"type=wasteToner;maxcapacity=100;level=90;colorantname=no-color;unit=percent;class=receptacleThatIsFilled;");
                    response.text(0x41, "printer-supply-description", "Waste receptacle");
                }
                59 => response.attribute(
                    0x30,
                    "printer-supply",
                    b"type=toner;maxcapacity=100;level=5;\ncolorantname=black;",
                ),
                60 => {
                    response.attribute(
                        0x30,
                        "printer-supply",
                        b"type=toner;maxcapacity=100;level=25;colorantname=black;unit=percent;",
                    );
                    response.integer("marker-levels", 50);
                }
                _ => {}
            }
            match fake.mode {
                8 => response.attribute(0x33, "copies-supported", &[0, 0, 0, 1, 0, 0, 0, 5]),
                9 => response.integer("copies-supported", 5),
                10 => response.attribute(0x33, "copies-supported", &[0, 0, 0, 5, 0, 0, 0, 1]),
                11 => {
                    response.attribute(0x33, "copies-supported", &[0, 0, 0, 1, 0, 0, 0, 5]);
                    response.additional(0x33, &[0, 0, 0, 1, 0, 0, 0, 5]);
                }
                _ => {}
            }

            match fake.mode {
                12..=15 => {
                    response.text(0x44, "sides-supported", "one-sided");
                    response.additional(0x44, b"two-sided-long-edge");
                    if fake.mode == 14 {
                        response.integer("pwg-raster-document-sheet-back", 1);
                    } else {
                        response.text(
                            0x44,
                            "pwg-raster-document-sheet-back",
                            if fake.mode == 13 { "rotated" } else { "normal" },
                        );
                    }
                    if fake.mode == 15 {
                        response.additional(0x44, b"normal");
                    }
                }
                16 => response.integer("sides-supported", 1),
                _ => {}
            }
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
            let requested: Vec<_> = request
                .attributes
                .iter()
                .filter(|a| a.group == 1 && a.name == "requested-attributes")
                .map(|a| {
                    assert_eq!(a.tag, 0x44);
                    std::str::from_utf8(&a.value).unwrap()
                })
                .collect();
            assert_eq!(
                requested,
                [
                    "job-id",
                    "job-name",
                    "job-state",
                    "job-state-reasons",
                    "job-impressions-completed",
                    "job-media-sheets-completed"
                ]
            );
            response.group(2);
            response.integer("job-id", 42);
            response.attribute(0x23, "job-state", &fake.state.lock().unwrap().to_be_bytes());
            for name in ["job-impressions-completed", "job-media-sheets-completed"] {
                match fake.mode {
                    32 => response.integer(name, 0),
                    33 => response.integer(
                        name,
                        if name == "job-impressions-completed" {
                            8
                        } else {
                            4
                        },
                    ),
                    34 => response.integer(name, -1),
                    35 => response.text(0x44, name, "8"),
                    36 => {
                        response.integer(name, 1);
                        response.additional(0x21, &2i32.to_be_bytes());
                    }
                    37 => response.attribute(0x12, name, &[]),
                    38 => response.attribute(0x21, name, &[0]),
                    39 if name == "job-impressions-completed" => response.integer(name, i32::MAX),
                    _ => {}
                }
            }
            match fake.mode {
                17 => response.text(0x44, "job-state-reasons", ""),
                18 => response.text(0x44, "job-state-reasons", &"x".repeat(256)),
                19 => response.text(0x44, "job-state-reasons", "vendor\nreason"),
                20 => {
                    response.text(0x44, "job-state-reasons", "media-needed");
                    for _ in 0..64 {
                        response.additional(0x44, b"vendor-reason");
                    }
                }
                21 => response.integer("job-state-reasons", 1),
                22 => {
                    response.text(0x44, "job-state-reasons", "media-needed");
                    response.additional(0x44, b"vendor-needs-attention");
                }
                _ => response.text(0x44, "job-state-reasons", "none"),
            }
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

#[tokio::test]
async fn copies_capability_is_typed_bounded_and_conservative_when_missing() {
    let (device, _, task) = printer(8).await;
    let capabilities = device.capabilities().await.unwrap();
    assert_eq!(capabilities.copies_supported, Some([1, 5]));
    for count in [1, 5] {
        capabilities.check_copies(count).unwrap();
    }
    assert!(matches!(
        capabilities.check_copies(6),
        Err(Error::UnsupportedCopies)
    ));
    assert!(matches!(
        capabilities.check_copies(0),
        Err(Error::InvalidInput)
    ));
    assert!(matches!(
        capabilities.check_copies(1000),
        Err(Error::InvalidInput)
    ));
    task.abort();
    let (device, _, task) = printer(0).await;
    let capabilities = device.capabilities().await.unwrap();
    assert_eq!(capabilities.copies_supported, None);
    capabilities.check_copies(1).unwrap();
    assert!(matches!(
        capabilities.check_copies(2),
        Err(Error::UnsupportedCopies)
    ));
    task.abort();
    for mode in [9, 10, 11] {
        let (device, fake, task) = printer(mode).await;
        assert!(matches!(
            device.capabilities().await,
            Err(Error::InvalidResponse)
        ));
        assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
        task.abort();
    }
}

#[tokio::test]
async fn sides_and_sheet_back_capabilities_are_conservative_and_typed() {
    use rip_core::print::Sides;
    for mode in [0, 12, 13, 14, 15, 16] {
        let (printer, _, task) = printer(mode).await;
        let result = printer.capabilities().await;
        if mode >= 14 {
            assert!(matches!(result, Err(Error::InvalidResponse)));
        } else {
            let caps = result.unwrap();
            assert!(caps.check_sides(Sides::OneSided).is_ok());
            assert_eq!(
                caps.check_sides(Sides::TwoSidedLongEdge).is_ok(),
                matches!(mode, 12 | 13)
            );
            assert!(caps.check_sides(Sides::TwoSidedShortEdge).is_err());
        }
        task.abort();
    }
}

#[tokio::test]
async fn job_polling_requests_only_needed_attributes_and_bounds_device_reasons() {
    for mode in 17..=22 {
        let (printer, fake, task) = printer(mode).await;
        let status = printer.job_status(42).await;
        if mode == 22 {
            assert_eq!(
                status.unwrap().reasons,
                ["media-needed", "vendor-needs-attention"]
            );
        } else {
            assert!(matches!(status, Err(Error::InvalidResponse)), "mode {mode}");
        }
        assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
        task.abort();
    }
}

#[tokio::test]
async fn live_queue_count_is_optional_typed_and_never_inferred_as_zero() {
    for (mode, expected) in [
        (0, None),
        (23, Some(0)),
        (24, Some(7)),
        (28, None),
        (30, Some(i32::MAX as u32)),
    ] {
        let (printer, fake, task) = printer(mode).await;
        assert_eq!(
            printer.capabilities().await.unwrap().queued_job_count,
            expected
        );
        assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
        task.abort();
    }
    for mode in [25, 26, 27, 29] {
        let (printer, fake, task) = printer(mode).await;
        assert!(matches!(
            printer.capabilities().await,
            Err(Error::InvalidResponse)
        ));
        assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
        task.abort();
    }
}

#[tokio::test]
async fn physical_progress_preserves_units_zero_unknown_and_rejects_invalid_counters() {
    for (mode, impressions, sheets) in [
        (0, None, None),
        (32, Some(0), Some(0)),
        (33, Some(8), Some(4)),
        (37, None, None),
        (39, Some(i32::MAX as u32), None),
    ] {
        let (device, _, task) = printer(mode).await;
        let progress = device.job_status(42).await.unwrap().progress;
        assert_eq!(progress.impressions_completed, impressions);
        assert_eq!(progress.media_sheets_completed, sheets);
        task.abort();
    }
    for mode in [34, 35, 36, 38] {
        let (device, fake, task) = printer(mode).await;
        assert!(matches!(
            device.job_status(42).await,
            Err(Error::InvalidResponse)
        ));
        assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
        task.abort();
    }
}

#[tokio::test]
async fn marker_supplies_preserve_reported_levels_and_isolate_invalid_optional_telemetry() {
    for mode in [0, 40, 41, 42, 43, 44, 45, 46, 47, 48] {
        let (device, fake, task) = printer(mode).await;
        let capabilities = device.capabilities().await.unwrap();
        assert!(capabilities.accepting_jobs);
        assert_eq!(capabilities.document_formats, ["application/pdf"]);
        let r = capabilities.marker_supplies;
        match mode {
            0 => assert_eq!(r.status, "UNREPORTED"),
            40 => {
                assert_eq!(r.markers.len(), 2);
                assert_eq!(r.markers[0].level_percent, Some(0));
                assert_eq!(r.markers[1].name.as_deref(), Some("シアン"));
                assert_eq!(r.markers[1].raw_level, Some(-2));
                assert_eq!(r.markers[1].level_percent, None);
                assert_eq!(r.markers[1].level_state, "UNKNOWN");
                assert_eq!(r.numeric_levels_reported, 1);
            }
            41 => {
                assert_eq!(
                    r.markers.iter().map(|m| m.level_state).collect::<Vec<_>>(),
                    ["UNAVAILABLE", "UNKNOWN", "NOT_AT_CAPACITY"]
                );
                assert_eq!(r.numeric_levels_reported, 0);
            }
            44 => {
                assert_eq!(r.status, "UNREPORTED");
                assert_eq!(r.levels_attribute_state, "UNKNOWN");
            }
            45 => {
                assert_eq!(r.markers[0].kind.as_deref(), Some("waste-toner"));
                assert_eq!(r.markers[0].level_percent, Some(90));
                assert_eq!(r.markers[0].high_level_percent, Some(0));
            }
            _ => {
                assert_eq!(r.status, "INVALID_RESPONSE");
                assert!(r.markers.is_empty());
                assert!(r.reason_code.is_some());
            }
        }
        assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
        task.abort();
    }
}

#[tokio::test]
async fn pwg_supplies_preserve_units_mib_sentinels_and_localized_descriptions_independently() {
    for mode in [0, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60] {
        let (device, fake, task) = printer(mode).await;
        let capabilities = device.capabilities().await.unwrap();
        assert!(capabilities.accepting_jobs);
        let r = capabilities.printer_supplies;
        match mode {
            0 => assert_eq!(r.status, "UNREPORTED"),
            50 => {
                assert_eq!(r.status, "REPORTED");
                assert_eq!(r.supplies[0].level_percent, Some(0));
                assert_eq!(
                    r.supplies[0]
                        .description
                        .as_ref()
                        .unwrap()
                        .language
                        .as_deref(),
                    Some("ja")
                );
                assert_eq!(r.supplies[0].description.as_ref().unwrap().text, "黒トナー");
            }
            51 => {
                assert_eq!(r.supplies[0].unit.as_deref(), Some("items"));
                assert_eq!(r.supplies[0].level, Some(2000));
                assert_eq!(r.supplies[0].level_percent, None);
            }
            52 => assert_eq!(
                r.supplies.iter().map(|s| s.level_state).collect::<Vec<_>>(),
                ["OTHER", "UNKNOWN", "SOME_AVAILABLE"]
            ),
            55 => {
                assert_eq!(r.status, "UNREPORTED");
                assert_eq!(r.attribute_state, "UNKNOWN");
            }
            56 => {
                assert_eq!(r.status, "UNREPORTED");
                assert_eq!(r.attribute_state, "UNSUPPORTED");
            }
            58 => {
                assert_eq!(r.supplies[0].level_meaning, "REMAINING_SPACE");
                assert_eq!(r.supplies[0].level_percent, Some(90));
                assert_eq!(r.supplies[0].description.as_ref().unwrap().language, None);
            }
            60 => {
                assert_eq!(r.supplies[0].level_percent, Some(25));
                assert_eq!(
                    capabilities.marker_supplies.markers[0].level_percent,
                    Some(50)
                );
            }
            _ => {
                assert_eq!(r.status, "INVALID_RESPONSE");
                assert!(r.supplies.is_empty());
                assert!(r.reason_code.is_some());
            }
        }
        assert_eq!(fake.prints.load(Ordering::SeqCst), 0);
        task.abort();
    }
}
