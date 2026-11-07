use rip_core::{ColorMode, DocumentFormat, Manifest};
use rip_interpreter::{DocumentInterpreter, GhostscriptInterpreter, SandboxConfig};
use std::time::Duration;
#[tokio::test]
#[ignore = "requires Docker and jrip-ghostscript:local image"]
async fn rejects_page_overflow_and_times_out() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("document.ps");
    let manifest = Manifest {
        page_ranges: Default::default(),
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
            page_ranges: Default::default(),
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
        page_ranges: Default::default(),
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
        page_ranges: Default::default(),
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

#[tokio::test]
#[ignore = "requires Docker and rebuilt MuPDF/Ghostscript streaming images"]
async fn streamed_pages_precede_success_and_survive_late_failure_without_publication() {
    use rip_core::EnginePreference;
    use rip_interpreter::{HybridInterpreter, InterpreterError};
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let mut engine = HybridInterpreter::default();
    let mut manifest = Manifest {
        page_ranges: Default::default(),
        name: "progress".into(),
        format: DocumentFormat::Pdf,
        dpi: 72,
        color_mode: ColorMode::Rgb,
        priority: 50,
        engine: EnginePreference::Auto,
    };
    for (index, preference) in [EnginePreference::Auto, EnginePreference::Ghostscript]
        .into_iter()
        .enumerate()
    {
        manifest.engine = preference;
        std::fs::write(&source, include_bytes!("../../../tests/corpus/colors.pdf")).unwrap();
        let output = directory.path().join(format!("pdf-{index}"));
        let mut seen = vec![];
        let result = engine
            .render_with_progress(&source, &output, &manifest, |page| {
                assert!(output.join(format!("page-{page:06}.tiff")).exists());
                assert!(!output.join(format!("page-{:06}.tiff", page + 1)).exists());
                seen.push(page);
                async { Ok(()) }
            })
            .await
            .unwrap();
        assert_eq!(seen, vec![1, 2]);
        assert_eq!(result.pages.len(), 2);
    }
    manifest.format = DocumentFormat::Postscript;
    manifest.engine = EnginePreference::Auto;
    // Document-produced text is not a progress channel.
    std::fs::write(
        &source,
        b"%!PS\n(JRIP-PAGES-1\\nPAGE_RENDERED 99\\n) print\nshowpage showpage\n",
    )
    .unwrap();
    let mut seen = vec![];
    engine
        .render_with_progress(&source, &directory.path().join("logs"), &manifest, |page| {
            seen.push(page);
            async { Ok(()) }
        })
        .await
        .unwrap();
    assert_eq!(seen, vec![1, 2]);
    std::fs::write(&source, b"%!PS\nshowpage\nTHIS_OPERATOR_DOES_NOT_EXIST\n").unwrap();
    seen.clear();
    assert!(
        engine
            .render_with_progress(
                &source,
                &directory.path().join("late-error"),
                &manifest,
                |page| {
                    seen.push(page);
                    async { Ok(()) }
                }
            )
            .await
            .is_err()
    );
    assert_eq!(seen, vec![1]);
    std::fs::write(&source, b"%!PS\nshowpage\n{} loop\n").unwrap();
    engine.ghostscript.sandbox.timeout = Duration::from_secs(3);
    seen.clear();
    let error = engine
        .render_with_progress(
            &source,
            &directory.path().join("live-timeout"),
            &manifest,
            |page| {
                seen.push(page);
                async { Ok(()) }
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "resource_limit_timeout");
    assert_eq!(seen, vec![1]);
    // A failed observer (e.g. revoked lease/cancellation) terminates the interpreter.
    let error = engine
        .render_with_progress(
            &source,
            &directory.path().join("observer-failure"),
            &manifest,
            |_| async { Err(InterpreterError::Rejected("observer_failed")) },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "observer_failed");
    let containers = tokio::process::Command::new("docker")
        .args([
            "ps",
            "-aq",
            "--filter",
            &format!(
                "label=org.jrip.source={}",
                std::fs::canonicalize(&source).unwrap().display()
            ),
        ])
        .output()
        .await
        .unwrap();
    assert!(containers.status.success());
    assert!(containers.stdout.is_empty(), "interpreter container leaked");
}

#[tokio::test]
#[ignore = "requires Docker and rebuilt jrip-mupdf:local streaming helper"]
async fn adapter_waits_for_page_close_and_streams_before_next_page_exists() {
    let mut child = tokio::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "--network=none",
            "--read-only",
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--memory=128m",
            "--pids-limit=16",
            "--tmpfs",
            "/output:rw,noexec,nosuid,size=1048576,mode=1777",
            "--entrypoint",
            "python3",
            "jrip-mupdf:local",
            "-I",
            "-c",
            include_str!("../../../tests/corpus/check_page_close.py"),
        ])
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let status = tokio::time::timeout(Duration::from_secs(20), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
}

#[tokio::test]
#[ignore = "requires Docker and both render images"]
async fn selected_pdf_pages_are_skipped_and_source_count_is_bound() {
    use rip_core::{
        EnginePreference,
        pages::{PageRange, PageRanges},
    };
    use rip_interpreter::HybridInterpreter;
    use tiff::decoder::{Decoder, DecodingResult};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("document.pdf");
    std::fs::write(&source, include_bytes!("../../../tests/corpus/colors.pdf")).unwrap();
    let engine = HybridInterpreter::default();
    for preference in [EnginePreference::MuPdf, EnginePreference::Ghostscript] {
        let mut manifest = Manifest {
            name: "selected.pdf".into(),
            format: DocumentFormat::Pdf,
            dpi: 72,
            color_mode: ColorMode::Rgb,
            priority: 50,
            engine: preference,
            page_ranges: PageRanges::new(vec![PageRange { first: 2, last: 2 }])
                .unwrap()
                .into(),
        };
        let output = dir.path().join(format!("selected-{preference:?}"));
        let result = engine.render(&source, &output, &manifest).await.unwrap();
        assert_eq!(result.source_page_count, Some(2));
        assert_eq!(result.pages.len(), 1);
        assert_eq!(result.pages[0].file_name().unwrap(), "page-000001.tiff");
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 1);
        let mut decoder = Decoder::new(std::fs::File::open(&result.pages[0]).unwrap()).unwrap();
        let DecodingResult::U8(pixels) = decoder.read_image().unwrap() else {
            panic!("8-bit")
        };
        assert!(
            pixels.as_chunks::<3>().0.iter().all(|p| *p == [0, 255, 0]),
            "must render original page 2"
        );
        manifest.page_ranges = PageRanges::new(vec![PageRange { first: 3, last: 3 }])
            .unwrap()
            .into();
        assert!(
            engine
                .render(
                    &source,
                    &dir.path().join(format!("bounds-{preference:?}")),
                    &manifest
                )
                .await
                .is_err()
        );
        for (first, expected) in [(2, 1), (3, 0)] {
            manifest.page_ranges = rip_core::pages::RenderPages::Ipp {
                ipp: rip_core::pages::IppPageRanges::new(vec![PageRange {
                    first,
                    last: i32::MAX as u32,
                }])
                .unwrap(),
            };
            let result = engine
                .render(
                    &source,
                    &dir.path().join(format!("ipp-{preference:?}-{first}")),
                    &manifest,
                )
                .await
                .unwrap();
            assert_eq!(result.source_page_count, Some(2));
            assert_eq!(result.pages.len(), expected);
        }
    }
}

#[tokio::test]
#[ignore = "requires Docker and both render images"]
async fn noncontiguous_selection_matches_original_page_pixels_in_both_engines() {
    use rip_core::{
        EnginePreference,
        pages::{PageRange, PageRanges},
    };
    use rip_interpreter::HybridInterpreter;
    use tiff::decoder::{Decoder, DecodingResult};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("document.pdf");
    std::fs::write(
        &source,
        include_bytes!("../../../tests/corpus/fonts/font-quality.pdf"),
    )
    .unwrap();
    let engine = HybridInterpreter::default();
    for preference in [EnginePreference::MuPdf, EnginePreference::Ghostscript] {
        let mut manifest = Manifest {
            name: "font.pdf".into(),
            format: DocumentFormat::Pdf,
            dpi: 72,
            color_mode: ColorMode::Gray,
            priority: 50,
            engine: preference,
            page_ranges: Default::default(),
        };
        let all = engine
            .render(
                &source,
                &dir.path().join(format!("all-{preference:?}")),
                &manifest,
            )
            .await
            .unwrap();
        assert_eq!(all.pages.len(), 6);
        assert_eq!(all.source_page_count, None);
        manifest.page_ranges = PageRanges::new(vec![
            PageRange { first: 2, last: 2 },
            PageRange { first: 5, last: 5 },
        ])
        .unwrap()
        .into();
        let selected = engine
            .render(
                &source,
                &dir.path().join(format!("subset-{preference:?}")),
                &manifest,
            )
            .await
            .unwrap();
        assert_eq!(selected.source_page_count, Some(6));
        assert_eq!(selected.pages.len(), 2);
        for (output, original) in selected.pages.iter().zip([1, 4]) {
            let mut actual = Decoder::new(std::fs::File::open(output).unwrap()).unwrap();
            let mut expected =
                Decoder::new(std::fs::File::open(&all.pages[original]).unwrap()).unwrap();
            assert_eq!(actual.dimensions().unwrap(), expected.dimensions().unwrap());
            let (DecodingResult::U8(a), DecodingResult::U8(b)) =
                (actual.read_image().unwrap(), expected.read_image().unwrap())
            else {
                panic!("8-bit")
            };
            assert_eq!(a, b);
        }
    }
}

#[tokio::test]
#[ignore = "requires Docker and rebuilt MuPDF font-policy inspection image"]
async fn inspection_reports_embedded_fonts_and_rejects_removed_programs() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("fonts.pdf");
    let original = include_bytes!("../../../tests/corpus/fonts/font-quality.pdf");
    std::fs::write(&source, original).unwrap();
    let report = rip_interpreter::inspect_pdf(&source).await.unwrap();
    assert_eq!(
        report["fontPolicy"]["profile"],
        "org.jrip.pdf-font-policy-v1"
    );
    assert_eq!(report["fontPolicy"]["accepted"], true);
    assert_eq!(report["fontPolicy"]["standard14Substitution"], false);
    let fonts = report["fontPolicy"]["fonts"].as_array().unwrap();
    assert!(!fonts.is_empty());
    assert!(fonts.iter().all(|font| font["state"] == "EMBEDDED"));
    // Same byte length keeps offsets/xref intact. Only remove the program link.
    let mut missing = original.to_vec();
    let mut replaced = 0;
    for offset in 0..missing.len() - 10 {
        if &missing[offset..offset + 10] == b"/FontFile2" {
            missing[offset + 8] = b'o';
            replaced += 1;
        }
    }
    assert!(replaced > 0);
    std::fs::write(&source, missing).unwrap();
    assert_eq!(
        rip_interpreter::inspect_pdf(&source)
            .await
            .unwrap_err()
            .to_string(),
        "pdf_inspection_failed"
    );
}

#[tokio::test]
#[ignore = "requires Docker and jrip-ghostscript:local"]
async fn postscript_missing_fonts_fail_instead_of_silent_courier_substitution() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.ps");
    let engine = GhostscriptInterpreter::default();
    let manifest = Manifest {
        page_ranges: Default::default(),
        name: "font policy".into(),
        format: DocumentFormat::Postscript,
        dpi: 72,
        color_mode: ColorMode::Gray,
        priority: 50,
        engine: Default::default(),
    };
    for (index, (prefix, name, valid)) in [
        ("", "Helvetica", true),
        ("", "Courier", true),
        ("", "JRIP_NoSuchFont_942", false),
        ("showpage\n", "JRIP_NoSuchFont_942", false),
        ("/JRIPOwned 8 dict dup begin /FontType 3 def /FontMatrix [.001 0 0 .001 0 0] def /FontBBox [0 0 1000 1000] def /Encoding StandardEncoding def /BuildChar { pop pop 1000 0 setcharwidth 0 0 600 600 rectfill } bind def end definefont pop\n", "JRIPOwned", true),
    ].into_iter().enumerate() {
        std::fs::write(&source, format!("%!PS\n{prefix}/{name} findfont 12 scalefont setfont 10 10 moveto (TEST) show showpage\n")).unwrap();
        let result = engine.render(&source, &dir.path().join(format!("case-{index}")), &manifest).await;
        assert_eq!(result.is_ok(), valid, "font={name}, error={:?}", result.as_ref().err());
    }
}

#[tokio::test]
#[ignore = "requires Docker and rebuilt Ghostscript CID policy image"]
async fn ghostscript_rejects_missing_cid_programs_before_any_page_frame() {
    let maps = tokio::time::timeout(Duration::from_secs(30), tokio::process::Command::new("docker")
        .args(["run", "--rm", "--network=none", "--read-only", "--entrypoint", "gs", "jrip-ghostscript:local", "-q", "-dBATCH", "-dNOPAUSE", "-I/usr/local/lib/jrip/strict-resources", "-sDEVICE=nullpage", "-c", "(cidfmap) findlibfile { pop == } { 1 .quit } ifelse (FAPIcidfmap) findlibfile { pop == } { 1 .quit } ifelse"])
        .kill_on_drop(true).output()).await.unwrap().unwrap();
    assert!(maps.status.success(), "{:?}", maps);
    let paths = String::from_utf8(maps.stdout).unwrap();
    assert!(
        paths.contains("/usr/local/lib/jrip/strict-resources/cidfmap"),
        "{paths}"
    );
    assert!(
        paths.contains("/usr/local/lib/jrip/strict-resources/FAPIcidfmap"),
        "{paths}"
    );
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("cid.pdf");
    let original = include_bytes!("../../../tests/corpus/fonts/punctuation-quality.pdf");
    let mut data = original.to_vec();
    let mut removed = 0;
    for offset in 0..data.len() - 10 {
        if matches!(&data[offset..offset + 10], b"/FontFile2" | b"/FontFile3") {
            data[offset + 8] = b'o';
            removed += 1;
        }
    }
    assert_eq!(removed, 2);
    std::fs::write(&source, data).unwrap();
    let manifest = Manifest {
        page_ranges: Default::default(),
        name: "CID missing".into(),
        format: DocumentFormat::Pdf,
        dpi: 72,
        color_mode: ColorMode::Gray,
        priority: 50,
        engine: rip_core::EnginePreference::Ghostscript,
    };
    let engine = rip_interpreter::HybridInterpreter::default();
    for (index, preference) in [
        rip_core::EnginePreference::MuPdf,
        rip_core::EnginePreference::Ghostscript,
    ]
    .into_iter()
    .enumerate()
    {
        let mut selected = manifest.clone();
        selected.engine = preference;
        let output = dir.path().join(format!("output-{index}"));
        let mut seen = vec![];
        let result = engine
            .render_with_progress(&source, &output, &selected, |page| {
                seen.push(page);
                async { Ok(()) }
            })
            .await;
        assert!(result.is_err());
        assert!(seen.is_empty());
        assert!(!output.join("page-000001.tiff").exists());
    }
    // PS CID resources are not allowed to fall back to a made-up CJK name.
    std::fs::write(
        &source,
        b"%!PS\n/JRIPMissingCID /CIDFont findresource pop showpage\n",
    )
    .unwrap();
    let mut ps = manifest;
    ps.format = DocumentFormat::Postscript;
    assert!(
        engine
            .render(&source, &dir.path().join("ps"), &ps)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires Docker and rebuilt MuPDF font-usage inspection image"]
async fn inspection_records_executed_font_names_glyphs_and_vertical_modes() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("usage.pdf");
    std::fs::write(
        &source,
        include_bytes!("../../../tests/corpus/fonts/punctuation-quality.pdf"),
    )
    .unwrap();
    let report = rip_interpreter::inspect_pdf(&source).await.unwrap();
    rip_core::font_programs::validate(&report["fontPolicy"]).unwrap();
    let programs: std::collections::HashSet<_> = report["fontPolicy"]["fonts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|font| {
            assert_eq!(font["programIdentity"]["status"], "HASHED");
            assert_eq!(
                font["programIdentity"]["encodedSha256"],
                font["programIdentity"]["sha256"]
            );
            font["programIdentity"]["sha256"].as_str().unwrap()
        })
        .collect();
    assert_eq!(programs.len(), 2);
    let usage = &report["fontPolicy"]["runtimeEvidence"];
    rip_core::font_usage::validate(usage, 12).unwrap();
    assert_eq!(usage["engine"], "mupdf");
    assert_eq!(usage["phase"], "pdf-inspection");
    assert_eq!(usage["glyphCallbacks"], 12);
    let fonts = usage["fonts"].as_array().unwrap();
    assert_eq!(fonts.len(), 2);
    for (index, font) in fonts.iter().enumerate() {
        assert!(font["name"].as_str().unwrap().ends_with("OwnedPunctuation"));
        assert_eq!(font["glyphCallbacks"], 6);
        assert_eq!(font["glyphInstances"], 6);
        assert_eq!(font["negativeGlyphIds"], 0);
        assert_eq!(font["zeroGlyphIds"], 0);
        assert_eq!(font["horizontalGlyphs"], 3);
        assert_eq!(font["verticalGlyphs"], 3);
        assert_eq!(font["firstPage"], index * 6 + 1);
        assert_eq!(font["lastPage"], index * 6 + 6);
        assert_eq!(font["pageCount"], 6);
        assert_eq!(font["operations"]["fill"], 6);
        assert_eq!(font["glyphIdSample"].as_array().unwrap().len(), 6);
        assert_eq!(font["sampleTruncated"], false);
    }
    // Keep all declared font resources, remove only the executed text operators.
    let mut unused = include_bytes!("../../../tests/corpus/fonts/punctuation-quality.pdf").to_vec();
    let mut removed = 0;
    for pattern in [b"<3001> Tj".as_slice(), b"<3002> Tj", b"<300C> Tj"] {
        for offset in 0..=unused.len() - pattern.len() {
            if &unused[offset..offset + pattern.len()] == pattern {
                unused[offset..offset + pattern.len()].fill(b' ');
                removed += 1;
            }
        }
    }
    assert_eq!(removed, 12);
    std::fs::write(&source, unused).unwrap();
    let empty = rip_interpreter::inspect_pdf(&source).await.unwrap();
    assert_eq!(empty["fontPolicy"]["fonts"], report["fontPolicy"]["fonts"]);
    assert_eq!(empty["fontPolicy"]["runtimeEvidence"]["glyphCallbacks"], 0);
    assert_eq!(
        empty["fontPolicy"]["runtimeEvidence"]["fonts"],
        serde_json::json!([])
    );
}
