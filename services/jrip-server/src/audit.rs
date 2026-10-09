use super::*;
use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
};

pub async fn record(State(app): State<App>, request: Request, next: Next) -> Response {
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str())
        .unwrap_or("unmatched");
    if !path.starts_with("/api/") {
        return next.run(request).await;
    }
    let identity = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .and_then(|token| app.access(token))
        .map(|(identity, _)| identity)
        .unwrap_or_else(|| "unauthenticated".into());
    let action = if path.ends_with("/{action}") {
        request
            .uri()
            .path()
            .rsplit('/')
            .next()
            .filter(|action| matches!(*action, "hold" | "release" | "cancel"))
            .unwrap_or("unsupported")
    } else {
        ""
    };
    let operation = format!("{} {} {}", request.method(), path, action)
        .trim_end()
        .to_string();
    let target = request
        .uri()
        .path()
        .split('/')
        .find_map(|part| Uuid::parse_str(part).ok());
    let correlation = Uuid::new_v4();
    let sequence = match app
        .repo
        .begin_audit(correlation, &identity, &operation, target)
        .await
    {
        Ok(sequence) => sequence,
        Err(error) => {
            tracing::error!(%error, "audit admission failed");
            return ApiError(StatusCode::SERVICE_UNAVAILABLE, "audit_unavailable").into_response();
        }
    };
    let mut response = next.run(request).await;
    if let Err(error) = app
        .repo
        .finish_audit(sequence, response.status().as_u16())
        .await
    {
        // The admitted record remains incomplete. Never replay a completed mutation.
        tracing::error!(%error, %correlation, "audit completion failed");
    }
    response.headers_mut().insert(
        "x-jrip-correlation-id",
        correlation.to_string().parse().unwrap(),
    );
    response
}

pub async fn list(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<EventCursor>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&app, &headers, "admin")?;
    if query.after < 0 || !(1..=100).contains(&query.limit) {
        return Err(bad("invalid_event_cursor"));
    }
    Ok(Json(
        app.repo.audit_records(query.after, query.limit).await?,
    ))
}
