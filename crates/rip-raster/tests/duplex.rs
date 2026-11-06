use rip_raster::{Color, DuplexBinding, Options};
use tiff::encoder::{TiffEncoder, colortype};

fn fixtures(
    root: &std::path::Path,
    channels: usize,
    count: usize,
) -> (Vec<std::path::PathBuf>, Vec<Vec<u8>>) {
    fixtures_grid(root, channels, count, 2, 2)
}
fn fixtures_grid(
    root: &std::path::Path,
    channels: usize,
    count: usize,
    width: u32,
    height: u32,
) -> (Vec<std::path::PathBuf>, Vec<Vec<u8>>) {
    let mut paths = Vec::new();
    let mut samples = Vec::new();
    for page in 0..count {
        let path = root.join(format!("{channels}-{page}.tiff"));
        let pixels: Vec<u8> = (0..width as usize * height as usize * channels)
            .map(|i| ((i * 31 + page * 51) % 256) as u8)
            .collect();
        let mut encoder = TiffEncoder::new(std::fs::File::create(&path).unwrap()).unwrap();
        match channels {
            1 => encoder
                .write_image::<colortype::Gray8>(width, height, &pixels)
                .unwrap(),
            3 => encoder
                .write_image::<colortype::RGB8>(width, height, &pixels)
                .unwrap(),
            4 => encoder
                .write_image::<colortype::CMYK8>(width, height, &pixels)
                .unwrap(),
            _ => unreachable!(),
        }
        paths.push(path);
        samples.push(if channels == 1 {
            pixels.into_iter().map(|v| 255 - v).collect()
        } else {
            pixels
        });
    }
    (paths, samples)
}
#[cfg(target_os = "macos")]
fn color(channels: usize) -> Color {
    match channels {
        1 => Color::Black,
        3 => Color::DeviceRgb,
        _ => Color::DeviceCmyk,
    }
}

#[test]
fn duplex_is_explicit_bounded_and_cannot_bypass_the_simplex_inspector() {
    let dir = tempfile::tempdir().unwrap();
    let (pages, _) = fixtures(dir.path(), 3, 3);
    for binding in [DuplexBinding::LongEdge, DuplexBinding::ShortEdge] {
        let mut bytes = Vec::new();
        rip_raster::convert_normal_duplex(
            &pages,
            &Options {
                dpi: 300,
                color: Color::DeviceRgb,
            },
            None,
            binding,
            &mut bytes,
        )
        .unwrap();
        assert!(rip_raster::inspect(&bytes).is_err()); // Existing outbound path stays simplex-only.
        let headers = rip_raster::inspect_normal_duplex(&bytes, binding).unwrap();
        assert_eq!(headers.len(), 3); // Odd jobs are not padded or duplicated.
        let other = if binding == DuplexBinding::LongEdge {
            DuplexBinding::ShortEdge
        } else {
            DuplexBinding::LongEdge
        };
        assert!(rip_raster::inspect_normal_duplex(&bytes, other).is_err());
        for (field, value) in [
            (272, 0u32),
            (272, 2),
            (368, 2),
            (456, u32::MAX),
            (460, u32::MAX),
        ] {
            let mut bad = bytes.clone();
            let offset = headers[1].offset + field;
            bad[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
            assert!(
                rip_raster::inspect_normal_duplex(&bad, binding).is_err(),
                "field {field}"
            );
        }
        assert!(rip_raster::inspect_normal_duplex(&bytes[..bytes.len() - 1], binding).is_err());
    }
    let mut simplex = Vec::new();
    rip_raster::convert(
        &pages,
        &Options {
            dpi: 300,
            color: Color::DeviceRgb,
        },
        &mut simplex,
    )
    .unwrap();
    assert!(rip_raster::inspect_normal_duplex(&simplex, DuplexBinding::LongEdge).is_err());
}

#[test]
fn duplex_file_and_cli_preserve_atomic_publication_and_reject_bad_options() {
    let dir = tempfile::tempdir().unwrap();
    let (pages, _) = fixtures(dir.path(), 1, 1);
    let output = dir.path().join("normal.pwg");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_rip-raster"))
        .arg(&output)
        .args(["300", "black", "--duplex-normal=short-edge"])
        .arg(&pages[0])
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(&output).unwrap();
    assert_eq!(
        rip_raster::inspect_normal_duplex(&bytes, DuplexBinding::ShortEdge)
            .unwrap()
            .len(),
        1
    );
    assert!(
        rip_raster::convert_normal_duplex_file(
            &pages,
            &Options {
                dpi: 300,
                color: Color::Black
            },
            None,
            DuplexBinding::LongEdge,
            &output
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
    for arguments in [
        vec!["--duplex-normal=rotated"],
        vec!["--duplex-normal=long-edge"],
    ] {
        let target = dir.path().join("bad.pwg");
        let status = std::process::Command::new(env!("CARGO_BIN_EXE_rip-raster"))
            .arg(&target)
            .args(["300", "black"])
            .args(arguments)
            .status()
            .unwrap();
        assert!(!status.success());
        assert!(!target.exists());
    }
    let bad = dir.path().join("profile-mismatch.pwg");
    assert!(
        rip_raster::convert_normal_duplex_file(
            &pages,
            &Options {
                dpi: 300,
                color: Color::Black
            },
            Some(&"a".repeat(64)),
            DuplexBinding::LongEdge,
            &bad
        )
        .is_err()
    );
    assert!(!bad.exists());
}

#[cfg(target_os = "macos")]
#[test]
fn independent_cups_checks_all_sheet_back_transforms_and_every_pixel() {
    use rip_raster::{DuplexLayout, SheetBack};
    use std::{ffi::c_void, os::fd::AsRawFd};
    #[link(name = "cups")]
    unsafe extern "C" {
        fn cupsRasterOpen(fd: i32, mode: i32) -> *mut c_void;
        fn cupsRasterReadHeader2(r: *mut c_void, header: *mut u32) -> u32;
        fn cupsRasterReadPixels(r: *mut c_void, bytes: *mut u8, length: u32) -> u32;
        fn cupsRasterClose(r: *mut c_void);
    }
    for channels in [1, 3, 4] {
        for count in [3, 4] {
            for (width, height) in [(2, 2), (3, 3), (3, 2)] {
                for binding in [DuplexBinding::LongEdge, DuplexBinding::ShortEdge] {
                    for sheet_back in [
                        SheetBack::Normal,
                        SheetBack::Rotated,
                        SheetBack::Flipped,
                        SheetBack::ManualTumble,
                    ] {
                        let dir = tempfile::tempdir().unwrap();
                        let (pages, expected) =
                            fixtures_grid(dir.path(), channels, count, width, height);
                        let output = dir.path().join("duplex.pwg");
                        let layout = DuplexLayout {
                            binding,
                            sheet_back,
                        };
                        rip_raster::convert_duplex_file(
                            &pages,
                            &Options {
                                dpi: 300,
                                color: color(channels),
                            },
                            None,
                            layout,
                            &output,
                        )
                        .unwrap();
                        assert_eq!(
                            rip_raster::inspect_duplex(&std::fs::read(&output).unwrap(), layout)
                                .unwrap()
                                .len(),
                            count
                        );
                        let file = std::fs::File::open(output).unwrap();
                        unsafe {
                            let raster = cupsRasterOpen(file.as_raw_fd(), 0);
                            assert!(!raster.is_null());
                            for (page, source) in expected.iter().enumerate() {
                                // Independent Table 9 expectations; only even physical pages are backs.
                                let (reverse_x, reverse_y) = if page % 2 == 0 {
                                    (false, false)
                                } else {
                                    match (binding, sheet_back) {
                                        (DuplexBinding::LongEdge, SheetBack::Flipped) => {
                                            (false, true)
                                        }
                                        (DuplexBinding::ShortEdge, SheetBack::Flipped) => {
                                            (true, false)
                                        }
                                        (DuplexBinding::LongEdge, SheetBack::Rotated)
                                        | (DuplexBinding::ShortEdge, SheetBack::ManualTumble) => {
                                            (true, true)
                                        }
                                        _ => (false, false),
                                    }
                                };
                                let mut header = [0u32; 449];
                                assert_eq!(cupsRasterReadHeader2(raster, header.as_mut_ptr()), 1);
                                assert_eq!(header[272 / 4], 1);
                                assert_eq!(
                                    header[368 / 4],
                                    u32::from(binding == DuplexBinding::ShortEdge)
                                );
                                assert_eq!(header[456 / 4], if reverse_x { u32::MAX } else { 1 });
                                assert_eq!(header[460 / 4], if reverse_y { u32::MAX } else { 1 });
                                assert_eq!(header[372 / 4], width);
                                assert_eq!(header[376 / 4], height);
                                assert_eq!(header[452 / 4], count as u32);
                                let mut pixels = vec![0u8; source.len()];
                                assert_eq!(
                                    cupsRasterReadPixels(
                                        raster,
                                        pixels.as_mut_ptr(),
                                        pixels.len() as u32
                                    ),
                                    pixels.len() as u32
                                );
                                for y in 0..height as usize {
                                    for x in 0..width as usize {
                                        for c in 0..channels {
                                            let sx =
                                                if reverse_x { width as usize - 1 - x } else { x };
                                            let sy = if reverse_y {
                                                height as usize - 1 - y
                                            } else {
                                                y
                                            };
                                            assert_eq!(
                                                pixels[(y * width as usize + x) * channels + c],
                                                source[(sy * width as usize + sx) * channels + c],
                                                "{layout:?} page {page} ({x},{y}) channel {c}"
                                            );
                                        }
                                    }
                                }
                            }
                            let mut header = [0u32; 449];
                            assert_eq!(cupsRasterReadHeader2(raster, header.as_mut_ptr()), 0);
                            cupsRasterClose(raster);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn registered_icc_is_required_and_preserved_for_normal_duplex() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("profiled.tiff");
    let profile = lcms2::Profile::new_srgb().icc().unwrap();
    {
        let mut encoder = TiffEncoder::new(std::fs::File::create(&source).unwrap()).unwrap();
        let mut image = encoder.new_image::<colortype::RGB8>(2, 2).unwrap();
        image
            .encoder()
            .write_tag(tiff::tags::Tag::Unknown(34675), profile.as_slice())
            .unwrap();
        image
            .write_data(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255])
            .unwrap();
    }
    let digest = format!("{:x}", Sha256::digest(&profile));
    let options = Options {
        dpi: 300,
        color: Color::DeviceRgb,
    };
    let mut bytes = Vec::new();
    rip_raster::convert_normal_duplex(
        &[&source],
        &options,
        Some(&digest),
        DuplexBinding::LongEdge,
        &mut bytes,
    )
    .unwrap();
    assert_eq!(
        rip_raster::inspect_normal_duplex(&bytes, DuplexBinding::LongEdge)
            .unwrap()
            .len(),
        1
    );
    for expected in [None, Some("wrong")] {
        assert!(
            rip_raster::convert_normal_duplex(
                &[&source],
                &options,
                expected,
                DuplexBinding::LongEdge,
                &mut Vec::new()
            )
            .is_err()
        );
    }
}

#[test]
fn all_back_modes_validate_page_parity_and_cli_requires_explicit_coordinate_mode() {
    use rip_raster::{DuplexLayout, SheetBack};
    let dir = tempfile::tempdir().unwrap();
    let (pages, _) = fixtures(dir.path(), 3, 3);
    for binding in [DuplexBinding::LongEdge, DuplexBinding::ShortEdge] {
        for sheet_back in [
            SheetBack::Normal,
            SheetBack::Rotated,
            SheetBack::Flipped,
            SheetBack::ManualTumble,
        ] {
            let layout = DuplexLayout {
                binding,
                sheet_back,
            };
            let mut bytes = Vec::new();
            rip_raster::convert_duplex(
                &pages,
                &Options {
                    dpi: 300,
                    color: Color::DeviceRgb,
                },
                None,
                layout,
                &mut bytes,
            )
            .unwrap();
            let parsed = rip_raster::inspect_duplex(&bytes, layout).unwrap();
            assert_eq!(parsed.len(), 3);
            let mut bad = bytes.clone();
            bad[parsed[0].offset + 456..parsed[0].offset + 460]
                .copy_from_slice(&u32::MAX.to_be_bytes());
            assert!(rip_raster::inspect_duplex(&bad, layout).is_err());
            assert!(rip_raster::inspect_duplex(&bytes[..bytes.len() - 1], layout).is_err());
        }
    }
    let output = dir.path().join("rotated.pwg");
    assert!(
        std::process::Command::new(env!("CARGO_BIN_EXE_rip-raster"))
            .arg(&output)
            .args(["300", "rgb", "--duplex=long-edge", "--sheet-back=rotated"])
            .args(&pages)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        rip_raster::inspect_duplex(
            &std::fs::read(output).unwrap(),
            DuplexLayout {
                binding: DuplexBinding::LongEdge,
                sheet_back: SheetBack::Rotated
            }
        )
        .unwrap()
        .len(),
        3
    );
    for flag in [None, Some("--sheet-back=unknown")] {
        let output = dir.path().join("invalid.pwg");
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_rip-raster"));
        command
            .arg(&output)
            .args(["300", "rgb", "--duplex=long-edge"]);
        if let Some(flag) = flag {
            command.arg(flag);
        }
        command.args(&pages);
        assert!(!command.status().unwrap().success());
        assert!(!output.exists());
    }
}
