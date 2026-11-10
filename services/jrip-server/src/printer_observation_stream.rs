//! Coalesced latest-state SSE, not a lossless change-event log or a device probe.
use crate::{ApiError, App, authorize, bad, internal, printer_observation};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{convert::Infallible, time::Duration};
use uuid::Uuid;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Position {
    stream: Uuid,
    serial: i64,
}
impl Position {
    fn parse(text: &str) -> Result<Self, ApiError> {
        let (id, number) = text.split_once(':').ok_or(bad("invalid_event_cursor"))?;
        let stream = Uuid::parse_str(id).map_err(|_| bad("invalid_event_cursor"))?;
        let serial = number
            .parse::<i64>()
            .map_err(|_| bad("invalid_event_cursor"))?;
        if serial < 0 || stream.to_string() != id || serial.to_string() != number {
            return Err(bad("invalid_event_cursor"));
        }
        Ok(Self { stream, serial })
    }
    fn id(self) -> String {
        format!("{}:{}", self.stream, self.serial)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cursor {
    after: Option<String>,
}
struct Connection {
    app: App,
    headers: HeaderMap,
    printer: String,
    binding: String,
    seen: Option<Position>,
    initialized: bool,
    pending: Option<Value>,
    closed: bool,
    expires: tokio::time::Instant,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
fn position(page: &Value) -> Result<Option<Position>, ApiError> {
    if page["latest"].is_null() {
        return Ok(None);
    }
    let stream = page["streamId"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .ok_or_else(|| internal("invalid observation stream"))?;
    let serial = page["latest"]["querySerial"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or_else(|| internal("invalid observation serial"))?;
    Ok(Some(Position { stream, serial }))
}
fn end(event: &str, code: &str) -> Event {
    Event::default()
        .event(event)
        .data(json!({"reasonCode":code}).to_string())
}
pub(crate) async fn connect(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(cursor): Query<Cursor>,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    // Validate URL cursors even if Last-Event-ID takes precedence.
    let from_query = cursor.after.as_deref().map(Position::parse).transpose()?;
    let seen = match headers.get("last-event-id") {
        Some(value) => {
            let value = value.to_str().map_err(|_| bad("invalid_event_cursor"))?;
            if value.is_empty() {
                None
            } else {
                Some(Position::parse(value)?)
            }
        }
        None => from_query,
    };
    let printer = printer_observation::configured(&app, &id)?;
    let binding = printer_observation::binding(&app, printer)?;
    let permit = app.event_streams.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "event_stream_capacity_exhausted",
        )
    })?;
    let page = app.repo.printer_observations(&id, &binding, 0, 1).await?;
    position(&page)?;
    let connection = Connection {
        app,
        headers,
        printer: id,
        binding,
        seen,
        initialized: false,
        pending: Some(page),
        closed: false,
        expires: tokio::time::Instant::now() + Duration::from_secs(900),
        _permit: permit,
    };
    let events = stream::unfold(connection, |mut state| async move {
        if state.closed {
            return None;
        }
        loop {
            if tokio::time::Instant::now() >= state.expires {
                state.closed = true;
                return Some((
                    Ok::<_, Infallible>(end("stream-end", "RECONNECT_REQUIRED")),
                    state,
                ));
            }
            if authorize(&state.app, &state.headers, "jobs:read").is_err() {
                state.closed = true;
                return Some((Ok(end("stream-error", "AUTHORIZATION_EXPIRED")), state));
            }
            if let Some(page) = state.pending.take() {
                let next = match position(&page) {
                    Ok(next) => next,
                    Err(_) => {
                        state.closed = true;
                        return Some((
                            Ok(end("stream-error", "OBSERVATION_STREAM_UNAVAILABLE")),
                            state,
                        ));
                    }
                };
                let changed = if state.initialized {
                    next != state.seen
                } else {
                    state.seen.is_none() || next != state.seen
                };
                let reason = match (state.seen, next, state.initialized) {
                    (_, None, _) => "NO_REPORT",
                    (Some(old), Some(new), _) if old.stream != new.stream => "STREAM_CHANGED",
                    (Some(old), Some(new), _) if old.serial > new.serial => "CURSOR_AHEAD",
                    (_, _, false) => "INITIAL_OR_RESUMED",
                    _ => "UPDATED",
                };
                state.initialized = true;
                if changed {
                    state.seen = next;
                    let data = json!({"schemaVersion":1,"profile":"org.jrip.printer-observation-stream-v1","printerId":state.printer,"streamId":page["streamId"],"latest":page["latest"],"reasonCode":reason,"deliveryMeaning":"coalesced-latest-snapshot","isLive":false,"authorizationGranted":false});
                    let event = Event::default()
                        .event("printer-observation")
                        .id(next.map(Position::id).unwrap_or_default())
                        .data(data.to_string());
                    return Some((Ok(event), state));
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            match state
                .app
                .repo
                .printer_observations(&state.printer, &state.binding, 0, 1)
                .await
            {
                Ok(page) => state.pending = Some(page),
                Err(_) => {
                    state.closed = true;
                    return Some((
                        Ok(end("stream-error", "OBSERVATION_STREAM_UNAVAILABLE")),
                        state,
                    ));
                }
            }
        }
    });
    let mut response = Sse::new(events)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response();
    response
        .headers_mut()
        .insert("cache-control", "private, no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-accel-buffering", "no".parse().unwrap());
    Ok(response)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_is_canonical_and_bounds_checked() {
        let id = Uuid::new_v4();
        assert!(Position::parse(&format!("{id}:0")).is_ok());
        assert!(Position::parse(&format!("{id}:{}", i64::MAX)).is_ok());
        for tail in ["-1", "+1", "01", "9223372036854775808", "1:2", ""] {
            assert!(Position::parse(&format!("{id}:{tail}")).is_err());
        }
        assert!(Position::parse("not-a-uuid:1").is_err());
    }
}
