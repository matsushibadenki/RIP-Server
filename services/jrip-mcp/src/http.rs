//! Stateful Streamable HTTP with token-bound sessions and one notification stream per session.
use crate::{MAX_MESSAGE_BYTES, Session};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::post,
};
use futures_util::stream;
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;
const VERSION: &str = "2025-11-25";
const SESSION_SECONDS: u64 = 900;
struct Entry {
    token: String,
    session: Mutex<Session>,
    stream: Arc<Semaphore>,
    expires: tokio::time::Instant,
    closed: AtomicBool,
}
#[derive(Clone)]
struct Http {
    app: jrip_server::App,
    sessions: Arc<Mutex<BTreeMap<Uuid, Arc<Entry>>>>,
    origins: Arc<Vec<String>>,
}
/// An empty Origin allowlist rejects every request carrying an Origin header.
/// Remote sessions deliberately do not receive a filesystem inbox.
pub fn router(app: jrip_server::App, allowed_origins: Vec<String>) -> Router {
    let state = Http {
        app,
        sessions: Arc::new(Mutex::new(BTreeMap::new())),
        origins: Arc::new(allowed_origins),
    };
    Router::new()
        .route("/mcp", post(receive).get(listen).delete(terminate))
        .layer(DefaultBodyLimit::max(MAX_MESSAGE_BYTES))
        .layer(axum::middleware::from_fn_with_state(state.clone(), gate))
        .with_state(state)
}
type Failure = (StatusCode, &'static str);
async fn gate(
    State(state): State<Http>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    // Validate Origin and authentication before reading any POST body.
    match auth(&state, request.headers()) {
        Ok(_) => next.run(request).await,
        Err(error) => error.into_response(),
    }
}
fn auth(state: &Http, headers: &HeaderMap) -> Result<(String, String, Vec<String>), Failure> {
    for name in [
        "authorization",
        "mcp-session-id",
        "mcp-protocol-version",
        "content-type",
    ] {
        if headers.get_all(name).iter().count() > 1 {
            return Err((StatusCode::BAD_REQUEST, "ambiguous_request_headers"));
        }
    }
    if headers.get_all("origin").iter().count() > 1 {
        return Err((StatusCode::FORBIDDEN, "invalid_origin"));
    }
    if let Some(origin) = headers.get("origin") {
        let valid = origin
            .to_str()
            .ok()
            .is_some_and(|origin| state.origins.iter().any(|allowed| allowed == origin));
        if !valid {
            return Err((StatusCode::FORBIDDEN, "invalid_origin"));
        }
    }
    if headers
        .get("mcp-protocol-version")
        .is_some_and(|value| value.as_bytes() != VERSION.as_bytes())
    {
        return Err((StatusCode::BAD_REQUEST, "unsupported_protocol_version"));
    }
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or((StatusCode::UNAUTHORIZED, "unauthorized"))?;
    let (identity, scopes) = state
        .app
        .access(token)
        .ok_or((StatusCode::UNAUTHORIZED, "unauthorized"))?;
    Ok((token.into(), identity, scopes))
}
fn accepts(headers: &HeaderMap, kind: &str) -> bool {
    headers
        .get_all("accept")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| {
            value.split(',').any(|part| {
                let mut pieces = part.split(';');
                pieces.next().is_some_and(|mime| mime.trim() == kind)
                    && !pieces.any(|parameter| {
                        parameter
                            .trim()
                            .strip_prefix("q=")
                            .is_some_and(|q| q.parse::<f32>().ok() == Some(0.0))
                    })
            })
        })
}
async fn entry(state: &Http, headers: &HeaderMap, token: &str) -> Result<Arc<Entry>, Failure> {
    let id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or((StatusCode::BAD_REQUEST, "session_id_required"))?;
    let mut sessions = state.sessions.lock().await;
    let now = tokio::time::Instant::now();
    sessions.retain(|_, entry| entry.expires > now && !entry.closed.load(Ordering::Acquire));
    let entry = sessions
        .get(&id)
        .filter(|entry| entry.token == token)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "session_not_found"))?;
    Ok(entry)
}
async fn receive(
    State(state): State<Http>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    let (token, identity, scopes) = auth(&state, &headers)?;
    if !accepts(&headers, "application/json") || !accepts(&headers, "text/event-stream") {
        return Err((
            StatusCode::NOT_ACCEPTABLE,
            "accept_json_and_event_stream_required",
        ));
    }
    if !headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim() == "application/json")
        })
    {
        return Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "json_required"));
    }
    let message: Value =
        serde_json::from_slice(&body).map_err(|_| (StatusCode::BAD_REQUEST, "invalid_json"))?;
    if !message.is_object()
        || message["jsonrpc"] != "2.0"
        || !message["method"].is_string()
        || message
            .get("id")
            .is_some_and(|id| !(id.is_string() || id.is_number()))
    {
        return Err((StatusCode::BAD_REQUEST, "invalid_message"));
    }
    if message["method"] == "initialize" && !headers.contains_key("mcp-session-id") {
        if message.get("id").is_none() {
            return Err((StatusCode::BAD_REQUEST, "initialize_request_required"));
        }
        let mut session = Session::new(jrip_server::router(state.app.clone()), token.clone())
            .with_access(identity, scopes, None)
            .with_audit(state.app.repo.clone());
        let reply = session
            .handle(message)
            .await
            .ok_or((StatusCode::BAD_REQUEST, "invalid_initialize"))?;
        if reply.get("error").is_some() {
            return Ok(Json(reply).into_response());
        }
        let mut sessions = state.sessions.lock().await;
        let now = tokio::time::Instant::now();
        sessions.retain(|_, entry| entry.expires > now && !entry.closed.load(Ordering::Acquire));
        if sessions.len() >= 64 {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "session_capacity_exhausted",
            ));
        }
        let id = Uuid::new_v4();
        sessions.insert(
            id,
            Arc::new(Entry {
                token,
                session: Mutex::new(session),
                stream: Arc::new(Semaphore::new(1)),
                expires: now + Duration::from_secs(SESSION_SECONDS),
                closed: AtomicBool::new(false),
            }),
        );
        let mut response = Json(reply).into_response();
        response
            .headers_mut()
            .insert("mcp-session-id", id.to_string().parse().unwrap());
        response
            .headers_mut()
            .insert("cache-control", "no-store".parse().unwrap());
        return Ok(response);
    }
    let entry = entry(&state, &headers, &token).await?;
    let mut session = entry.session.lock().await;
    if entry.closed.load(Ordering::Acquire) || entry.expires <= tokio::time::Instant::now() {
        return Err((StatusCode::NOT_FOUND, "session_not_found"));
    }
    if message.get("id").is_none()
        && !session.ready
        && !(message["method"] == "notifications/initialized" && session.initialized)
    {
        return Err((StatusCode::BAD_REQUEST, "initialize_session_first"));
    }
    match session.handle(message).await {
        Some(reply) => Ok(Json(reply).into_response()),
        None => Ok(StatusCode::ACCEPTED.into_response()),
    }
}
struct Connection {
    state: Http,
    headers: HeaderMap,
    entry: Arc<Entry>,
    pending: VecDeque<Value>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    prime: bool,
}
async fn listen(State(state): State<Http>, headers: HeaderMap) -> Result<Response, Failure> {
    let (token, _, _) = auth(&state, &headers)?;
    if !accepts(&headers, "text/event-stream") {
        return Err((StatusCode::NOT_ACCEPTABLE, "accept_event_stream_required"));
    }
    if headers.contains_key("last-event-id") {
        return Err((
            StatusCode::BAD_REQUEST,
            "notification_event_ids_not_supported",
        ));
    }
    let entry = entry(&state, &headers, &token).await?;
    let permit = entry
        .stream
        .clone()
        .try_acquire_owned()
        .map_err(|_| (StatusCode::CONFLICT, "notification_stream_already_open"))?;
    {
        let mut session = entry.session.lock().await;
        if !session.ready {
            return Err((StatusCode::BAD_REQUEST, "initialize_session_first"));
        }
        // Hints are expendable; reannounce existing history on each new GET stream.
        for cursor in session.subscriptions.values_mut() {
            *cursor = 0;
        }
    }
    let events = stream::unfold(
        Connection {
            state,
            headers,
            entry,
            pending: VecDeque::new(),
            _permit: permit,
            prime: true,
        },
        |mut connection| async move {
            if connection.prime {
                connection.prime = false;
                return Some((
                    Ok::<_, Infallible>(Event::default().retry(Duration::from_secs(1)).data("")),
                    connection,
                ));
            }
            loop {
                if connection.entry.closed.load(Ordering::Acquire)
                    || connection.entry.expires <= tokio::time::Instant::now()
                    || auth(&connection.state, &connection.headers).is_err()
                {
                    return None;
                }
                if let Some(value) = connection.pending.pop_front() {
                    return Some((
                        Ok(Event::default().event("message").data(value.to_string())),
                        connection,
                    ));
                }
                connection.pending = connection
                    .entry
                    .session
                    .lock()
                    .await
                    .notifications()
                    .await
                    .into();
                if connection.pending.is_empty() {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        },
    );
    let mut response = Sse::new(events)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response();
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-accel-buffering", "no".parse().unwrap());
    Ok(response)
}
async fn terminate(State(state): State<Http>, headers: HeaderMap) -> Result<Response, Failure> {
    let (token, _, _) = auth(&state, &headers)?;
    let entry = entry(&state, &headers, &token).await?;
    entry.closed.store(true, Ordering::Release);
    let mut sessions = state.sessions.lock().await;
    sessions.retain(|_, candidate| !Arc::ptr_eq(candidate, &entry));
    Ok(StatusCode::NO_CONTENT.into_response())
}
