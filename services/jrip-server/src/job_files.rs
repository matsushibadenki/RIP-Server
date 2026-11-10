//! Local desktop actions. No client-provided filesystem paths or commands.
use super::*;
use axum::{Extension, extract::ConnectInfo};
use std::net::SocketAddr;

pub(crate) async fn capabilities(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    let admin = authorize(&app, &headers, "admin").is_ok();
    Ok(Json(
        serde_json::json!({"finder":cfg!(target_os="macos") && app.local_finder && admin,"delete":admin,"fontInstall":admin,"fontInstallApprovalRequired":app.require_settings_approval,"deleteApprovalRequired":app.require_delete_approval}),
    ))
}
pub(crate) async fn reveal(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
) -> Result<StatusCode, ApiError> {
    authorize(&app, &headers, "admin")?;
    if !cfg!(target_os = "macos")
        || !app.local_finder
        || peer.is_none_or(|Extension(ConnectInfo(addr))| !addr.ip().is_loopback())
    {
        return Err(ApiError(StatusCode::FORBIDDEN, "local_finder_unavailable"));
    }
    let path = raster_path(&app, id).await?;
    let status = tokio::process::Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .status()
        .await
        .map_err(internal)?;
    if !status.success() {
        return Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, "finder_failed"));
    }
    Ok(StatusCode::NO_CONTENT)
}
pub(crate) async fn raster_path(app: &App, id: Uuid) -> Result<PathBuf, ApiError> {
    let job = app.repo.get(id).await?;
    if job.state != JobState::Completed {
        return Err(ApiError(StatusCode::CONFLICT, "raster_unavailable"));
    }
    let artifacts: Vec<rip_core::Artifact> =
        serde_json::from_value(app.repo.artifacts(id).await?).map_err(internal)?;
    let artifact = artifacts
        .first()
        .ok_or(ApiError(StatusCode::NOT_FOUND, "raster_unavailable"))?;
    if artifact.name != format!("page-{:06}.tiff", artifact.page) {
        return Err(bad("invalid_artifact"));
    }
    let directory = app.root.join("jobs").join(id.to_string()).join("raster");
    let file = directory.join(&artifact.name);
    for path in [
        app.root.join("jobs"),
        directory.parent().unwrap().to_path_buf(),
        directory,
        file.clone(),
    ] {
        if tokio::fs::canonicalize(&path)
            .await
            .map_err(|_| ApiError(StatusCode::NOT_FOUND, "raster_unavailable"))?
            != path
        {
            return Err(bad("invalid_artifact"));
        }
    }
    if !tokio::fs::metadata(&file)
        .await
        .map_err(internal)?
        .is_file()
    {
        return Err(bad("invalid_artifact"));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn finder_paths_are_registered_and_cannot_escape_job_directory() {
        let dir = tempfile::tempdir().unwrap();
        let app = crate::open(dir.path().into(), "token".into())
            .await
            .unwrap();
        let id = Uuid::new_v4();
        let mut job:Job=serde_json::from_value(serde_json::json!({"id":id,"manifest":{"name":"test.pdf","format":"application/pdf"},"state":"SPOOLING","source_sha256":"a".repeat(64),"created_at":0,"updated_at":0,"revision":0,"error_code":null,"selected_engine":null,"worker_run_id":null})).unwrap();
        app.repo.insert(&job).await.unwrap();
        let path = app.root.join("jobs").join(id.to_string()).join("raster");
        tokio::fs::create_dir_all(&path).await.unwrap();
        let file = path.join("page-000001.tiff");
        tokio::fs::write(&file, b"raster").await.unwrap();
        assert!(raster_path(&app, id).await.is_err());
        job=app.repo.complete_with_artifacts(job,&[serde_json::from_value(serde_json::json!({"page":1,"name":"page-000001.tiff","mediaType":"image/tiff","bytes":6,"sha256":"a".repeat(64)})).unwrap()]).await.unwrap();
        assert_eq!(raster_path(&app, id).await.unwrap(), file);
        #[cfg(unix)]
        {
            std::fs::remove_file(&file).unwrap();
            let other = app.root.join("elsewhere.tiff");
            std::fs::write(&other, b"other").unwrap();
            std::os::unix::fs::symlink(other, &file).unwrap();
            assert!(raster_path(&app, job.id).await.is_err());
        }
    }
    #[tokio::test]
    async fn reveal_requires_admin_opt_in_and_local_peer_before_launch() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = crate::open(dir.path().into(), "token".into())
            .await
            .unwrap();
        assert!(matches!(
            reveal(
                State(app.clone()),
                HeaderMap::new(),
                Path(Uuid::new_v4()),
                None
            )
            .await,
            Err(ApiError(StatusCode::UNAUTHORIZED, _))
        ));
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer token".parse().unwrap());
        assert!(matches!(
            reveal(
                State(app.clone()),
                headers.clone(),
                Path(Uuid::new_v4()),
                None
            )
            .await,
            Err(ApiError(StatusCode::FORBIDDEN, _))
        ));
        app.local_finder = true;
        let peer = Some(Extension(ConnectInfo("192.0.2.1:1234".parse().unwrap())));
        assert!(matches!(
            reveal(State(app), headers, Path(Uuid::new_v4()), peer).await,
            Err(ApiError(StatusCode::FORBIDDEN, _))
        ));
    }
}
