use rip_font::{convert, hash, store};
const FONT: &[u8] = include_bytes!("data/shaping.ttf");
#[test]
fn normalization_preserves_names_shaping_and_outlines() {
    let converted = convert(FONT).unwrap();
    assert_eq!(converted.len(), 1);
    let c = &converted[0];
    let original = ttf_parser::Face::parse(FONT, 0).unwrap();
    let normalized = ttf_parser::Face::parse(&c.bytes, 0).unwrap();
    assert_eq!(c.font.source_sha256, hash(FONT));
    assert_eq!(c.font.sha256, hash(&c.bytes));
    assert_eq!(original.number_of_glyphs(), normalized.number_of_glyphs());
    for record in original.raw_face().table_records {
        let tag = record.tag.to_bytes();
        if tag == *b"head" || tag == *b"DSIG" {
            continue;
        }
        assert_eq!(
            original.raw_face().table(record.tag),
            normalized.raw_face().table(record.tag),
            "{tag:?}"
        );
    }
    let sum = c.bytes.chunks(4).fold(0u32, |a, b| {
        let mut word = [0; 4];
        word[..b.len()].copy_from_slice(b);
        a.wrapping_add(u32::from_be_bytes(word))
    });
    assert_eq!(sum, 0xb1b0afba);
}
#[test]
fn snapshots_are_atomic_idempotent_and_approval_bound() {
    let root = tempfile::tempdir().unwrap();
    assert!(store::current(root.path()).unwrap().is_none());
    let first = store::install_expected(root.path(), convert(FONT).unwrap(), Some("none")).unwrap();
    assert_eq!(
        store::current(root.path()).unwrap().unwrap().digest,
        first.digest
    );
    let again = store::install(root.path(), convert(FONT).unwrap()).unwrap();
    assert_eq!(again.path, first.path);
    assert_eq!(
        store::install_expected(root.path(), convert(FONT).unwrap(), Some("none")).unwrap_err(),
        "font_snapshot_conflict"
    );
    let mut different = convert(FONT).unwrap();
    different[0].font.sha256 = "0".repeat(64);
    assert_eq!(
        store::install(root.path(), different).unwrap_err(),
        "font_name_conflict"
    );
    assert_eq!(
        hash(&std::fs::read(first.path.join(&first.fonts[0].file)).unwrap()),
        first.fonts[0].sha256
    );
}
#[test]
fn malformed_and_oversized_input_are_rejected() {
    assert!(convert(b"not a font").is_err());
    assert!(convert(&vec![0; rip_font::MAX_BYTES + 1]).is_err());
    assert!(!rip_font::safe_name("Unsafe/name"));
}
#[test]
#[ignore = "requires local Docker RIP images"]
fn registered_programs_load_in_both_engines() {
    for source in [FONT, include_bytes!("data/static.otf").as_slice()] {
        let root = tempfile::tempdir().unwrap();
        let snap = store::install(root.path(), convert(source).unwrap()).unwrap();
        for engine in ["mupdf", "ghostscript"] {
            let result = std::process::Command::new("docker")
                .args([
                    "run",
                    "--rm",
                    "--network=none",
                    "--read-only",
                    "--memory=512m",
                    "--pids-limit=64",
                    "--tmpfs",
                    "/tmp:rw,size=96m",
                    "--tmpfs",
                    "/input:rw,size=32m",
                    "--tmpfs",
                    "/output:rw,size=32m",
                    "--mount",
                    &format!(
                        "type=bind,src={},dst=/jrip-fonts,readonly",
                        snap.path.display()
                    ),
                    "--mount",
                    &format!(
                        "type=bind,src={},dst=/test.py,readonly",
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/engines.py")
                            .display()
                    ),
                    "-e",
                    &format!("JRIP_FONT_MANIFEST_SHA256={}", snap.digest),
                    "--entrypoint",
                    "/opt/jrip-python/bin/python",
                    &format!("jrip-{engine}:local"),
                    "-I",
                    "/test.py",
                    engine,
                ])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{engine}: {} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}
