//! Diagnostic only: original samples against the destination's ICC gamut.
//! The separate NULL-output proof transform cannot paint alarm colors into output.
use super::*;
use serde_json::{Value, json};
thread_local! { static NATIVE_ERROR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
// The callback never dereferences native pointers or records untrusted ICC text.
extern "C" fn native_error(_: lcms2_sys::Context, _: u32, _: *const std::ffi::c_char) {
    let _ = NATIVE_ERROR.try_with(|failed| failed.set(true));
}
pub const PROFILE: &str = "org.jrip.gamut-warning-v1";
pub const METHOD: &str = "lcms2-relative-colorimetric-null-proof-v1";
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamutReport {
    schema_version: u32,
    profile: &'static str,
    method: &'static str,
    reference_intent: &'static str,
    black_point_compensation: bool,
    modifies_pixels: bool,
    sampling: &'static str,
    pub status: &'static str,
    total_pixels: usize,
    pub checked_pixels: usize,
    pub out_of_gamut_pixels: Option<usize>,
    out_of_gamut_percent: Option<f64>,
    reason_code: Option<&'static str>,
    pub warnings: Vec<Value>,
}
impl GamutReport {
    fn unavailable(pixels: usize, reason: &'static str) -> Self {
        Self {
            schema_version: 1,
            profile: PROFILE,
            method: METHOD,
            reference_intent: "relative_colorimetric",
            black_point_compensation: false,
            modifies_pixels: false,
            sampling: "all-pixels",
            status: "NOT_ASSESSED",
            total_pixels: pixels,
            checked_pixels: 0,
            out_of_gamut_pixels: None,
            out_of_gamut_percent: None,
            reason_code: Some(reason),
            warnings: vec![
                json!({"code":"GAMUT_NOT_ASSESSED","messages":{"en":"Destination gamut could not be assessed; do not interpret this as in-gamut","ja":"出力色域を判定できません。色域内とは解釈しないでください","zh-CN":"无法评估输出色域，请勿视为色域内"}}),
            ],
        }
    }
}
pub(crate) fn assess(
    samples: &[u8],
    input_icc: &[u8],
    output_icc: Option<&[u8]>,
    input_color: Color,
    device_link: bool,
) -> GamutReport {
    let pixels = samples.len() / input_color.channels();
    if device_link {
        return GamutReport::unavailable(pixels, "DEVICE_LINK_NO_SEPARATE_OUTPUT_PROFILE");
    }
    let result = (|| {
        NATIVE_ERROR.with(|failed| failed.set(false));
        let mut context = ThreadContext::new();
        context.set_error_logging_function(Some(native_error));
        let mut codes = [0u16; 16];
        codes[0] = u16::MAX;
        context.set_alarm_codes(codes);
        let input = Profile::new_icc_context(&context, input_icc)
            .map_err(|_| "PROOF_TRANSFORM_UNAVAILABLE")?;
        let proof =
            Profile::new_icc_context(&context, output_icc.ok_or("PROOF_TRANSFORM_UNAVAILABLE")?)
                .map_err(|_| "PROOF_TRANSFORM_UNAVAILABLE")?;
        let null = Profile::new_null_context(&context);
        let check: Transform<u8, u16, ThreadContext> = Transform::new_proofing_context(
            &context,
            &input,
            input_color.format(),
            &null,
            PixelFormat::GRAY_16,
            &proof,
            lcms2::Intent::RelativeColorimetric,
            lcms2::Intent::RelativeColorimetric,
            Flags::SOFT_PROOFING | Flags::GAMUT_CHECK | Flags::NO_OPTIMIZE,
        )
        .map_err(|_| "PROOF_TRANSFORM_UNAVAILABLE")?;
        // LCMS can report an internal proof-pipeline error while returning a transform.
        // Such a transform must never be interpreted as an all-zero in-gamut mask.
        if NATIVE_ERROR.with(|failed| failed.get()) {
            return Err("PROOF_TRANSFORM_UNAVAILABLE");
        }
        let mut mask = [0u16; 4096];
        let mut outside = 0usize;
        for chunk in samples.chunks(4096 * input_color.channels()) {
            let count = chunk.len() / input_color.channels();
            check.transform_pixels(chunk, &mut mask[..count]);
            for flag in &mask[..count] {
                match *flag {
                    0 => (),
                    u16::MAX => outside += 1,
                    _ => return Err("UNEXPECTED_GAMUT_MASK"),
                }
            }
        }
        if NATIVE_ERROR.with(|failed| failed.get()) {
            return Err("PROOF_TRANSFORM_UNAVAILABLE");
        }
        Ok(outside)
    })();
    match result {
        Err(reason) => GamutReport::unavailable(pixels, reason),
        Ok(outside) => GamutReport {
            schema_version: 1,
            profile: PROFILE,
            method: METHOD,
            reference_intent: "relative_colorimetric",
            black_point_compensation: false,
            modifies_pixels: false,
            sampling: "all-pixels",
            status: "ASSESSED",
            total_pixels: pixels,
            checked_pixels: pixels,
            out_of_gamut_pixels: Some(outside),
            out_of_gamut_percent: Some(outside as f64 * 100.0 / pixels as f64),
            reason_code: None,
            warnings: if outside == 0 {
                vec![]
            } else {
                vec![
                    json!({"code":"OUT_OF_GAMUT","messages":{"en":"Some source pixels are outside the destination ICC gamut; output pixels were not altered by this warning","ja":"出力ICC色域外の原画素があります。この警告による出力画素の変更はありません","zh-CN":"部分原始像素超出输出ICC色域；警告不会改变输出像素"}}),
                ]
            },
        },
    }
}
/// Reject missing/legacy/inconsistent worker diagnostics instead of inventing zero clipping.
pub fn valid_report(value: &Value, pixels: u64, device_link: bool) -> bool {
    if value["schemaVersion"] != 1
        || value["profile"] != PROFILE
        || value["method"] != METHOD
        || value["referenceIntent"] != "relative_colorimetric"
        || value["blackPointCompensation"] != false
        || value["modifiesPixels"] != false
        || value["sampling"] != "all-pixels"
        || value["totalPixels"].as_u64() != Some(pixels)
        || pixels == 0
    {
        return false;
    }
    let Some(warnings) = value["warnings"].as_array() else {
        return false;
    };
    match value["status"].as_str() {
        Some("ASSESSED") if !device_link => {
            let Some(outside) = value["outOfGamutPixels"].as_u64().filter(|n| *n <= pixels) else {
                return false;
            };
            value["checkedPixels"].as_u64() == Some(pixels)
                && value["reasonCode"].is_null()
                && value["outOfGamutPercent"].as_f64().is_some_and(|n| {
                    n.is_finite() && (n - outside as f64 * 100.0 / pixels as f64).abs() < 1e-9
                })
                && if outside == 0 {
                    warnings.is_empty()
                } else {
                    warnings.len() == 1 && warnings[0]["code"] == "OUT_OF_GAMUT"
                }
        }
        Some("NOT_ASSESSED") => {
            value["checkedPixels"] == 0
                && value["outOfGamutPixels"].is_null()
                && value["outOfGamutPercent"].is_null()
                && warnings.len() == 1
                && warnings[0]["code"] == "GAMUT_NOT_ASSESSED"
                && if device_link {
                    value["reasonCode"] == "DEVICE_LINK_NO_SEPARATE_OUTPUT_PROFILE"
                } else {
                    matches!(
                        value["reasonCode"].as_str(),
                        Some("PROOF_TRANSFORM_UNAVAILABLE" | "UNEXPECTED_GAMUT_MASK")
                    )
                }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn narrow_rgb() -> Vec<u8> {
        let white = lcms2::CIExyY {
            x: 0.3127,
            y: 0.329,
            Y: 1.0,
        };
        let primaries = lcms2::CIExyYTRIPLE {
            Red: lcms2::CIExyY {
                x: 0.5,
                y: 0.33,
                Y: 1.0,
            },
            Green: lcms2::CIExyY {
                x: 0.3,
                y: 0.5,
                Y: 1.0,
            },
            Blue: lcms2::CIExyY {
                x: 0.2,
                y: 0.17,
                Y: 1.0,
            },
        };
        let gamma = lcms2::ToneCurve::new(2.2);
        Profile::new_rgb(&white, &primaries, &[&gamma, &gamma, &gamma])
            .unwrap()
            .icc()
            .unwrap()
    }
    fn policy() -> Policy {
        Policy {
            schema_version: 1,
            input_color: Color::Rgb,
            output_color: Color::Rgb,
            intent: Intent::RelativeColorimetric,
            black_point_compensation: false,
            black_preservation: BlackPreservation::None,
            tac_percent: None,
        }
    }
    #[test]
    fn identity_and_narrow_destination_distinguish_real_gamut_risk_without_changing_output() {
        let input = Profile::new_srgb().icc().unwrap();
        let narrow = narrow_rgb();
        let samples = [255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128];
        let (_, identity) = convert(&samples, &input, Some(&input), &policy()).unwrap();
        assert_eq!(identity.gamut.status, "ASSESSED");
        assert_eq!(identity.gamut.out_of_gamut_pixels, Some(0));
        assert!(identity.gamut.warnings.is_empty());
        let (pixels, report) = convert(&samples, &input, Some(&narrow), &policy()).unwrap();
        assert_eq!(report.gamut.status, "ASSESSED");
        assert!(report.gamut.out_of_gamut_pixels.unwrap() >= 3);
        assert_eq!(report.gamut.warnings[0]["code"], "OUT_OF_GAMUT");
        let source = Profile::new_icc(&input).unwrap();
        let destination = Profile::new_icc(&narrow).unwrap();
        let normal: Transform<u8, u8> = Transform::new(
            &source,
            PixelFormat::RGB_8,
            &destination,
            PixelFormat::RGB_8,
            lcms2::Intent::RelativeColorimetric,
        )
        .unwrap();
        let mut expected = vec![0; samples.len()];
        normal.transform_pixels(&samples, &mut expected);
        assert_eq!(pixels, expected); // The diagnostic mask never substitutes output pixels.
        assert!(valid_report(
            &serde_json::to_value(&report.gamut).unwrap(),
            4,
            false
        ));
    }
    #[test]
    fn all_pixels_including_chunk_tail_are_checked_and_grays_do_not_raise_alarm() {
        let input = Profile::new_srgb().icc().unwrap();
        let narrow = narrow_rgb();
        let mut samples = vec![128u8; 4101 * 3];
        samples[4096 * 3..4096 * 3 + 3].copy_from_slice(&[255, 0, 0]);
        samples[4100 * 3..].copy_from_slice(&[0, 0, 255]);
        let report = assess(&samples, &input, Some(&narrow), Color::Rgb, false);
        assert_eq!(report.checked_pixels, 4101);
        assert_eq!(report.out_of_gamut_pixels, Some(2));
        let neutral = assess(
            &[0, 0, 0, 128, 128, 128, 255, 255, 255],
            &input,
            Some(&narrow),
            Color::Rgb,
            false,
        );
        assert_eq!(neutral.out_of_gamut_pixels, Some(0));
    }
    #[test]
    fn unavailable_device_link_and_invalid_reports_never_mean_zero_risk() {
        let input = Profile::new_srgb().icc().unwrap();
        for (link, output) in [(true, None), (false, None)] {
            let report = assess(&[0, 0, 0], &input, output, Color::Rgb, link);
            assert_eq!(report.status, "NOT_ASSESSED");
            assert_eq!(report.out_of_gamut_pixels, None);
            assert_eq!(report.checked_pixels, 0);
            let value = serde_json::to_value(&report).unwrap();
            assert!(valid_report(&value, 1, link));
            let mut tampered = value.clone();
            tampered["outOfGamutPixels"] = json!(0);
            assert!(!valid_report(&tampered, 1, link));
        }
        let invalid = assess(&[0, 0, 0], &input, Some(b"invalid"), Color::Rgb, false);
        assert_eq!(invalid.status, "NOT_ASSESSED");
        let (_, report) = convert(&[0, 0, 0], &input, Some(&input), &policy()).unwrap();
        assert_eq!(report.gamut.status, "ASSESSED");
        let value = serde_json::to_value(report.gamut).unwrap();
        assert!(valid_report(&value, 1, false));
        for (key, replacement) in [
            ("profile", json!("other")),
            ("checkedPixels", json!(0)),
            ("outOfGamutPixels", json!(2)),
            ("outOfGamutPercent", json!(50)),
            ("warnings", json!([{"code":"OUT_OF_GAMUT"}])),
        ] {
            let mut bad = value.clone();
            bad[key] = replacement;
            assert!(!valid_report(&bad, 1, false), "{key}");
        }
        assert!(!valid_report(&Value::Null, 1, false));
    }
}

#[cfg(test)]
mod additional_layout_tests {
    use super::*;
    #[test]
    fn grayscale_destination_reports_checked_rgb_input_pixels() {
        let input = Profile::new_srgb().icc().unwrap();
        let white = lcms2::CIExyY {
            x: 0.3457,
            y: 0.3585,
            Y: 1.0,
        };
        let curve = lcms2::ToneCurve::new(2.2);
        let output = Profile::new_gray(&white, &curve).unwrap().icc().unwrap();
        let policy = Policy {
            schema_version: 1,
            input_color: Color::Rgb,
            output_color: Color::Gray,
            intent: Intent::RelativeColorimetric,
            black_point_compensation: false,
            black_preservation: BlackPreservation::None,
            tac_percent: None,
        };
        let (pixels, report) =
            convert(&[255, 0, 0, 128, 128, 128], &input, Some(&output), &policy).unwrap();
        assert_eq!(pixels.len(), 2);
        assert_eq!(report.gamut.status, "ASSESSED");
        assert_eq!(report.gamut.checked_pixels, 2);
        assert!(report.gamut.out_of_gamut_pixels.unwrap() >= 1);
    }
    #[test]
    #[ignore = "requires Docker jrip-ghostscript:local for its standard CMYK ICC fixture"]
    fn cmyk_output_and_input_profiles_are_assessed_with_real_engine_profile() {
        let output = std::process::Command::new("docker")
            .args([
                "run",
                "--rm",
                "--network=none",
                "--read-only",
                "--entrypoint=cat",
                "jrip-ghostscript:local",
                "/usr/share/color/icc/ghostscript/default_cmyk.icc",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stdout.len() <= MAX_PROFILE);
        let rgb = Profile::new_srgb().icc().unwrap();
        let cmyk = output.stdout;
        let mut policy = Policy {
            schema_version: 1,
            input_color: Color::Rgb,
            output_color: Color::Cmyk,
            intent: Intent::RelativeColorimetric,
            black_point_compensation: false,
            black_preservation: BlackPreservation::None,
            tac_percent: None,
        };
        let (pixels, report) = convert(
            &[255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128],
            &rgb,
            Some(&cmyk),
            &policy,
        )
        .unwrap();
        assert_eq!(report.gamut.status, "ASSESSED");
        assert_eq!(report.gamut.checked_pixels, 4);
        assert!(report.gamut.out_of_gamut_pixels.unwrap() > 0);
        policy.input_color = Color::Cmyk;
        policy.output_color = Color::Rgb;
        let (_, back) = convert(&pixels, &cmyk, Some(&rgb), &policy).unwrap();
        assert_eq!(back.gamut.status, "ASSESSED");
        assert_eq!(back.gamut.checked_pixels, 4);
    }
}
