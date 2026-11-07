//! Same-origin operator console. No credentials or user data in static assets.
use axum::{
    http::header,
    response::{IntoResponse, Response},
};
fn asset(content: &'static str, mime: &'static str) -> Response {
    ([(header::CONTENT_TYPE, mime),
      (header::CACHE_CONTROL, "no-store"),
      (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
      (header::CONTENT_SECURITY_POLICY, "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src blob:; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"),
      (header::REFERRER_POLICY, "no-referrer")], content).into_response()
}
pub async fn index() -> Response {
    asset(include_str!("../ui/index.html"), "text/html; charset=utf-8")
}
pub async fn script() -> Response {
    asset(
        include_str!("../ui/app.js"),
        "text/javascript; charset=utf-8",
    )
}
pub async fn style() -> Response {
    asset(include_str!("../ui/style.css"), "text/css; charset=utf-8")
}

pub async fn delivery_script() -> Response {
    asset(
        include_str!("../ui/delivery.js"),
        "text/javascript; charset=utf-8",
    )
}

pub async fn printer_status_script() -> Response {
    asset(
        include_str!("../ui/printer-status.js"),
        "text/javascript; charset=utf-8",
    )
}
