//! Explicit, bounded ICC transforms independent of the RIP interpreter.
pub mod gamut;
pub mod raster;
pub mod registry;
use lcms2::{ColorSpaceSignature, Flags, PixelFormat, Profile, ThreadContext, Transform};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
const MAX_PROFILE: usize = 4 * 1024 * 1024;
const MAX_SAMPLES: usize = 64 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Tiff(#[from] tiff::TiffError),
    #[error("invalid_icc_profile")]
    Profile,
    #[error("unsupported_color_policy")]
    Policy,
    #[error("invalid_raster_samples_or_budget")]
    Samples,
    #[error("icc_transform_failed")]
    Transform,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    Rgb,
    Cmyk,
    Gray,
}
impl Color {
    pub fn channels(self) -> usize {
        match self {
            Self::Rgb => 3,
            Self::Cmyk => 4,
            Self::Gray => 1,
        }
    }
    fn format(self) -> PixelFormat {
        match self {
            Self::Rgb => PixelFormat::RGB_8,
            Self::Cmyk => PixelFormat::CMYK_8,
            Self::Gray => PixelFormat::GRAY_8,
        }
    }
    fn signature(self) -> ColorSpaceSignature {
        match self {
            Self::Rgb => ColorSpaceSignature::RgbData,
            Self::Cmyk => ColorSpaceSignature::CmykData,
            Self::Gray => ColorSpaceSignature::GrayData,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    Perceptual,
    RelativeColorimetric,
    AbsoluteColorimetric,
    Saturation,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlackPreservation {
    None,
    KOnly,
    KPlane,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub schema_version: u32,
    pub input_color: Color,
    pub output_color: Color,
    pub intent: Intent,
    pub black_point_compensation: bool,
    pub black_preservation: BlackPreservation,
    pub tac_percent: Option<u16>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub schema_version: u32,
    pub engine: &'static str,
    pub input_profile_sha256: String,
    pub output_profile_sha256: Option<String>,
    pub policy_sha256: String,
    pub source_sha256: String,
    pub output_sha256: String,
    pub pixels: usize,
    pub device_link: bool,
    pub maximum_output_ink_percent: Option<f64>,
    pub policy: Policy,
    pub gamut: gamut::GamutReport,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
/// Check ICC envelope and tag ranges before entering the C profile parser.
pub fn validate_profile(bytes: &[u8]) -> Result<(), Error> {
    if bytes.len() < 132
        || bytes.len() > MAX_PROFILE
        || &bytes[36..40] != b"acsp"
        || u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize != bytes.len()
    {
        return Err(Error::Profile);
    }
    if !matches!(&bytes[16..20], b"RGB " | b"CMYK" | b"GRAY")
        || u32::from_be_bytes(bytes[64..68].try_into().unwrap()) > 3
    {
        return Err(Error::Profile);
    }
    let link = &bytes[12..16] == b"link";
    if (link && !matches!(&bytes[20..24], b"RGB " | b"CMYK" | b"GRAY"))
        || (!link && !matches!(&bytes[20..24], b"XYZ " | b"Lab "))
    {
        return Err(Error::Profile);
    }
    let tags = u32::from_be_bytes(bytes[128..132].try_into().unwrap()) as usize;
    if tags > 1024 || 132 + tags * 12 > bytes.len() {
        return Err(Error::Profile);
    }
    for tag in bytes[132..132 + tags * 12].as_chunks::<12>().0 {
        let offset = u32::from_be_bytes(tag[4..8].try_into().unwrap()) as usize;
        let length = u32::from_be_bytes(tag[8..12].try_into().unwrap()) as usize;
        if offset < 132 + tags * 12
            || offset
                .checked_add(length)
                .is_none_or(|end| end > bytes.len())
        {
            return Err(Error::Profile);
        }
    }
    Ok(())
}
fn rendering_intent(policy: &Policy) -> Result<lcms2::Intent, Error> {
    use BlackPreservation::*;
    use Intent::*;
    use lcms2::Intent as I;
    Ok(match (policy.black_preservation, policy.intent) {
        (None, Perceptual) => I::Perceptual,
        (None, RelativeColorimetric) => I::RelativeColorimetric,
        (None, AbsoluteColorimetric) => I::AbsoluteColorimetric,
        (None, Saturation) => I::Saturation,
        (KOnly, Perceptual) => I::PreserveKOnlyPerceptual,
        (KOnly, RelativeColorimetric) => I::PreserveKOnlyRelativeColorimetric,
        (KOnly, Saturation) => I::PreserveKOnlySaturation,
        (KPlane, Perceptual) => I::PreserveKPlanePerceptual,
        (KPlane, RelativeColorimetric) => I::PreserveKPlaneRelativeColorimetric,
        (KPlane, Saturation) => I::PreserveKPlaneSaturation,
        _ => return Err(Error::Policy),
    })
}
/// No inferred source profile: callers must provide the exact profile bytes.
/// DeviceLink mode uses a link input and no output profile. A link defines its own mapping.
pub fn convert(
    samples: &[u8],
    input_icc: &[u8],
    output_icc: Option<&[u8]>,
    policy: &Policy,
) -> Result<(Vec<u8>, Report), Error> {
    validate_policy(policy)?;
    convert_validated(samples, input_icc, output_icc, policy)
}
pub fn validate_policy(policy: &Policy) -> Result<(), Error> {
    if policy.schema_version != 1
        || policy
            .tac_percent
            .is_some_and(|tac| !(100..=400).contains(&tac))
        || (policy.tac_percent.is_some() && policy.output_color != Color::Cmyk)
        || (policy.black_preservation != BlackPreservation::None
            && (policy.input_color != Color::Cmyk || policy.output_color != Color::Cmyk))
    {
        return Err(Error::Policy);
    }
    rendering_intent(policy)?;
    Ok(())
}
fn convert_validated(
    samples: &[u8],
    input_icc: &[u8],
    output_icc: Option<&[u8]>,
    policy: &Policy,
) -> Result<(Vec<u8>, Report), Error> {
    if samples.is_empty()
        || samples.len() > MAX_SAMPLES
        || !samples.len().is_multiple_of(policy.input_color.channels())
    {
        return Err(Error::Samples);
    }
    let pixels = samples.len() / policy.input_color.channels();
    let length = pixels
        .checked_mul(policy.output_color.channels())
        .filter(|length| *length <= MAX_SAMPLES)
        .ok_or(Error::Samples)?;
    validate_profile(input_icc)?;
    if let Some(output) = output_icc {
        validate_profile(output)?;
    }
    let context = ThreadContext::new();
    let input = Profile::new_icc_context(&context, input_icc).map_err(|_| Error::Profile)?;
    if input.color_space() != policy.input_color.signature() {
        return Err(Error::Policy);
    }
    let device_link = &input_icc[12..16] == b"link";
    let flags = if policy.black_point_compensation {
        Flags::BLACKPOINT_COMPENSATION
    } else {
        Flags::default()
    };
    let intent = rendering_intent(policy)?;
    let output = output_icc
        .map(|icc| Profile::new_icc_context(&context, icc).map_err(|_| Error::Profile))
        .transpose()?;
    let transform: Transform<u8, u8, ThreadContext> = if device_link {
        if output.is_some()
            || policy.black_preservation != BlackPreservation::None
            || policy.black_point_compensation
            || input.header_rendering_intent() != intent
            || input.pcs() != policy.output_color.signature()
        {
            return Err(Error::Policy);
        }
        Transform::new_multiprofile_context(
            &context,
            &[&input],
            policy.input_color.format(),
            policy.output_color.format(),
            intent,
            flags,
        )
        .map_err(|_| Error::Transform)?
    } else {
        let output = output.as_ref().ok_or(Error::Policy)?;
        if output_icc.is_some_and(|icc| &icc[12..16] == b"link")
            || output.color_space() != policy.output_color.signature()
        {
            return Err(Error::Policy);
        }
        Transform::new_flags_context(
            &context,
            &input,
            policy.input_color.format(),
            output,
            policy.output_color.format(),
            intent,
            flags,
        )
        .map_err(|_| Error::Transform)?
    };
    let mut converted = vec![0u8; length];
    transform.transform_pixels(samples, &mut converted);
    if let Some(tac) = policy.tac_percent {
        let limit =
            Profile::ink_limiting_context(&context, ColorSpaceSignature::CmykData, f64::from(tac))
                .map_err(|_| Error::Transform)?;
        let limiter: Transform<u8, u8, ThreadContext> = Transform::new_multiprofile_context(
            &context,
            &[&limit],
            PixelFormat::CMYK_8,
            PixelFormat::CMYK_8,
            lcms2::Intent::Perceptual,
            Flags::default(),
        )
        .map_err(|_| Error::Transform)?;
        limiter.transform_in_place(&mut converted);
        // Bound 8-bit rounding while keeping K and CMY ratios. TAC is an explicit policy.
        let maximum = usize::from(tac) * 255 / 100;
        for pixel in converted.as_chunks_mut::<4>().0 {
            let cmy = pixel[..3].iter().map(|v| usize::from(*v)).sum::<usize>();
            let available = maximum - usize::from(pixel[3]);
            if cmy > available {
                for value in &mut pixel[..3] {
                    *value = (usize::from(*value) * available / cmy) as u8;
                }
            }
        }
    }
    let maximum_output_ink_percent = if policy.output_color == Color::Cmyk {
        Some(
            converted
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| p.iter().map(|v| u32::from(*v)).sum::<u32>())
                .max()
                .unwrap_or(0) as f64
                * 100.0
                / 255.0,
        )
    } else {
        None
    };
    let report = Report {
        schema_version: 1,
        engine: "LittleCMS 2",
        input_profile_sha256: hash(input_icc),
        output_profile_sha256: output_icc.map(hash),
        policy_sha256: hash(&serde_json::to_vec(policy).map_err(|_| Error::Policy)?),
        source_sha256: hash(samples),
        output_sha256: hash(&converted),
        pixels,
        device_link,
        maximum_output_ink_percent,
        policy: policy.clone(),
        gamut: gamut::assess(
            samples,
            input_icc,
            output_icc,
            policy.input_color,
            device_link,
        ),
    };
    Ok((converted, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rgb_policy() -> Policy {
        Policy {
            schema_version: 1,
            input_color: Color::Rgb,
            output_color: Color::Rgb,
            intent: Intent::RelativeColorimetric,
            black_point_compensation: true,
            black_preservation: BlackPreservation::None,
            tac_percent: None,
        }
    }
    #[test]
    fn srgb_identity_preserves_samples_and_records_exact_hashes() {
        let profile = Profile::new_srgb().icc().unwrap();
        let data = [255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128];
        let (result, report) = convert(&data, &profile, Some(&profile), &rgb_policy()).unwrap();
        assert_eq!(result, data);
        assert_eq!(report.pixels, 4);
        assert_eq!(report.input_profile_sha256, hash(&profile));
        assert_eq!(report.output_sha256, hash(&result));
        assert!(!report.device_link);
    }
    #[test]
    fn malformed_profiles_and_unsupported_policy_never_transform() {
        let profile = Profile::new_srgb().icc().unwrap();
        let mut malformed = profile.clone();
        malformed[0..4].copy_from_slice(&1u32.to_be_bytes());
        assert!(validate_profile(&malformed).is_err());
        assert!(convert(&[1, 2], &profile, Some(&profile), &rgb_policy()).is_err());
        let mut policy = rgb_policy();
        policy.tac_percent = Some(300);
        assert!(convert(&[1, 2, 3], &profile, Some(&profile), &policy).is_err());
        policy = rgb_policy();
        policy.black_preservation = BlackPreservation::KOnly;
        assert!(convert(&[1, 2, 3], &profile, Some(&profile), &policy).is_err());
    }
    #[test]
    fn cmyk_device_link_limits_ink_and_preserves_pure_k() {
        let profile = Profile::ink_limiting(ColorSpaceSignature::CmykData, 300.0)
            .unwrap()
            .icc()
            .unwrap();
        let policy = Policy {
            schema_version: 1,
            input_color: Color::Cmyk,
            output_color: Color::Cmyk,
            intent: Intent::Perceptual,
            black_point_compensation: false,
            black_preservation: BlackPreservation::None,
            tac_percent: Some(240),
        };
        let (pixels, report) =
            convert(&[255, 255, 255, 255, 0, 0, 0, 200], &profile, None, &policy).unwrap();
        assert!(report.device_link);
        assert_eq!(report.gamut.status, "NOT_ASSESSED");
        assert_eq!(report.gamut.out_of_gamut_pixels, None);
        assert!(report.maximum_output_ink_percent.unwrap() <= 240.0);
        assert_eq!(&pixels[4..], &[0, 0, 0, 200]);
        assert!(pixels[0..4].iter().map(|v| u32::from(*v)).sum::<u32>() <= 612);
    }
}
