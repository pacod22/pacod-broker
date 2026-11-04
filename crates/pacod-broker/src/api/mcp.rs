//! Discovery as MCP tools (BRK-09): an agent connected to the broker's `/mcp` endpoint can find
//! and describe resources itself.
//!
//! Streamable HTTP, answered with plain JSON (the transport allows a JSON response instead of an
//! event stream for request–response methods). JSON-RPC 2.0, without an SDK: the server needs
//! `initialize`, `ping`, `tools/list` and `tools/call`, and nothing else.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::{HeaderMap, StatusCode};
use pacod_broker_index::Query;
use serde_json::{Value, json};

use super::views::{ProviderView, ResourceView};
use super::{Shared, caller};
use crate::auth::perm;

const PROTOCOL: &str = "2025-06-18";

pub async fn get() -> Response {
    // No server-initiated stream: the broker sends nothing unasked.
    (StatusCode::METHOD_NOT_ALLOWED, [(http::header::ALLOW, "POST")]).into_response()
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn result(id: Value, r: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": r })
}

fn tools() -> Value {
    json!([
        {
            "name": "discover_resources",
            "title": "Discover resources",
            "description": "Find AI models, MCP servers, agents, tools and compute announced on this broker. Filter by kind, words, capabilities, protocols, regions, maximum prices, availability at an instant or over a span, hardware, software and reputation; sort within a kind by price, latency or reputation. Every result says why it matched and carries a ticket to quote when reporting how a call went.",
            "inputSchema": schemars::schema_for!(Query),
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "describe_resource",
            "title": "Describe a resource",
            "description": "Everything about one resource: its description, interfaces, capabilities, prices and availability now, live status, and reputation.",
            "inputSchema": { "type": "object", "properties": { "id": { "type": "string", "description": "The resource id (res_…)." } }, "required": ["id"], "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "describe_provider",
            "title": "Describe a provider",
            "description": "A provider's profile, its hardware and software inventory where visible, its reputation and the resources it offers.",
            "inputSchema": { "type": "object", "properties": { "id": { "type": "string", "description": "The provider id (prv_…)." } }, "required": ["id"], "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        }
    ])
}

fn text_result(v: &Value, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(v).unwrap_or_default() }],
        "structuredContent": v,
        "isError": is_error,
    })
}

pub async fn handle(State(app): State<Shared>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return Json(error(Value::Null, -32700, "parse error")).into_response(),
    };
    if msg.is_array() {
        return Json(error(Value::Null, -32600, "batches are not supported")).into_response();
    }
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
    let Some(id) = id else {
        // A notification (`notifications/initialized`, …): accepted, no answer.
        return StatusCode::ACCEPTED.into_response();
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let c = match caller(&app, &headers).await {
        Ok(c) => c,
        Err(p) => return p.into_response(),
    };
    if !c.may(perm::DISCOVER, &app.cfg) {
        return super::Problem::unauthorized("discovery needs a pacod-id token").into_response();
    }
    metrics::counter!("broker_mcp_requests_total", "method" => method.clone()).increment(1);
    let out = match method.as_str() {
        "initialize" => result(
            id,
            json!({
                "protocolVersion": PROTOCOL,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "pacod-broker", "title": "pacod-broker", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Use discover_resources to find models, MCP servers, agents, tools and compute; describe_resource and describe_provider for details. Quote a result's ticket when reporting usage at /v1/reports/usage."
            }),
        ),
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match name {
                "discover_resources" => match serde_json::from_value::<Query>(args) {
                    Ok(q) => match super::public::run_query(&app, &c, &q) {
                        Ok(r) => result(id, text_result(&serde_json::to_value(r).unwrap_or_default(), false)),
                        Err(p) => result(id, text_result(&serde_json::to_value(p).unwrap_or_default(), true)),
                    },
                    Err(e) => result(
                        id,
                        text_result(&json!({ "error": format!("the arguments do not fit the schema: {e}") }), true),
                    ),
                },
                "describe_resource" | "describe_provider" => {
                    let Some(rid) = args.get("id").and_then(Value::as_str) else {
                        return Json(error(id, -32602, "`id` is required")).into_response();
                    };
                    let idx = app.registry.index();
                    let viewer = c.viewer(&app.cfg, None);
                    let v = if name == "describe_resource" {
                        idx.get(rid)
                            .filter(|e| {
                                !e.live.is_expired(jiff::Timestamp::now())
                                    && viewer.can_see(&e.record.spec.access.visibility)
                            })
                            .map(|e| serde_json::to_value(ResourceView::of(&app, Some(&c), e)).unwrap_or_default())
                    } else {
                        idx.provider(rid)
                            .map(|p| serde_json::to_value(ProviderView::of(&app, &idx, p, &viewer)).unwrap_or_default())
                    };
                    match v {
                        Some(v) => result(id, text_result(&v, false)),
                        None => result(id, text_result(&json!({ "error": format!("no {rid} visible to you") }), true)),
                    }
                }
                _ => error(id, -32602, &format!("no tool {name:?}")),
            }
        }
        _ => error(id, -32601, &format!("method {method:?} not found")),
    };
    Json(out).into_response()
}
