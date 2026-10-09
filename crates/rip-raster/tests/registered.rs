use rip_raster::{Color, Options, convert, convert_registered};
use sha2::{Digest, Sha256};
use tiff::{
    encoder::{Rational, TiffEncoder, colortype::RGB8},
    tags::{ResolutionUnit, Tag},
};
#[test]
fn registered_profile_and_rational_resolution_must_match() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("page.tiff");
    let mut encoder = TiffEncoder::new(std::fs::File::create(&path).unwrap()).unwrap();
    let mut image = encoder.new_image::<RGB8>(2, 1).unwrap();
    image.resolution(ResolutionUnit::Inch, Rational { n: 300, d: 1 });
    // The adapter only checks the already-registered exact bytes, never parses native ICC.
    let profile = b"test-registered-profile";
    image
        .encoder()
        .write_tag(Tag::Unknown(34675), profile.as_slice())
        .unwrap();
    image.write_data(&[255, 0, 0, 0, 0, 255]).unwrap();
    drop(encoder);
    let options = Options {
        dpi: 300,
        color: Color::DeviceRgb,
    };
    let digest = format!("{:x}", Sha256::digest(profile));
    assert!(convert(&[&path], &options, &mut Vec::new()).is_err());
    let mut bytes = Vec::new();
    convert_registered(&[&path], &options, Some(&digest), &mut bytes).unwrap();
    assert_eq!(rip_raster::inspect(&bytes).unwrap().len(), 1);
    assert!(
        convert_registered(&[&path], &options, Some(&"a".repeat(64)), &mut Vec::new()).is_err()
    );
    assert!(
        convert_registered(
            &[&path],
            &Options {
                dpi: 600,
                color: Color::DeviceRgb
            },
            Some(&digest),
            &mut Vec::new()
        )
        .is_err()
    );
}
