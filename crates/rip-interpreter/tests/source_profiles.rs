#[tokio::test]
#[ignore = "requires Docker and rebuilt jrip-mupdf:local with source ICC analyzer"]
async fn sandbox_analyzes_pdf_and_ps_declarations_without_profile_selection() {
    let root = tempfile::tempdir().unwrap();
    for (bytes, kinds) in [
        (
            include_bytes!("../../../tests/corpus/source-icc.pdf").as_slice(),
            vec!["PDF_OUTPUT_INTENT", "PDF_ICC_BASED"],
        ),
        (
            include_bytes!("../../../tests/corpus/source-icc.ps").as_slice(),
            vec!["PS_DSC_ICC"],
        ),
    ] {
        let source = root.path().join("document");
        std::fs::write(&source, bytes).unwrap();
        let report = rip_interpreter::inspect_source_profiles(
            &source,
            uuid::Uuid::new_v4(),
            "jrip-mupdf:local",
        )
        .await
        .unwrap();
        assert_eq!(report["status"], "DECLARATIONS_INSPECTED");
        assert_eq!(report["automaticSelection"], false);
        assert_eq!(report["outputIntentIsInputProfile"], false);
        let profiles = report["profiles"].as_array().unwrap();
        assert_eq!(profiles.len(), kinds.len());
        for kind in kinds {
            assert!(
                profiles
                    .iter()
                    .any(|p| p["kind"] == kind && p["state"] == "VALIDATED_ENVELOPE")
            );
        }
        assert_eq!(
            report["inspectorRevision"],
            "jrip-source-icc-v1-pypdf-6.21.2"
        );
    }
}
