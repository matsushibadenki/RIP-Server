//! Single-page TIFF adapter with exact geometry/DPI/profile checks.
use super::*;
use std::io::{Read, Seek, Write};
use tiff::{
    ColorType,
    decoder::{Decoder, DecodingResult, Limits},
    encoder::{Rational, TiffEncoder, colortype},
    tags::{ResolutionUnit, Tag},
};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub width: u32,
    pub height: u32,
    pub color_space: Color,
    pub profile_sha256: Option<String>,
    pub profile_state: &'static str,
}
/// Bounded Rust-only metadata inspection, without entering the native ICC parser.
pub fn describe_tiff<R: Read + Seek>(source: R) -> Result<Metadata, Error> {
    let mut limits = Limits::default();
    limits.decoding_buffer_size = MAX_SAMPLES;
    limits.intermediate_buffer_size = 8 * 1024 * 1024;
    let mut decoder = Decoder::new(source)?.with_limits(limits);
    if decoder.more_images()
        || decoder
            .find_tag_unsigned::<u16>(Tag::Orientation)?
            .is_some_and(|orientation| orientation != 1)
    {
        return Err(Error::Samples);
    }
    let color = match decoder.colortype()? {
        ColorType::RGB(8) => Color::Rgb,
        ColorType::CMYK(8) => Color::Cmyk,
        ColorType::Gray(8) => Color::Gray,
        _ => return Err(Error::Samples),
    };
    let (width, height) = decoder.dimensions()?;
    if width == 0
        || height == 0
        || (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(color.channels()))
            .is_none_or(|size| size > MAX_SAMPLES)
    {
        return Err(Error::Samples);
    }
    let embedded = decoder
        .find_tag(Tag::Unknown(34675))?
        .map(|value| value.into_u8_vec())
        .transpose()?;
    if embedded
        .as_ref()
        .is_some_and(|bytes| bytes.len() > MAX_PROFILE)
    {
        return Err(Error::Profile);
    }
    let profile_state = match embedded.as_deref() {
        None => "MISSING",
        Some(bytes) if validate_profile(bytes).is_err() => "INVALID",
        Some(bytes) if &bytes[12..16] == b"link" => "DEVICE_LINK",
        Some(bytes)
            if &bytes[16..20]
                != match color {
                    Color::Rgb => b"RGB ",
                    Color::Cmyk => b"CMYK",
                    Color::Gray => b"GRAY",
                } =>
        {
            "COLOR_SPACE_MISMATCH"
        }
        Some(_) => "VALIDATED_ENVELOPE",
    };
    Ok(Metadata {
        profile_state,
        width,
        height,
        color_space: color,
        profile_sha256: embedded.map(|bytes| hash(&bytes)),
    })
}

pub fn convert_tiff<R: Read + Seek, W: Write + Seek>(
    source: R,
    output: W,
    input_icc: &[u8],
    output_icc: Option<&[u8]>,
    policy: &Policy,
    dpi: u32,
) -> Result<Report, Error> {
    if !(72..=2400).contains(&dpi) {
        return Err(Error::Samples);
    }
    let mut limits = Limits::default();
    limits.decoding_buffer_size = MAX_SAMPLES;
    limits.intermediate_buffer_size = 8 * 1024 * 1024;
    let mut decoder = Decoder::new(source)?.with_limits(limits);
    if decoder.more_images()
        || decoder
            .find_tag_unsigned::<u16>(Tag::Orientation)?
            .is_some_and(|orientation| orientation != 1)
    {
        return Err(Error::Samples);
    }
    let expected = match policy.input_color {
        Color::Rgb => ColorType::RGB(8),
        Color::Cmyk => ColorType::CMYK(8),
        Color::Gray => ColorType::Gray(8),
    };
    if decoder.colortype()? != expected {
        return Err(Error::Samples);
    }
    if let Some(profile) = decoder.find_tag(Tag::Unknown(34675))? {
        let embedded = profile.into_u8_vec()?;
        if embedded.len() > MAX_PROFILE || hash(&embedded) != hash(input_icc) {
            return Err(Error::Profile);
        }
    }
    if let Some(unit) = decoder.find_tag_unsigned::<u16>(Tag::ResolutionUnit)? {
        if !matches!(unit, 1..=3) {
            return Err(Error::Samples);
        }
        if unit != 1 {
            for tag in [Tag::XResolution, Tag::YResolution] {
                let tiff::decoder::ifd::Value::Rational(numerator, denominator) =
                    decoder.get_tag(tag)?
                else {
                    return Err(Error::Samples);
                };
                if denominator == 0 {
                    return Err(Error::Samples);
                }
                let measured = f64::from(numerator) / f64::from(denominator)
                    * if unit == 3 { 2.54 } else { 1.0 };
                if !measured.is_finite() || (measured - f64::from(dpi)).abs() > 0.01 {
                    return Err(Error::Samples);
                }
            }
        }
    }
    let (width, height) = decoder.dimensions()?;
    let size = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(policy.input_color.channels()))
        .ok_or(Error::Samples)?;
    if size == 0 || size > MAX_SAMPLES {
        return Err(Error::Samples);
    }
    let DecodingResult::U8(samples) = decoder.read_image()? else {
        return Err(Error::Samples);
    };
    if samples.len() != size {
        return Err(Error::Samples);
    }
    let (converted, report) = convert(&samples, input_icc, output_icc, policy)?;
    let mut encoder = TiffEncoder::new(output)?;
    macro_rules! encode {
        ($type:ty) => {{
            let mut image = encoder.new_image::<$type>(width, height)?;
            image.resolution(ResolutionUnit::Inch, Rational { n: dpi, d: 1 });
            image.encoder().write_tag(Tag::Orientation, 1u16)?;
            if let Some(profile) = output_icc {
                image.encoder().write_tag(Tag::Unknown(34675), profile)?;
            }
            image.write_data(&converted)?;
        }};
    }
    match policy.output_color {
        Color::Rgb => encode!(colortype::RGB8),
        Color::Cmyk => encode!(colortype::CMYK8),
        Color::Gray => encode!(colortype::Gray8),
    };
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn tiff_conversion_preserves_geometry_and_embeds_output_profile() {
        let mut input = Cursor::new(Vec::new());
        TiffEncoder::new(&mut input)
            .unwrap()
            .write_image::<colortype::RGB8>(2, 1, &[255, 0, 0, 0, 0, 255])
            .unwrap();
        input.set_position(0);
        let mut output = Cursor::new(Vec::new());
        let icc = Profile::new_srgb().icc().unwrap();
        let policy = Policy {
            schema_version: 1,
            input_color: Color::Rgb,
            output_color: Color::Rgb,
            intent: Intent::RelativeColorimetric,
            black_point_compensation: false,
            black_preservation: BlackPreservation::None,
            tac_percent: None,
        };
        let report = convert_tiff(input, &mut output, &icc, Some(&icc), &policy, 300).unwrap();
        assert_eq!(report.pixels, 2);
        let encoded = output.get_ref().clone();
        let mut second = Cursor::new(Vec::new());
        assert!(
            convert_tiff(
                Cursor::new(encoded.clone()),
                &mut second,
                &icc,
                Some(&icc),
                &policy,
                300
            )
            .is_ok()
        );
        assert!(
            convert_tiff(
                Cursor::new(encoded),
                Cursor::new(Vec::new()),
                &icc,
                Some(&icc),
                &policy,
                600
            )
            .is_err()
        );
        output.set_position(0);
        let mut decoded = Decoder::new(output).unwrap();
        assert_eq!(decoded.dimensions().unwrap(), (2, 1));
        assert_eq!(
            decoded
                .get_tag(Tag::Unknown(34675))
                .unwrap()
                .into_u8_vec()
                .unwrap(),
            icc
        );
        let DecodingResult::U8(samples) = decoded.read_image().unwrap() else {
            panic!("unexpected output layout")
        };
        assert_eq!(samples, vec![255, 0, 0, 0, 0, 255]);
    }
}

#[cfg(test)]
mod profile_evidence_tests {
    use super::*;
    #[test]
    fn embedded_profiles_are_classified_without_guessing_or_native_parsing() {
        let rgb = Profile::new_srgb().icc().unwrap();
        let mut mismatch = rgb.clone();
        mismatch[16..20].copy_from_slice(b"GRAY");
        let mut link = rgb.clone();
        link[12..16].copy_from_slice(b"link");
        link[20..24].copy_from_slice(b"RGB ");
        for (profile, state) in [
            (None, "MISSING"),
            (Some(rgb), "VALIDATED_ENVELOPE"),
            (Some(vec![0; 16]), "INVALID"),
            (Some(mismatch), "COLOR_SPACE_MISMATCH"),
            (Some(link), "DEVICE_LINK"),
        ] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            {
                let mut encoder = TiffEncoder::new(&mut bytes).unwrap();
                let mut image = encoder.new_image::<colortype::RGB8>(1, 1).unwrap();
                if let Some(profile) = &profile {
                    image
                        .encoder()
                        .write_tag(Tag::Unknown(34675), profile.as_slice())
                        .unwrap();
                }
                image.write_data(&[0, 0, 0]).unwrap();
            }
            bytes.set_position(0);
            let metadata = describe_tiff(bytes).unwrap();
            assert_eq!(metadata.profile_state, state);
            assert_eq!(metadata.profile_sha256, profile.map(|bytes| hash(&bytes)));
        }
    }
}
