//! The OpenAPI 3.1 document of both APIs, built from the same types the handlers use
//! (schemars, JSON Schema 2020-12), so it cannot drift from the code. `pacod-broker openapi`
//! prints it; the console's typed client is generated from it (`make openapi`), and
//! `make openapi-check` fails when the committed copy is out of date.

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, SchemaGenerator};
use serde_json::{Map, Value, json};

use crate::api::Problem;
use crate::api::admin::{IncidentBody, ProviderRow, RepRow, ReportRow, Session, Status, VerifyBody};
use crate::api::public::{Accepted, ClientReport, FeedbackReport, UsageBatch, VerifyTicket};
use crate::api::views::{DiscoverResponse, ProviderView, ResourceView};
use crate::registry::{Announced, Announcement, Heartbeat, LeaseInfo, Pending, StaticProvider};

struct Doc {
    generator: SchemaGenerator,
    paths: Map<String, Value>,
}

impl Doc {
    fn schema<T: JsonSchema>(&mut self) -> Value {
        serde_json::to_value(self.generator.subschema_for::<T>()).unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments)]
    fn op(
        &mut self,
        method: &str,
        path: &str,
        tag: &str,
        summary: &str,
        body: Option<Value>,
        ok: u16,
        response: Option<Value>,
        params: &[(&str, &str, &str)],
    ) {
        let problem = self.schema::<Problem>();
        let mut o = json!({
            "tags": [tag],
            "summary": summary,
            "operationId": format!("{}_{}", method, path.trim_start_matches('/').replace(['/', '{', '}', '.', '-'], "_")).replace("__", "_").trim_end_matches('_').to_string(),
            "responses": {
                ok.to_string(): match &response {
                    Some(s) => json!({ "description": "OK", "content": { "application/json": { "schema": s } } }),
                    None => json!({ "description": "Done" }),
                },
                "default": { "description": "A problem", "content": { "application/problem+json": { "schema": problem } } }
            }
        });
        if let Some(b) = body {
            o["requestBody"] = json!({ "required": true, "content": { "application/json": { "schema": b } } });
        }
        let mut ps: Vec<Value> = path
            .split('/')
            .filter_map(|seg| seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
            .map(|n| json!({ "name": n, "in": "path", "required": true, "schema": { "type": "string" } }))
            .collect();
        for (name, kind, desc) in params {
            ps.push(json!({ "name": name, "in": "query", "required": false, "description": desc, "schema": { "type": kind } }));
        }
        if !ps.is_empty() {
            o["parameters"] = Value::Array(ps);
        }
        let entry = self.paths.entry(path.to_string()).or_insert_with(|| json!({}));
        entry[method] = o;
    }
}

pub fn document() -> Value {
    let settings = SchemaSettings::draft2020_12().with(|s| s.definitions_path = "/components/schemas".into());
    let mut d = Doc { generator: settings.into_generator(), paths: Map::new() };
    let any = json!({ "type": "object" });

    // The public API.
    let (a, ad) = (d.schema::<Announcement>(), d.schema::<Announced>());
    d.op(
        "put",
        "/v1/announce",
        "providers",
        "Announce a provider's resources under a lease",
        Some(a),
        200,
        Some(ad),
        &[],
    );
    let hb = d.schema::<Heartbeat>();
    d.op(
        "post",
        "/v1/leases/{id}/heartbeat",
        "providers",
        "Keep a lease alive and report live status",
        Some(hb),
        200,
        Some(any.clone()),
        &[("provider", "string", "Open mode: the provider name")],
    );
    d.op(
        "delete",
        "/v1/leases/{id}",
        "providers",
        "End a lease cleanly",
        None,
        204,
        None,
        &[("provider", "string", "Open mode: the provider name")],
    );
    let (q, dr) = (d.schema::<pacod_broker_index::Query>(), d.schema::<DiscoverResponse>());
    d.op("post", "/v1/discover", "discovery", "Discover resources", Some(q), 200, Some(dr.clone()), &[]);
    d.op(
        "get",
        "/v1/resources",
        "discovery",
        "Discover resources with query parameters",
        None,
        200,
        Some(dr),
        &[
            ("kind", "string", ""),
            ("q", "string", "Words"),
            ("tag", "string", "Comma-separated"),
            ("protocol", "string", "Comma-separated"),
            ("capability", "string", "Comma-separated"),
            ("region", "string", "Comma-separated"),
            ("provider", "string", ""),
            ("at", "string", "RFC 3339"),
            ("sort", "string", ""),
            ("limit", "integer", ""),
            ("cursor", "string", ""),
            ("include_unavailable", "boolean", ""),
        ],
    );
    let rv = d.schema::<ResourceView>();
    d.op("get", "/v1/resources/{id}", "discovery", "Read one resource", None, 200, Some(rv.clone()), &[]);
    let pv = d.schema::<ProviderView>();
    d.op("get", "/v1/providers/{id}", "discovery", "Read one provider", None, 200, Some(pv), &[]);
    d.op(
        "get",
        "/v1/watch",
        "discovery",
        "Changes as server-sent events",
        None,
        200,
        None,
        &[("after", "integer", "The generation the caller holds")],
    );
    let (ub, acc) = (d.schema::<UsageBatch>(), d.schema::<Accepted>());
    d.op(
        "post",
        "/v1/reports/usage",
        "reputation",
        "Report how calls went, with tickets",
        Some(ub),
        200,
        Some(acc.clone()),
        &[],
    );
    let fb = d.schema::<FeedbackReport>();
    d.op("post", "/v1/reports/feedback", "reputation", "Rate a resource, with a ticket", Some(fb), 200, Some(acc), &[]);
    let cr = d.schema::<ClientReport>();
    d.op(
        "post",
        "/v1/reports/client",
        "reputation",
        "A provider's report about a client",
        Some(cr),
        200,
        Some(any.clone()),
        &[],
    );
    let vt = d.schema::<VerifyTicket>();
    d.op(
        "post",
        "/v1/tickets/verify",
        "reputation",
        "Who presented a ticket (providers)",
        Some(vt),
        200,
        Some(any.clone()),
        &[],
    );
    d.op("get", "/v1/reputation/me", "reputation", "The caller's own reputation", None, 200, Some(any.clone()), &[]);
    d.op(
        "post",
        "/mcp",
        "mcp",
        "MCP (streamable HTTP): discover_resources, describe_resource, describe_provider",
        Some(any.clone()),
        200,
        Some(any.clone()),
        &[],
    );

    // The admin API.
    let s = d.schema::<Session>();
    d.op("get", "/api/v1/session", "admin", "Who the caller is and what they may do", None, 200, Some(s), &[]);
    let st = d.schema::<Status>();
    d.op("get", "/api/v1/status", "admin", "The broker's state", None, 200, Some(st), &[]);
    d.op(
        "get",
        "/api/v1/config",
        "admin",
        "The effective configuration, without secrets",
        None,
        200,
        Some(any.clone()),
        &[],
    );
    let dr = d.schema::<DiscoverResponse>();
    d.op(
        "get",
        "/api/v1/resources",
        "admin",
        "Every resource",
        None,
        200,
        Some(dr),
        &[
            ("q", "string", ""),
            ("kind", "string", ""),
            ("provider", "string", ""),
            ("sort", "string", ""),
            ("limit", "integer", ""),
            ("cursor", "string", ""),
        ],
    );
    d.op(
        "get",
        "/api/v1/resources/{id}",
        "admin",
        "One resource with its lease and reputation evidence",
        None,
        200,
        Some(any.clone()),
        &[],
    );
    let pr = d.schema::<Vec<ProviderRow>>();
    d.op("get", "/api/v1/providers", "admin", "Every provider", None, 200, Some(pr), &[]);
    d.op(
        "get",
        "/api/v1/providers/{id}",
        "admin",
        "One provider with its leases and reputation evidence",
        None,
        200,
        Some(any.clone()),
        &[],
    );
    for (action, summary) in [
        ("block", "Block a provider and end its leases"),
        ("unblock", "Unblock a provider"),
        ("approve", "Approve a waiting provider"),
    ] {
        d.op("post", &format!("/api/v1/providers/{{id}}/{action}"), "admin", summary, None, 204, None, &[]);
    }
    let vb = d.schema::<VerifyBody>();
    d.op("post", "/api/v1/providers/{id}/verify", "admin", "Vouch for a provider, or stop", Some(vb), 204, None, &[]);
    let ib = d.schema::<IncidentBody>();
    d.op(
        "post",
        "/api/v1/providers/{id}/incident",
        "moderation",
        "Record an upheld incident",
        Some(ib),
        204,
        None,
        &[],
    );
    let pe = d.schema::<Vec<Pending>>();
    d.op("get", "/api/v1/pending", "admin", "Providers waiting for approval", None, 200, Some(pe), &[]);
    let li = d.schema::<Vec<LeaseInfo>>();
    d.op("get", "/api/v1/leases", "admin", "Every lease", None, 200, Some(li), &[]);
    d.op("delete", "/api/v1/leases/{id}", "admin", "End a lease", None, 204, None, &[]);
    let (sl, sp, ad) = (d.schema::<Vec<StaticProvider>>(), d.schema::<StaticProvider>(), d.schema::<Announced>());
    d.op("get", "/api/v1/static", "admin", "Static entries", None, 200, Some(sl), &[]);
    d.op("put", "/api/v1/static", "admin", "Add or replace a provider's static entries", Some(sp), 200, Some(ad), &[]);
    d.op(
        "delete",
        "/api/v1/static/{provider_id}",
        "admin",
        "Remove a provider's static entries",
        None,
        200,
        Some(any.clone()),
        &[],
    );
    let rr = d.schema::<Vec<RepRow>>();
    d.op(
        "get",
        "/api/v1/reputation",
        "moderation",
        "Scores of one side",
        None,
        200,
        Some(rr),
        &[
            ("side", "string", "resource, provider or client"),
            ("sort", "string", "score or confidence"),
            ("limit", "integer", ""),
        ],
    );
    d.op(
        "get",
        "/api/v1/reputation/{side}/{id}",
        "moderation",
        "A score and its evidence",
        None,
        200,
        Some(any.clone()),
        &[],
    );
    d.op(
        "post",
        "/api/v1/reputation/{side}/{id}/{action}",
        "moderation",
        "freeze, unfreeze or reset a score",
        None,
        204,
        None,
        &[],
    );
    let _ = d.schema::<Vec<ReportRow>>();
    d.op("get", "/api/v1/reports", "moderation", "Client reports and notes", None, 200, Some(any.clone()), &[]);
    d.op(
        "post",
        "/api/v1/reports/{id}/reject",
        "moderation",
        "Take back an unfounded client report",
        None,
        204,
        None,
        &[],
    );
    d.op("get", "/api/v1/events", "admin", "Changes as server-sent events", None, 200, None, &[]);

    let schemas = d.generator.take_definitions(true);
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "pacod-broker",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Discovery of AI and compute resources, with prices and availability over time and reputation for providers and clients. The public API is on BROKER_LISTEN, the admin API (/api/v1) on BROKER_ADMIN_LISTEN.",
            "license": { "name": "Apache-2.0", "identifier": "Apache-2.0" }
        },
        "tags": [
            { "name": "providers" }, { "name": "discovery" }, { "name": "reputation" }, { "name": "mcp" }, { "name": "admin" }, { "name": "moderation" }
        ],
        "paths": d.paths,
        "components": {
            "schemas": schemas,
            "securitySchemes": { "bearer": { "type": "http", "scheme": "bearer", "description": "A pacod-id access token or service-account token (pidt_…); in open mode BROKER_ADMIN_TOKEN for the admin API" } }
        },
        "security": [{ "bearer": [] }]
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_document_is_complete_and_committed() {
        let doc = super::document();
        let text = serde_json::to_string_pretty(&doc).unwrap() + "\n";
        assert!(doc["paths"]["/v1/discover"]["post"]["requestBody"].is_object());
        assert!(doc["components"]["schemas"]["Query"].is_object());
        // Every $ref points at a schema the document has.
        let schemas = doc["components"]["schemas"].as_object().unwrap();
        let prefix = "\"$ref\": \"#/components/schemas/";
        for r in text.match_indices(prefix) {
            let rest = &text[r.0 + prefix.len()..];
            let name = &rest[..rest.find('"').unwrap()];
            assert!(schemas.contains_key(name), "missing schema {name}");
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/openapi.json");
        if std::env::var("UPDATE_OPENAPI").is_ok() {
            std::fs::write(path, &text).unwrap();
        }
        let committed = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            committed == text,
            "docs/openapi.json is out of date: UPDATE_OPENAPI=1 cargo test -p pacod-broker openapi"
        );
    }
}
