//! Authenticated atomic font installation. Conversion never touches OS fonts.
use super::*;
use axum::body::Bytes;
static INSTALL: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
pub(crate) async fn convert(bytes: Vec<u8>) -> Result<Vec<rip_font::Converted>, ApiError> {
    tokio::task::spawn_blocking(move || rip_font::convert(&bytes))
        .await
        .map_err(internal)?
        .map_err(|e| {
            let code = match e.as_str() {
                "variable_font_requires_static_instance" => "font_variable_unsupported",
                "color_font_unsupported" => "font_color_unsupported",
                "font_embedding_restricted" => "font_embedding_restricted",
                _ => "font_conversion_failed",
            };
            bad(code)
        })
}
pub(crate) async fn install(
    State(app): State<App>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    authorize(&app, &headers, "admin")?;
    if bytes.is_empty() || bytes.len() > rip_font::MAX_BYTES {
        return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "font_size_invalid"));
    }
    let permit = INSTALL
        .try_acquire()
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "font_install_busy"))?;
    let converted = convert(bytes.to_vec()).await?;
    let names: Vec<_> = converted.iter().map(|f| f.font.name.clone()).collect();
    let gs = crate::fonts::engine("ghostscript", "jrip-ghostscript:local").await;
    if gs["status"] != "available" {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "font_runtime_unavailable",
        ));
    }
    if names.iter().any(|name| {
        gs["fonts"]
            .as_array()
            .is_some_and(|a| a.iter().any(|f| f["name"] == *name))
    }) {
        return Err(ApiError(StatusCode::CONFLICT, "font_name_conflict"));
    }
    let expected = headers
        .get("x-jrip-font-snapshot")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let root = app.root.clone();
    let result = tokio::task::spawn_blocking(move || {
        rip_font::store::install_expected(&root, converted, expected.as_deref())
    })
    .await
    .map_err(internal)?
    .map_err(|e| match e.as_str() {
        "font_snapshot_conflict" => ApiError(StatusCode::CONFLICT, "font_snapshot_conflict"),
        "font_name_conflict" => ApiError(StatusCode::CONFLICT, "font_name_conflict"),
        "font_install_busy" => ApiError(StatusCode::CONFLICT, "font_install_busy"),
        _ => bad("font_install_failed"),
    })?;
    drop(permit);
    Ok((
        StatusCode::CREATED,
        Json(
            serde_json::json!({"profile":"org.jrip.font-installation-v1","snapshotSha256":result.digest,"installed":result.fonts.iter().filter(|f|names.contains(&f.name)).collect::<Vec<_>>(),"restartRequired":false}),
        ),
    ))
}
pub(crate) async fn catalog(root: PathBuf) -> Result<Option<rip_font::store::Snapshot>, ApiError> {
    tokio::task::spawn_blocking(move || rip_font::store::current(&root))
        .await
        .map_err(internal)?
        .map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "font_store_integrity_mismatch",
            )
        })
}
