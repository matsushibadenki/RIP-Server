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

#[tokio::test]
#[ignore = "requires Docker and rebuilt jrip-mupdf:local inspection adapter"]
async fn inspection_resolves_inherited_crop_rotation_and_user_unit() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("geometry.pdf");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 72 144] /CropBox [0 0 36 72] /Rotate 90 >>",
        "<< /Type /Page /Parent 2 0 R /Resources << >> /UserUnit 2 >>",
    ];
    let mut pdf = "%PDF-1.7\n".to_string();
    let mut offsets = vec![];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", index + 1));
    }
    let xref = pdf.len();
    pdf.push_str("xref\n0 4\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    std::fs::write(&source, &pdf).unwrap();
    let report = rip_interpreter::inspect_pdf(&source).await.unwrap();
    assert_eq!(report["pages"][0]["widthPoints"].as_f64(), Some(144.0));
    assert_eq!(report["pages"][0]["heightPoints"].as_f64(), Some(72.0));
    let broken = pdf.replace(&format!("startxref\n{xref}"), "startxref\n0");
    std::fs::write(&source, broken).unwrap();
    assert!(rip_interpreter::inspect_pdf(&source).await.is_err());
}
