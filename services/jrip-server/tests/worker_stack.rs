//! Keep worker futures small enough to compose on ordinary runtime/test stacks.
#[tokio::test]
async fn worker_futures_fit_the_composition_budget() {
    let dir = tempfile::tempdir().unwrap();
    let app = jrip_server::open(dir.path().into(), "test-token".into())
        .await
        .unwrap();
    for (name, bytes) in [
        (
            "pipeline",
            std::mem::size_of_val(&jrip_server::pipeline::tick(&app)),
        ),
        (
            "color",
            std::mem::size_of_val(&jrip_server::color::tick(&app)),
        ),
        (
            "delivery",
            std::mem::size_of_val(&jrip_server::delivery::tick(&app)),
        ),
    ] {
        eprintln!("{name}: {bytes} bytes");
        assert!(bytes <= 64 * 1024, "{name} future exceeds 64 KiB: {bytes}");
    }
}
