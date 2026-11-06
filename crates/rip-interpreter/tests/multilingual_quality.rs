//! Independent font-outline probes against both production raster engines.
use rip_core::{ColorMode, DocumentFormat, EnginePreference, Manifest};
use rip_interpreter::{DocumentInterpreter, HybridInterpreter};
use serde_json::Value;
use std::path::PathBuf;
use tiff::decoder::{Decoder, DecodingResult};

fn mismatches(paths: &[PathBuf], expected: &Value, channels: usize) -> usize {
    let pages = expected["pages"].as_array().unwrap();
    assert_eq!(paths.len(), pages.len());
    let mut errors = 0;
    for (index, path) in paths.iter().enumerate() {
        let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
        let (width, height) = decoder.dimensions().unwrap();
        assert_eq!(u64::from(width), expected["width"].as_u64().unwrap());
        assert_eq!(u64::from(height), expected["height"].as_u64().unwrap());
        assert_eq!(
            decoder.colortype().unwrap(),
            if channels == 1 {
                tiff::ColorType::Gray(8)
            } else {
                tiff::ColorType::RGB(8)
            }
        );
        let DecodingResult::U8(pixels) = decoder.read_image().unwrap() else {
            panic!("8-bit raster required")
        };
        assert_eq!(pixels.len(), width as usize * height as usize * channels);
        for probe in pages[index]["probes"].as_array().unwrap() {
            let x = probe[0].as_u64().unwrap() as usize;
            let y = probe[1].as_u64().unwrap() as usize;
            let value = probe[2].as_u64().unwrap() as u8;
            assert!(x < width as usize && y < height as usize);
            let offset = (y * width as usize + x) * channels;
            // Permit one pixel of hinting phase for ink; whitespace stays exact.
            let mismatch = if value == 0 {
                !(y.saturating_sub(1)..=(y + 1).min(height as usize - 1)).any(|py| {
                    (x.saturating_sub(1)..=(x + 1).min(width as usize - 1)).any(|px| {
                        let at = (py * width as usize + px) * channels;
                        pixels[at..at + channels].iter().all(|&sample| sample <= 75)
                    })
                })
            } else {
                pixels[offset..offset + channels]
                    .iter()
                    .any(|&sample| sample < 180)
            };
            if mismatch {
                errors += 1;
                if errors <= 5 {
                    eprintln!(
                        "page {} ({x},{y}): expected {value}, got {:?}",
                        index + 1,
                        &pixels[offset..offset + channels]
                    );
                }
            }
        }
        for (x, y) in [(12, 12), (width as usize - 12, height as usize - 12)] {
            let offset = (y * width as usize + x) * channels;
            assert!(
                pixels[offset..offset + channels]
                    .iter()
                    .all(|&sample| sample == 255)
            );
        }
    }
    errors
}

#[tokio::test]
#[ignore = "requires Docker and rebuilt MuPDF/Ghostscript images"]
async fn both_engines_match_independent_multilingual_font_outline_probes() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("multilingual.pdf");
    let original = include_bytes!("../../../tests/corpus/multilingual/multilingual-quality.pdf");
    std::fs::write(&source, original).unwrap();
    let expected: Value = serde_json::from_slice(include_bytes!(
        "../../../tests/corpus/multilingual/probes.json"
    ))
    .unwrap();
    let interpreter = HybridInterpreter::default();
    let mut manifest = Manifest {
        page_ranges: Default::default(),
        name: "multilingual proof".into(),
        format: DocumentFormat::Pdf,
        dpi: expected["dpi"].as_u64().unwrap() as u32,
        color_mode: ColorMode::Rgb,
        priority: 50,
        engine: EnginePreference::MuPdf,
    };
    for (engine_index, engine) in [EnginePreference::MuPdf, EnginePreference::Ghostscript]
        .into_iter()
        .enumerate()
    {
        for (color_index, (mode, channels)) in [(ColorMode::Rgb, 3), (ColorMode::Gray, 1)]
            .into_iter()
            .enumerate()
        {
            manifest.engine = engine;
            manifest.color_mode = mode;
            let output = dir
                .path()
                .join(format!("engine-{engine_index}-{color_index}"));
            let rendered = interpreter
                .render(&source, &output, &manifest)
                .await
                .unwrap();
            if let Some(qa) = std::env::var_os("JRIP_MULTILINGUAL_QA") {
                let target =
                    std::path::Path::new(&qa).join(format!("engine-{engine_index}-{color_index}"));
                std::fs::create_dir_all(&target).unwrap();
                for (page, path) in rendered.pages.iter().enumerate() {
                    std::fs::copy(path, target.join(format!("page-{}.tiff", page + 1))).unwrap();
                }
            }
            assert_eq!(
                mismatches(&rendered.pages, &expected, channels),
                0,
                "engine={engine:?}, mode={mode:?}"
            );
        }
    }
    // Keep program data and ToUnicode intact, but map C to D in Encoding CMaps.
    // Equal lengths preserve the original xref. The outline oracle must reject it.
    let mut wrong = original.to_vec();
    let mut replacements = 0;
    for offset in 0..wrong.len() - 9 {
        if &wrong[offset..offset + 9] == b"<0043> 36" {
            wrong[offset + 8] = b'7';
            replacements += 1;
        }
    }
    assert_eq!(replacements, 2);
    std::fs::write(&source, wrong).unwrap();
    manifest.engine = EnginePreference::MuPdf;
    manifest.color_mode = ColorMode::Gray;
    let rendered = interpreter
        .render(&source, &dir.path().join("wrong-cid"), &manifest)
        .await
        .unwrap();
    assert!(
        mismatches(&rendered.pages, &expected, 1) > 0,
        "wrong CID selection was not detected"
    );
}
