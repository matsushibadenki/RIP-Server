//! Display conversion with explicit source ICC; never infer a CMYK profile.
use crate::{Color, Error, validate_profile};
use lcms2::{Flags, Intent, PixelFormat, Profile, ThreadContext, Transform};

pub fn to_srgb(samples: &[u8], color: Color, icc: &[u8]) -> Result<Vec<u8>, Error> {
    validate_profile(icc)?;
    if &icc[12..16] == b"link"
        || samples.is_empty()
        || !samples.len().is_multiple_of(color.channels())
        || samples.len() > 4 * 1024 * 1024
    {
        return Err(Error::Policy);
    }
    let context = ThreadContext::new();
    let input = Profile::new_icc_context(&context, icc).map_err(|_| Error::Profile)?;
    if input.color_space() != color.signature() {
        return Err(Error::Policy);
    }
    let output = Profile::new_srgb_context(&context);
    let transform: Transform<u8, u8, ThreadContext> = Transform::new_flags_context(
        &context,
        &input,
        color.format(),
        &output,
        PixelFormat::RGB_8,
        Intent::RelativeColorimetric,
        Flags::BLACKPOINT_COMPENSATION,
    )
    .map_err(|_| Error::Transform)?;
    let mut rgb = vec![0; samples.len() / color.channels() * 3];
    transform.transform_pixels(samples, &mut rgb);
    Ok(rgb)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rgb_gamma_is_converted_and_invalid_profile_contracts_fail_closed() {
        let white = lcms2::CIExyY {
            x: 0.3127,
            y: 0.3290,
            Y: 1.0,
        };
        let primaries = lcms2::CIExyYTRIPLE {
            Red: lcms2::CIExyY {
                x: 0.64,
                y: 0.33,
                Y: 1.0,
            },
            Green: lcms2::CIExyY {
                x: 0.30,
                y: 0.60,
                Y: 1.0,
            },
            Blue: lcms2::CIExyY {
                x: 0.15,
                y: 0.06,
                Y: 1.0,
            },
        };
        let gamma = lcms2::ToneCurve::new(1.0);
        let profile = Profile::new_rgb(&white, &primaries, &[&gamma; 3])
            .unwrap()
            .icc()
            .unwrap();
        let rgb = to_srgb(&[128; 3], Color::Rgb, &profile).unwrap();
        assert!(rgb.iter().all(|v| (186..=190).contains(v)));
        assert!(to_srgb(&[0; 4], Color::Cmyk, &profile).is_err());
        assert!(to_srgb(&[0; 3], Color::Rgb, b"invalid").is_err());
        let link = Profile::ink_limiting(lcms2::ColorSpaceSignature::CmykData, 300.0)
            .unwrap()
            .icc()
            .unwrap();
        assert!(to_srgb(&[0; 4], Color::Cmyk, &link).is_err());
        assert!(to_srgb(&[0; 2], Color::Rgb, &profile).is_err());
    }
}
