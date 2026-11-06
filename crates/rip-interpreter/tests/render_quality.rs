//! Semantic pixel probes, not engine-generated golden images.
use rip_core::{ColorMode, EnginePreference, Manifest};
use rip_interpreter::{DocumentInterpreter, HybridInterpreter};
use tiff::decoder::{Decoder, DecodingResult};

struct Page {
    width: usize,
    height: usize,
    channels: usize,
    pixels: Vec<u8>,
}
impl Page {
    fn open(path: &std::path::Path, channels: usize) -> Self {
        let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
        let (width, height) = decoder.dimensions().unwrap();
        assert_eq!((width, height), (72, 72));
        assert_eq!(
            decoder.colortype().unwrap(),
            match channels {
                1 => tiff::ColorType::Gray(8),
                3 => tiff::ColorType::RGB(8),
                4 => tiff::ColorType::CMYK(8),
                _ => panic!("unsupported fixture channels"),
            }
        );
        let DecodingResult::U8(pixels) = decoder.read_image().unwrap() else {
            panic!("8-bit raster required")
        };
        assert_eq!(pixels.len(), width as usize * height as usize * channels);
        Self {
            width: width as usize,
            height: height as usize,
            channels,
            pixels,
        }
    }
    // Fixture probes are in PDF's lower-left coordinates, away from antialiased edges.
    fn probe(&self, x: usize, y: usize, expected: &[u8], tolerance: u8) {
        assert_eq!(expected.len(), self.channels);
        let index = ((self.height - 1 - y) * self.width + x) * self.channels;
        let actual = &self.pixels[index..index + self.channels];
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(a, b)| a.abs_diff(*b) <= tolerance),
            "({x},{y}): actual={actual:?}, expected={expected:?}"
        );
    }
}
#[tokio::test]
#[ignore = "requires Docker and local MuPDF/Ghostscript images"]
async fn both_engines_preserve_transparency_blending_overprint_and_embedded_japanese_glyphs() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("quality.pdf");
    std::fs::write(
        &source,
        include_bytes!("../../../tests/corpus/render-quality.pdf"),
    )
    .unwrap();
    for engine in [EnginePreference::MuPdf, EnginePreference::Ghostscript] {
        for color in [ColorMode::Rgb, ColorMode::Cmyk] {
            let manifest:Manifest=serde_json::from_value(serde_json::json!({"name":"quality.pdf","format":"application/pdf","dpi":72,"color_mode":color,"engine":engine})).unwrap();
            let output = directory.path().join(format!("{engine:?}-{color:?}"));
            let rendered = HybridInterpreter::default()
                .render(&source, &output, &manifest)
                .await
                .unwrap();
            assert_eq!(rendered.pages.len(), 4, "{engine:?}/{color:?}");
            let pages: Vec<_> = rendered
                .pages
                .iter()
                .map(|p| Page::open(p, if color == ColorMode::Rgb { 3 } else { 4 }))
                .collect();
            if color == ColorMode::Rgb {
                pages[0].probe(16, 16, &[0, 0, 255], 3);
                pages[0].probe(40, 40, &[128, 0, 128], 4);
                pages[0].probe(60, 60, &[255, 128, 128], 4);
                pages[1].probe(16, 16, &[0, 255, 255], 3);
                pages[1].probe(40, 40, &[0, 0, 0], 4);
                pages[1].probe(60, 60, &[255, 0, 0], 3);
                // Distinct positive and negative interior probes prevent blank/tofu regressions.
                pages[3].probe(16, 28, &[0, 0, 0], 3);
                pages[3].probe(24, 32, &[255, 255, 255], 3);
                pages[3].probe(48, 36, &[0, 0, 0], 3);
                pages[3].probe(37, 46, &[255, 255, 255], 3);
            } else {
                pages[2].probe(8, 16, &[255, 0, 0, 0], 2);
                pages[2].probe(20, 16, &[255, 0, 255, 0], 2);
                pages[2].probe(20, 48, &[0, 0, 255, 0], 2);
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires Docker and local MuPDF/Ghostscript images"]
async fn both_engines_render_subset_truetype_cid_mapping_and_cff_without_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("font-quality.pdf");
    std::fs::write(
        &source,
        include_bytes!("../../../tests/corpus/fonts/font-quality.pdf"),
    )
    .unwrap();
    for engine in [EnginePreference::MuPdf, EnginePreference::Ghostscript] {
        for color in [ColorMode::Rgb, ColorMode::Gray] {
            let manifest: Manifest = serde_json::from_value(serde_json::json!({"name":"font-quality.pdf","format":"application/pdf","dpi":72,"color_mode":color,"engine":engine})).unwrap();
            let rendered = HybridInterpreter::default()
                .render(
                    &source,
                    &directory.path().join(format!("{engine:?}-{color:?}")),
                    &manifest,
                )
                .await
                .unwrap();
            assert_eq!(rendered.pages.len(), 3, "{engine:?}");
            for (index, path) in rendered.pages.iter().enumerate() {
                eprintln!("font regression: {engine:?}/{color:?}, page {}", index + 1);
                let channels = if color == ColorMode::Gray { 1 } else { 3 };
                let page = Page::open(path, channels);
                let black = vec![0; channels];
                let white = vec![255; channels];
                // 日: filled frame and two separate counters, with clear background.
                page.probe(16, 28, &black, 3);
                page.probe(24, 32, &white, 3);
                page.probe(28, 40, &white, 3);
                page.probe(23, 39, &black, 3); // Owned sentinel, absent in fallback fonts.
                page.probe(24, 36, &black, 3);
                // 本: stem ink and whitespace distinguish it from 日 and .notdef boxes.
                page.probe(48, 36, &black, 3);
                page.probe(37, 46, &white, 3);
                page.probe(4, 4, &white, 3);
            }
        }
    }
}
