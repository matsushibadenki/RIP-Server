//! Bounded, sample-verified TIFF facts. No color conversion or DPI inference.
use crate::Error;
use std::io::{Read, Seek, SeekFrom};
use tiff::{
    decoder::{Decoder, DecodingResult, Limits, ifd::Value},
    tags::Tag,
};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub unit: &'static str,
    pub x: [u64; 2],
    pub y: [u64; 2],
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    #[serde(flatten)]
    pub metadata: rip_color::raster::Metadata,
    pub bits_per_component: u8,
    pub resolution: Option<Resolution>,
    pub samples_verified: bool,
}
fn rational(value: Value, centimeters: bool) -> Result<[u64; 2], Error> {
    let Value::Rational(n, d) = value else {
        return Err(Error::Unsupported);
    };
    if n == 0 || d == 0 {
        return Err(Error::Unsupported);
    }
    let (mut n, mut d) = (u64::from(n), u64::from(d));
    if centimeters {
        n *= 254;
        d *= 100;
    }
    let (mut a, mut b) = (n, d);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    Ok([n / a, d / a])
}
pub fn tiff(mut source: impl Read + Seek) -> Result<Facts, Error> {
    let metadata = rip_color::raster::describe_tiff(&mut source).map_err(|_| Error::Unsupported)?;
    source.seek(SeekFrom::Start(0))?;
    let mut limits = Limits::default();
    limits.decoding_buffer_size = 64 * 1024 * 1024;
    limits.intermediate_buffer_size = 64 * 1024 * 1024;
    let mut decoder = Decoder::new(source)?.with_limits(limits);
    let resolution = match decoder.find_tag_unsigned::<u16>(Tag::ResolutionUnit)? {
        Some(unit @ (2 | 3)) => match (
            decoder.find_tag(Tag::XResolution)?,
            decoder.find_tag(Tag::YResolution)?,
        ) {
            (None, None) => None,
            (Some(x), Some(y)) => Some(Resolution {
                unit: "dots-per-inch",
                x: rational(x, unit == 3)?,
                y: rational(y, unit == 3)?,
            }),
            _ => return Err(Error::Unsupported),
        },
        None | Some(1) => None,
        _ => return Err(Error::Unsupported),
    };
    let channels = match metadata.color_space {
        rip_color::Color::Rgb => 3,
        rip_color::Color::Cmyk => 4,
        rip_color::Color::Gray => 1,
    };
    let expected = u64::from(metadata.width) * u64::from(metadata.height) * channels;
    if expected > 64 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    let DecodingResult::U8(samples) = decoder.read_image()? else {
        return Err(Error::Unsupported);
    };
    if samples.len() as u64 != expected {
        return Err(Error::Unsupported);
    }
    Ok(Facts {
        metadata,
        bits_per_component: 8,
        resolution,
        samples_verified: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tiff::encoder::{Rational, TiffEncoder, colortype};
    fn rgb(unit: u16, n: u32, d: u32) -> Cursor<Vec<u8>> {
        let mut file = Cursor::new(vec![]);
        {
            let mut encoder = TiffEncoder::new(&mut file).unwrap();
            let mut image = encoder.new_image::<colortype::RGB8>(2, 1).unwrap();
            image
                .encoder()
                .write_tag(Tag::ResolutionUnit, unit)
                .unwrap();
            for tag in [Tag::XResolution, Tag::YResolution] {
                image.encoder().write_tag(tag, Rational { n, d }).unwrap();
            }
            image.write_data(&[255, 0, 0, 0, 0, 255]).unwrap();
        }
        file.set_position(0);
        file
    }
    #[test]
    fn descriptor_preserves_geometry_profile_absence_and_exact_resolution_units() {
        let facts = tiff(rgb(2, 300, 1)).unwrap();
        assert_eq!(
            (
                facts.metadata.width,
                facts.metadata.height,
                facts.bits_per_component
            ),
            (2, 1, 8)
        );
        assert_eq!(facts.metadata.profile_state, "MISSING");
        assert!(facts.metadata.profile_sha256.is_none());
        assert_eq!(facts.resolution.unwrap().x, [300, 1]);
        assert!(facts.samples_verified);
        assert_eq!(
            tiff(rgb(3, 100, 1)).unwrap().resolution.unwrap().y,
            [254, 1]
        );
        assert!(tiff(rgb(1, 300, 1)).unwrap().resolution.is_none());
    }
    #[test]
    fn descriptor_rejects_invalid_resolution_and_multiple_images() {
        for (unit, n, d) in [(0, 300, 1), (4, 300, 1), (2, 0, 1), (2, 300, 0)] {
            assert!(tiff(rgb(unit, n, d)).is_err());
        }
        let mut file = Cursor::new(vec![]);
        {
            let mut encoder = TiffEncoder::new(&mut file).unwrap();
            for _ in 0..2 {
                encoder.write_image::<colortype::Gray8>(1, 1, &[0]).unwrap();
            }
        }
        file.set_position(0);
        assert!(tiff(file).is_err());
    }
}
