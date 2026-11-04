//! Bounded sRGB screen preview with explicit embedded ICC conversion.
use crate::Error;
use std::io::{Read, Seek};
use tiff::{
    ColorType,
    decoder::{Decoder, DecodingResult, Limits},
    tags::Tag,
};

/// A top-down, 24-bit BMP, at most 1024 pixels on either axis.
/// Embedded ICC is converted to sRGB; unprofiled CMYK is rejected.
pub fn bmp(reader: impl Read + Seek) -> Result<Vec<u8>, Error> {
    let mut limits = Limits::default();
    limits.decoding_buffer_size = 64 * 1024 * 1024;
    limits.intermediate_buffer_size = 8 * 1024 * 1024;
    let mut decoder = Decoder::new(reader)?.with_limits(limits);
    let (w, h) = decoder.dimensions()?;
    if w == 0
        || h == 0
        || decoder.more_images()
        || decoder
            .find_tag_unsigned::<u16>(Tag::Orientation)?
            .is_some_and(|v| v != 1)
    {
        return Err(Error::Unsupported);
    }
    let icc = decoder
        .find_tag(Tag::Unknown(34675))?
        .map(|v| v.into_u8_vec())
        .transpose()?;
    let channels = match decoder.colortype()? {
        ColorType::RGB(8) => 3usize,
        ColorType::Gray(8) => 1,
        ColorType::CMYK(8) if icc.is_some() => 4,
        _ => return Err(Error::Unsupported),
    };
    let size = u64::from(w) * u64::from(h) * channels as u64;
    if size > 64 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    let DecodingResult::U8(pixels) = decoder.read_image()? else {
        return Err(Error::Unsupported);
    };
    if pixels.len() as u64 != size {
        return Err(Error::Unsupported);
    }
    let longest = w.max(h).max(1024);
    let width = (u64::from(w) * 1024 / u64::from(longest)).max(1) as usize;
    let height = (u64::from(h) * 1024 / u64::from(longest)).max(1) as usize;
    let mut sampled = Vec::with_capacity(width * height * channels);
    for y in 0..height {
        for x in 0..width {
            let index = ((y as u64 * u64::from(h) / height as u64) * u64::from(w)
                + x as u64 * u64::from(w) / width as u64) as usize
                * channels;
            sampled.extend_from_slice(&pixels[index..index + channels]);
        }
    }
    let rgb = if let Some(icc) = icc {
        let color = match channels {
            1 => rip_color::Color::Gray,
            3 => rip_color::Color::Rgb,
            _ => rip_color::Color::Cmyk,
        };
        rip_color::preview::to_srgb(&sampled, color, &icc).map_err(|_| Error::Unsupported)?
    } else if channels == 1 {
        sampled.into_iter().flat_map(|v| [v; 3]).collect()
    } else {
        sampled
    };
    let stride = (width * 3).div_ceil(4) * 4;
    let mut out = vec![0u8; 54 + stride * height];
    let length = out.len() as u32;
    out[..2].copy_from_slice(b"BM");
    out[2..6].copy_from_slice(&length.to_le_bytes());
    out[10..14].copy_from_slice(&54u32.to_le_bytes());
    out[14..18].copy_from_slice(&40u32.to_le_bytes());
    out[18..22].copy_from_slice(&(width as i32).to_le_bytes());
    out[22..26].copy_from_slice(&(-(height as i32)).to_le_bytes());
    out[26..28].copy_from_slice(&1u16.to_le_bytes());
    out[28..30].copy_from_slice(&24u16.to_le_bytes());
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) * 3;
            let offset = 54 + y * stride + x * 3;
            out[offset..offset + 3].copy_from_slice(&[rgb[index + 2], rgb[index + 1], rgb[index]]);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tiff::encoder::{TiffEncoder, colortype};
    #[test]
    fn preview_keeps_orientation_channels_padding_and_bounds() {
        let mut file = Cursor::new(Vec::new());
        TiffEncoder::new(&mut file)
            .unwrap()
            .write_image::<colortype::RGB8>(1, 2, &[255, 0, 0, 0, 0, 255])
            .unwrap();
        file.set_position(0);
        let bmp = bmp(file).unwrap();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(&bmp[22..26], &(-2i32).to_le_bytes());
        assert_eq!(&bmp[54..], &[0, 0, 255, 0, 255, 0, 0, 0]);
        let mut file = Cursor::new(Vec::new());
        TiffEncoder::new(&mut file)
            .unwrap()
            .write_image::<colortype::Gray8>(2048, 2, &vec![127; 4096])
            .unwrap();
        file.set_position(0);
        let bmp = super::bmp(file).unwrap();
        assert_eq!(&bmp[18..22], &1024i32.to_le_bytes());
        assert_eq!(&bmp[54..57], &[127; 3]);
    }
    #[test]
    fn profiles_rotation_and_multiple_images_require_explicit_processing() {
        for tag in [Tag::Orientation, Tag::Unknown(34675)] {
            let mut file = Cursor::new(Vec::new());
            {
                let mut encoder = TiffEncoder::new(&mut file).unwrap();
                let mut image = encoder.new_image::<colortype::RGB8>(1, 1).unwrap();
                if tag == Tag::Orientation {
                    image.encoder().write_tag(tag, 6u16).unwrap();
                } else {
                    image.encoder().write_tag(tag, &[1u8, 2, 3][..]).unwrap();
                }
                image.write_data(&[0; 3]).unwrap();
            }
            file.set_position(0);
            assert!(bmp(file).is_err());
        }
        let mut file = Cursor::new(Vec::new());
        {
            let mut encoder = TiffEncoder::new(&mut file).unwrap();
            for _ in 0..2 {
                encoder
                    .write_image::<colortype::RGB8>(1, 1, &[0; 3])
                    .unwrap();
            }
        }
        file.set_position(0);
        assert!(bmp(file).is_err());
    }
    #[test]
    fn embedded_rgb_and_cmyk_profiles_convert_to_screen_rgb() {
        let rgb_profile = lcms2::Profile::new_srgb().icc().unwrap();
        let mut file = Cursor::new(Vec::new());
        {
            let mut encoder = TiffEncoder::new(&mut file).unwrap();
            let mut image = encoder.new_image::<colortype::RGB8>(1, 1).unwrap();
            image
                .encoder()
                .write_tag(Tag::Unknown(34675), rgb_profile.as_slice())
                .unwrap();
            image.write_data(&[200, 40, 10]).unwrap();
        }
        file.set_position(0);
        let output = bmp(file).unwrap();
        for (a, b) in output[54..57].iter().zip([10u8, 40, 200]) {
            assert!(a.abs_diff(b) <= 1);
        }
        let mut file = Cursor::new(Vec::new());
        {
            let mut encoder = TiffEncoder::new(&mut file).unwrap();
            let mut image = encoder.new_image::<colortype::CMYK8>(2, 1).unwrap();
            image
                .encoder()
                .write_tag(
                    Tag::Unknown(34675),
                    include_bytes!("../tests/fixtures/synthetic-cmyk.icc").as_slice(),
                )
                .unwrap();
            image.write_data(&[0, 0, 0, 0, 0, 0, 0, 255]).unwrap();
        }
        file.set_position(0);
        let output = bmp(file).unwrap();
        assert!(output[54..57].iter().all(|v| *v > 245));
        assert!(output[57..60].iter().all(|v| *v < 10));
    }
    #[test]
    fn device_cmyk_and_malformed_data_are_rejected() {
        let mut file = Cursor::new(Vec::new());
        TiffEncoder::new(&mut file)
            .unwrap()
            .write_image::<colortype::CMYK8>(1, 1, &[0; 4])
            .unwrap();
        file.set_position(0);
        assert!(bmp(file).is_err());
        assert!(bmp(Cursor::new(b"broken")).is_err());
    }
}
