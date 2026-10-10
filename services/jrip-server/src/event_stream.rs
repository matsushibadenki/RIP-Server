//! Bounded SSE over durable history; each connection owns only one page of events.
use super::*;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream;
use serde_json::Value;
use std::{collections::VecDeque, convert::Infallible, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cursor {
    after: Option<i64>,
}
#[derive(Clone, Copy)]
enum Kind {
    Pipeline,
    Inspection,
}
impl Kind {
    fn scope(self) -> &'static str {
        match self {
            Self::Pipeline => "jobs:read",
            Self::Inspection => "jobs:submit",
        }
    }
    fn event(self) -> &'static str {
        match self {
            Self::Pipeline => "timeline",
            Self::Inspection => "inspection",
        }
    }
    async fn read(
        self,
        app: &App,
        headers: &HeaderMap,
        id: Uuid,
        after: i64,
    ) -> Result<Value, ApiError> {
        authorize(app, headers, self.scope())?;
        Ok(match self {
            Self::Pipeline => app.repo.unified_pipeline_events(id, after, 100).await?,
            Self::Inspection => {
                app.repo
                    .inspection_events(id, &pipeline::actor(app, headers)?, after, 100)
                    .await?
            }
        })
    }
}
pub(crate) async fn pipeline(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(cursor): Query<Cursor>,
) -> Result<Response, ApiError> {
    connect(app, headers, id, cursor, Kind::Pipeline).await
}
pub(crate) async fn inspection(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(cursor): Query<Cursor>,
) -> Result<Response, ApiError> {
    connect(app, headers, id, cursor, Kind::Inspection).await
}
struct Connection {
    app: App,
    headers: HeaderMap,
    id: Uuid,
    kind: Kind,
    after: i64,
    events: VecDeque<Value>,
    more: bool,
    closed: bool,
    expires: tokio::time::Instant,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
async fn connect(
    app: App,
    headers: HeaderMap,
    id: Uuid,
    cursor: Cursor,
    kind: Kind,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, kind.scope())?;
    // Reconnecting SSE clients retain their original URL; the received ID takes precedence.
    let after = match headers.get("last-event-id") {
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or(bad("invalid_event_cursor"))?,
        None => cursor.after.unwrap_or(0),
    };
    if after < 0 || cursor.after.is_some_and(|after| after < 0) {
        return Err(bad("invalid_event_cursor"));
    }
    let permit = app.event_streams.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "event_stream_capacity_exhausted",
        )
    })?;
    let page = kind.read(&app, &headers, id, after).await?;
    let connection = Connection {
        app,
        headers,
        id,
        kind,
        after,
        events: page["events"]
            .as_array()
            .ok_or_else(|| internal("invalid events"))?
            .iter()
            .cloned()
            .collect(),
        more: page["hasMore"].as_bool().unwrap_or(false),
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
                    Ok::<_, Infallible>(
                        Event::default()
                            .event("stream-end")
                            .data("{\"reasonCode\":\"RECONNECT_REQUIRED\"}"),
                    ),
                    state,
                ));
            }
            if let Some(value) = state.events.pop_front() {
                state.after = value["sequence"]
                    .as_i64()
                    .expect("repository event sequence");
                let event = Event::default()
                    .event(state.kind.event())
                    .id(state.after.to_string())
                    .data(value.to_string());
                return Some((Ok(event), state));
            }
            if !state.more {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            match state
                .kind
                .read(&state.app, &state.headers, state.id, state.after)
                .await
            {
                Ok(page) => {
                    state.events = page["events"]
                        .as_array()
                        .expect("repository event array")
                        .iter()
                        .cloned()
                        .collect();
                    state.more = page["hasMore"].as_bool().unwrap_or(false);
                }
                Err(_) => {
                    state.closed = true;
                    return Some((
                        Ok(Event::default()
                            .event("stream-error")
                            .data("{\"reasonCode\":\"EVENT_STREAM_UNAVAILABLE\"}")),
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
        .insert("x-accel-buffering", "no".parse().unwrap());
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    Ok(response)
}
