use rip_core::{ColorMode, DocumentFormat, Manifest};
use rip_interpreter::{DocumentInterpreter, GhostscriptInterpreter, SandboxConfig};
use std::time::Duration;
#[tokio::test]
#[ignore = "requires Docker and jrip-ghostscript:local image"]
async fn rejects_page_overflow_and_times_out() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("document.ps");
    let manifest = Manifest {
        name: "limits.ps".into(),
        format: DocumentFormat::Postscript,
        dpi: 72,
        color_mode: ColorMode::Gray,
        priority: 50,
        engine: Default::default(),
    };
    std::fs::write(&source, b"%!PS\nshowpage showpage\n").unwrap();
    let engine = GhostscriptInterpreter {
        sandbox: SandboxConfig {
            max_pages: 1,
            ..Default::default()
        },
    };
    assert!(
        engine
            .render(&source, &directory.path().join("pages"), &manifest)
            .await
            .is_err()
    );
    std::fs::write(&source, b"%!PS\n{} loop\n").unwrap();
    let engine = GhostscriptInterpreter {
        sandbox: SandboxConfig {
            timeout: Duration::from_secs(2),
            ..Default::default()
        },
    };
    let error = engine
        .render(&source, &directory.path().join("timeout"), &manifest)
        .await
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "resource_limit_timeout");
}

#[tokio::test]
#[ignore = "requires Docker, jrip-mupdf:local and jrip-ghostscript:local images"]
async fn hybrid_pdf_colors_order_and_explicit_ghostscript() {
    use rip_core::EnginePreference;
    use rip_interpreter::HybridInterpreter;
    use tiff::decoder::{Decoder, DecodingResult};
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("document.pdf");
    std::fs::write(&source, include_bytes!("../../../tests/corpus/colors.pdf")).unwrap();
    let engine = HybridInterpreter::default();
    for (index, color) in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Gray]
        .into_iter()
        .enumerate()
    {
        let manifest = Manifest {
            name: "colors.pdf".into(),
            format: DocumentFormat::Pdf,
            dpi: 72,
            color_mode: color,
            priority: 50,
            engine: EnginePreference::Auto,
        };
        let result = engine
            .render(
                &source,
                &directory.path().join(format!("color-{index}")),
                &manifest,
            )
            .await
            .unwrap();
        assert_eq!(result.pages.len(), 2);
        for (page_index, page) in result.pages.iter().enumerate() {
            let mut decoder = Decoder::new(std::fs::File::open(page).unwrap()).unwrap();
            assert_eq!(decoder.dimensions().unwrap(), (20, 20));
            let expected = match color {
                ColorMode::Rgb => tiff::ColorType::RGB(8),
                ColorMode::Cmyk => tiff::ColorType::CMYK(8),
                ColorMode::Gray => tiff::ColorType::Gray(8),
            };
            assert_eq!(decoder.colortype().unwrap(), expected);
            let DecodingResult::U8(pixels) = decoder.read_image().unwrap() else {
                panic!("expected 8-bit samples")
            };
            if color == ColorMode::Rgb {
                let expected = if page_index == 0 {
                    [255, 0, 0]
                } else {
                    [0, 255, 0]
                };
                let (rgb_pixels, remainder) = pixels.as_chunks::<3>();
                assert!(remainder.is_empty(), "incomplete RGB pixel");
                assert!(rgb_pixels.iter().all(|pixel| *pixel == expected));
            }
        }
    }
    let manifest = Manifest {
        name: "colors.pdf".into(),
        format: DocumentFormat::Pdf,
        dpi: 72,
        color_mode: ColorMode::Rgb,
        priority: 50,
        engine: EnginePreference::Ghostscript,
    };
    let rendered = engine
        .render(&source, &directory.path().join("gs-pdf"), &manifest)
        .await
        .unwrap();
    assert_eq!(rendered.pages.len(), 2);
}

#[tokio::test]
#[ignore = "requires Docker and jrip-mupdf:local image"]
async fn mupdf_rejects_overflow_and_broken_pdf_without_fallback() {
    use rip_interpreter::{HybridInterpreter, MuPdfInterpreter};
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("document.pdf");
    let manifest = Manifest {
        name: "test.pdf".into(),
        format: DocumentFormat::Pdf,
        dpi: 72,
        color_mode: ColorMode::Rgb,
        priority: 50,
        engine: Default::default(),
    };
    std::fs::write(&source, include_bytes!("../../../tests/corpus/colors.pdf")).unwrap();
    let mut engine = HybridInterpreter::default();
    engine.mupdf.sandbox.max_pages = 1;
    // A fallback would try this deliberately nonexistent image instead.
    engine.ghostscript.sandbox.image = "jrip-must-not-run:missing".into();
    let error = engine
        .render(&source, &directory.path().join("limit"), &manifest)
        .await
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "interpreter_failed");
    std::fs::write(&source, b"%PDF-1.4\nbroken").unwrap();
    engine.mupdf = MuPdfInterpreter::default();
    let error = engine
        .render(&source, &directory.path().join("broken"), &manifest)
        .await
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "interpreter_failed");
    assert!(!directory.path().join("broken/page-000001.tiff").exists());
}
