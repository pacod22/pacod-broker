//! The HTTP surfaces: the public API (discovery, registration, reports, MCP) and the admin API.

pub mod admin;
pub mod mcp;
pub mod problem;
pub mod public;
pub mod views;

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;

pub use problem::Problem;

use crate::app::App;
use crate::auth::Caller;

/// The caller of a request, or the problem to answer with.
pub async fn caller(app: &App, headers: &http::HeaderMap) -> Result<Caller, Problem> {
    app.caller(headers).await.map_err(|e| match e {
        crate::auth::AuthError::Invalid => Problem::unauthorized("the token is not valid"),
        crate::auth::AuthError::Unavailable => {
            Problem::unavailable("identity_unavailable", "pacod-id cannot be asked now; try again shortly")
        }
    })
}

/// Requires a permission.
pub fn require(app: &App, caller: &Caller, permission: &str) -> Result<(), Problem> {
    if caller.may(permission, &app.cfg) {
        return Ok(());
    }
    Err(match caller {
        Caller::Anonymous => Problem::unauthorized(&format!("sign in: {permission} needs a pacod-id token")),
        _ => Problem::forbidden(&format!("the caller may not {permission}")),
    })
}

/// Logs every request after the pacod convention (`<METHOD> <path> <status>`, health checks at
/// debug) and counts it by route, never by raw path, so ids do not become metric labels.
pub async fn observe(req: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let route =
        req.extensions().get::<MatchedPath>().map_or_else(|| "unmatched".to_string(), |m| m.as_str().to_string());
    let client_ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    let res = next.run(req).await;
    let status = res.status().as_u16();
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    metrics::counter!("broker_http_requests_total", "route" => route.clone(), "method" => method.to_string(), "status" => status.to_string()).increment(1);
    metrics::histogram!("broker_http_request_seconds", "route" => route.clone()).record(ms / 1000.0);
    let quiet = path == "/healthz" || path == "/readyz" || path == "/metrics";
    if quiet {
        tracing::debug!(target: "pacod_broker::http", method = %method, path = %path, status, duration_ms = ms, client_ip = %client_ip, "{method} {path} {status}");
    } else if status >= 500 {
        tracing::warn!(target: "pacod_broker::http", method = %method, path = %path, status, duration_ms = ms, client_ip = %client_ip, "{method} {path} {status}");
    } else {
        tracing::info!(target: "pacod_broker::http", method = %method, path = %path, status, duration_ms = ms, client_ip = %client_ip, "{method} {path} {status}");
    }
    res
}

pub type Shared = Arc<App>;
