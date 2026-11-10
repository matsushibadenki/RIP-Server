//! Cached device evidence is scoped to the exact configured destination and security context.
use crate::{ApiError, App, authorize, bad, delivery::Printer, internal};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use rip_printer::{Capabilities, PrinterBackend};
use rip_storage::printer_observation::QueryFailure;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
pub(crate) fn binding(app: &App, printer: &Printer) -> Result<String, ApiError> {
    let security = app
        .printer_security
        .iter()
        .find(|s| s.printer_id == printer.id);
    let configuration = json!({"backend":"native-ipp","printer":printer,"security":security.map(|s|json!({"caPem":s.ca_pem,"bearerToken":s.bearer_token}))});
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&configuration).map_err(internal)?)
    ))
}
fn projection(c: &Capabilities) -> Value {
    let mut reasons = c.reasons.clone();
    reasons.sort();
    reasons.dedup();
    json!({"schemaVersion":1,"profile":"org.jrip.printer-state-snapshot-v1","state":c.state,"acceptingJobs":c.accepting_jobs,"queuedJobCount":c.queued_job_count,
        "conditions":rip_core::printer_conditions::describe(c.state,&reasons),"reasons":reasons,"markerSupplies":c.marker_supplies,"printerSupplies":c.printer_supplies})
}
pub(crate) async fn probe(app: &App, printer: &Printer) -> Result<Capabilities, ApiError> {
    let binding = binding(app, printer)?;
    let serial = app.repo.begin_printer_query(&printer.id, &binding).await?;
    let result = match crate::delivery::backend(app, printer) {
        Ok(backend) => backend.capabilities().await.map_err(|error| {
            (
                if matches!(error, rip_printer::Error::InvalidResponse) {
                    QueryFailure::InvalidResponse
                } else {
                    QueryFailure::Unavailable
                },
                bad("printer_unavailable"),
            )
        }),
        Err(_) => Err((
            QueryFailure::ConfigurationUnavailable,
            bad("invalid_printer"),
        )),
    };
    match result {
        Ok(capabilities) => {
            app.repo
                .finish_printer_query(&printer.id, &binding, serial, Ok(projection(&capabilities)))
                .await?;
            Ok(capabilities)
        }
        Err((failure, error)) => {
            app.repo
                .finish_printer_query(&printer.id, &binding, serial, Err(failure))
                .await?;
            Err(error)
        }
    }
}
pub(crate) fn configured<'a>(app: &'a App, id: &str) -> Result<&'a Printer, ApiError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(bad("invalid_printer"));
    }
    app.printers
        .iter()
        .find(|p| p.id == id)
        .ok_or(bad("printer_not_configured"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    #[serde(default)]
    after: i64,
    #[serde(default = "default_limit")]
    limit: u32,
}
fn default_limit() -> u32 {
    50
}
fn response(value: Value) -> Response {
    ([(header::CACHE_CONTROL, "private, no-store")], Json(value)).into_response()
}
pub async fn list(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(cursor): Query<Cursor>,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    if cursor.after < 0 || !(1..=100).contains(&cursor.limit) {
        return Err(bad("invalid_printer_observation_query"));
    }
    let printer = configured(&app, &id)?;
    let binding = binding(&app, printer)?;
    let mut history = app
        .repo
        .printer_observations(&id, &binding, cursor.after, cursor.limit)
        .await?;
    for event in history["events"].as_array_mut().expect("event array") {
        event["reference"] = json!(format!(
            "/api/v1/printers/{id}/observations/{}",
            event["sequence"]
        ));
    }
    history["liveReference"] = json!(format!("/api/v1/printers/{id}"));
    history["mcpReference"] = json!(format!("jrip://printers/{id}/observations"));
    Ok(response(history))
}
pub async fn event(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, sequence)): Path<(String, i64)>,
) -> Result<Response, ApiError> {
    authorize(&app, &headers, "jobs:read")?;
    if sequence < 1 {
        return Err(bad("invalid_printer_observation_query"));
    }
    let printer = configured(&app, &id)?;
    let binding = binding(&app, printer)?;
    Ok(response(
        app.repo
            .printer_observation_event(&id, &binding, sequence)
            .await?,
    ))
}
