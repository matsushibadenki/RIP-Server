use std::process::Command;
#[test]
fn cli_registers_profiles_and_refuses_output_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let icc = root.path().join("srgb.icc");
    let bytes = lcms2::Profile::new_srgb().icc().unwrap();
    std::fs::write(&icc, &bytes).unwrap();
    let registered = Command::new(env!("CARGO_BIN_EXE_rip-color"))
        .arg("register")
        .arg(root.path().join("registry"))
        .arg(&icc)
        .output()
        .unwrap();
    assert!(registered.status.success());
    let record: serde_json::Value = serde_json::from_slice(&registered.stdout).unwrap();
    let described = Command::new(env!("CARGO_BIN_EXE_rip-color"))
        .arg("describe")
        .arg(root.path().join("registry"))
        .arg(record["sha256"].as_str().unwrap())
        .output()
        .unwrap();
    assert!(described.status.success());
    assert_eq!(
        record,
        serde_json::from_slice::<serde_json::Value>(&described.stdout).unwrap()
    );

    let source = root.path().join("source.tiff");
    let mut encoder =
        tiff::encoder::TiffEncoder::new(std::fs::File::create(&source).unwrap()).unwrap();
    encoder
        .write_image::<tiff::encoder::colortype::RGB8>(1, 1, &[255, 0, 0])
        .unwrap();
    drop(encoder);
    let policy = root.path().join("policy.json");
    std::fs::write(&policy, br#"{"schemaVersion":1,"inputColor":"rgb","outputColor":"rgb","intent":"relative_colorimetric","blackPointCompensation":false,"blackPreservation":"none","tacPercent":null}"#).unwrap();
    let destination = root.path().join("result.tiff");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_rip-color"))
            .arg(&source)
            .arg(&icc)
            .arg(&icc)
            .arg(&policy)
            .arg("300")
            .arg(&destination)
            .output()
            .unwrap()
    };
    assert!(run().status.success());
    let output = std::fs::read(&destination).unwrap();
    assert!(!run().status.success());
    assert_eq!(std::fs::read(destination).unwrap(), output);
}
