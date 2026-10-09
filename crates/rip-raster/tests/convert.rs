use rip_raster::{Color, Options, convert_file};
use tiff::encoder::{TiffEncoder, colortype};
fn gray(path: &std::path::Path, width: u32, values: &[u8]) {
    let mut encoder = TiffEncoder::new(std::fs::File::create(path).unwrap()).unwrap();
    encoder
        .write_image::<colortype::Gray8>(width, 1, values)
        .unwrap();
}
#[test]
fn keeps_page_order_black_polarity_and_header_and_does_not_replace_output() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.tiff");
    let second = dir.path().join("second.tiff");
    gray(&first, 2, &[0, 255]);
    gray(&second, 2, &[64, 128]);
    let output = dir.path().join("output.pwg");
    let options = Options {
        dpi: 300,
        color: Color::Black,
    };
    convert_file(&[first, second], &options, &output).unwrap();
    let bytes = std::fs::read(&output).unwrap();
    assert_eq!(&bytes[..4], b"RaS2");
    assert_eq!(&bytes[4..13], b"PwgRaster");
    assert_eq!(
        u32::from_be_bytes(bytes[4 + 452..4 + 456].try_into().unwrap()),
        2
    );
    assert_eq!(&bytes[1800..1804], &[0, 255, 255, 0]);
    assert_eq!(&bytes[3600..3604], &[0, 255, 191, 127]);
    assert!(convert_file(&[dir.path().join("first.tiff")], &options, &output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
}
#[test]
fn rejects_color_mismatch_bad_dpi_and_removes_partial_output() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.tiff");
    gray(&input, 1, &[0]);
    let output = dir.path().join("output.pwg");
    assert!(
        convert_file(
            &[&input],
            &Options {
                dpi: 300,
                color: Color::DeviceRgb
            },
            &output
        )
        .is_err()
    );
    assert!(!output.exists());
    assert!(
        convert_file(
            &[&input],
            &Options {
                dpi: 0,
                color: Color::Black
            },
            &output
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn rgb_and_cmyk_preserve_samples_and_chunk_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    for (channels, color) in [(3, Color::DeviceRgb), (4, Color::DeviceCmyk)] {
        let input = dir.path().join(format!("{channels}.tiff"));
        let pixels: Vec<u8> = (0..129 * channels).map(|i| (i % 256) as u8).collect();
        let mut encoder = TiffEncoder::new(std::fs::File::create(&input).unwrap()).unwrap();
        if channels == 3 {
            encoder
                .write_image::<colortype::RGB8>(129, 1, &pixels)
                .unwrap();
        } else {
            encoder
                .write_image::<colortype::CMYK8>(129, 1, &pixels)
                .unwrap();
        }
        drop(encoder);
        let output = dir.path().join(format!("{channels}.pwg"));
        convert_file(&[input], &Options { dpi: 300, color }, &output).unwrap();
        let bytes = std::fs::read(output).unwrap();
        assert_eq!(bytes[1800], 0);
        assert_eq!(bytes[1801], 129);
        assert_eq!(
            &bytes[1802..1802 + 128 * channels],
            &pixels[..128 * channels]
        );
        assert_eq!(bytes[1802 + 128 * channels], 0);
        assert_eq!(&bytes[1803 + 128 * channels..], &pixels[128 * channels..]);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn independent_cups_reader_decodes_pixels_and_multiple_pages() {
    use std::{ffi::c_void, os::fd::AsRawFd};
    #[link(name = "cups")]
    unsafe extern "C" {
        fn cupsRasterOpen(fd: i32, mode: i32) -> *mut c_void;
        fn cupsRasterReadHeader2(r: *mut c_void, header: *mut u32) -> u32;
        fn cupsRasterReadPixels(r: *mut c_void, bytes: *mut u8, length: u32) -> u32;
        fn cupsRasterClose(r: *mut c_void);
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.tiff");
    gray(&input, 129, &[200; 129]);
    let output = dir.path().join("output.pwg");
    convert_file(
        &[&input, &input],
        &Options {
            dpi: 300,
            color: Color::Black,
        },
        &output,
    )
    .unwrap();
    let file = std::fs::File::open(output).unwrap();
    unsafe {
        let raster = cupsRasterOpen(file.as_raw_fd(), 0);
        assert!(!raster.is_null());
        for _ in 0..2 {
            let mut header = [0u32; 449];
            assert_eq!(cupsRasterReadHeader2(raster, header.as_mut_ptr()), 1);
            assert_eq!(header[372 / 4], 129);
            assert_eq!(header[400 / 4], 3);
            assert_eq!(header[452 / 4], 2);
            let mut pixels = [0u8; 129];
            assert_eq!(cupsRasterReadPixels(raster, pixels.as_mut_ptr(), 129), 129);
            assert_eq!(pixels, [55; 129]);
        }
        cupsRasterClose(raster);
    }
}

#[test]
fn metadata_requiring_color_management_rotation_or_resampling_is_rejected() {
    use tiff::{encoder::Rational, tags::Tag};
    let dir = tempfile::tempdir().unwrap();
    for case in 0..3 {
        let input = dir.path().join(format!("{case}.tiff"));
        let mut encoder = TiffEncoder::new(std::fs::File::create(&input).unwrap()).unwrap();
        let mut image = encoder.new_image::<colortype::Gray8>(1, 1).unwrap();
        match case {
            0 => image.encoder().write_tag(Tag::Orientation, 6u16).unwrap(),
            1 => image
                .encoder()
                .write_tag(Tag::Unknown(34675), &[1u8, 2, 3][..])
                .unwrap(),
            _ => {
                image
                    .encoder()
                    .write_tag(Tag::ResolutionUnit, 2u16)
                    .unwrap();
                image
                    .encoder()
                    .write_tag(Tag::XResolution, Rational { n: 72, d: 1 })
                    .unwrap();
                image
                    .encoder()
                    .write_tag(Tag::YResolution, Rational { n: 72, d: 1 })
                    .unwrap();
            }
        }
        image.write_data(&[0]).unwrap();
        drop(encoder);
        let output = dir.path().join(format!("{case}.pwg"));
        assert!(
            convert_file(
                &[input],
                &Options {
                    dpi: 300,
                    color: Color::Black
                },
                &output
            )
            .is_err()
        );
        assert!(!output.exists());
    }
}
#[test]
fn inspector_rejects_truncation_invalid_resolution_and_reserved_packet() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("one.tiff");
    gray(&input, 2, &[0, 255]);
    let output = dir.path().join("one.pwg");
    convert_file(
        &[input],
        &Options {
            dpi: 300,
            color: Color::Black,
        },
        &output,
    )
    .unwrap();
    let bytes = std::fs::read(output).unwrap();
    let pages = rip_raster::inspect(&bytes).unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].width, 2);
    assert!(rip_raster::inspect(&bytes[..bytes.len() - 1]).is_err());
    let mut bad = bytes.clone();
    bad[4 + 276..4 + 280].copy_from_slice(&0u32.to_be_bytes());
    assert!(rip_raster::inspect(&bad).is_err());
    let mut bad = bytes;
    bad[1801] = 128;
    assert!(rip_raster::inspect(&bad).is_err());
}
