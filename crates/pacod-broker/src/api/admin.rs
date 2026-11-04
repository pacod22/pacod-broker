//! The admin API (docs/api.md §5): what the console docks to.
//!
//! It is served on its own listener (`BROKER_ADMIN_LISTEN`), off by default: a broker runs
//! without it and without the console. In the full stack the listener binds an alias on the
//! internal network `pacod-admin-net`, so only servers reach it. In `open` mode it takes
//! `BROKER_ADMIN_TOKEN` when one is set; in `oidc` mode a pacod-id token with the broker's admin
//! permissions.

use std::convert::Infallible;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query as QueryParams, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{delete, get, post};
use futures_util::{Stream, StreamExt};
use http::{HeaderMap, StatusCode};
use jiff::Timestamp;
use pacod_broker_index::{Ctx, Query, Viewer};
use pacod_broker_reputation::{Evidence, Side};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio_stream::wrappers::BroadcastStream;

use super::problem::Body;
use super::public::NOTES;
use super::views::{DiscoverResponse, ProviderSummary, ProviderView, ResourceView};
use super::{Problem, Shared, caller};
use crate::auth::{Caller, perm};
use crate::registry::{Announcement, LeaseInfo, Pending, StaticProvider};

pub fn router(app: Shared) -> Router {
    let api = Router::new()
        .route("/session", get(session))
        .route("/status", get(status))
        .route("/config", get(config))
        .route("/resources", get(resources))
        .route("/resources/{id}", get(resource))
        .route("/resources/{id}/timeline", get(timeline))
        .route("/providers", get(providers))
        .route("/providers/{id}", get(provider))
        .route("/providers/{id}/block", post(block))
        .route("/providers/{id}/unblock", post(unblock))
        .route("/providers/{id}/approve", post(approve))
        .route("/providers/{id}/verify", post(verify))
        .route("/providers/{id}/incident", post(incident))
        .route("/pending", get(pending))
        .route("/leases", get(leases))
        .route("/leases/{id}", delete(evict))
        .route("/static", get(static_list).put(static_put))
        .route("/static/{provider_id}", delete(static_delete))
        .route("/reputation", get(reputation_list))
        .route("/reputation/{side}/{id}", get(reputation_one))
        .route("/reputation/{side}/{id}/{action}", post(reputation_action))
        .route("/reports", get(reports))
        .route("/reports/{id}/reject", post(reject_report))
        .route("/events", get(events))
        .route("/openapi.json", get(openapi));
    Router::new().nest("/api/v1", api).route("/healthz", get(|| async { "ok" })).with_state(app)
}

/// The caller of an admin request, with the permission checked.
async fn admin(app: &Shared, headers: &HeaderMap, permission: &str) -> Result<Caller, Problem> {
    match &app.identity {
        None => {
            if let Some(token) = &app.cfg.admin_token {
                let given = crate::auth::bearer(headers).unwrap_or_default();
                if !bool::from(given.as_bytes().ct_eq(token.expose().as_bytes())) {
                    return Err(Problem::unauthorized("the admin API needs BROKER_ADMIN_TOKEN as a bearer token"));
                }
            }
            Ok(Caller::Open { client: None })
        }
        Some(id) => {
            if !id.ready() {
                return Err(Problem::unavailable(
                    "identity_unavailable",
                    "the broker's copy of the roles is not current; try again shortly",
                ));
            }
            let c = caller(app, headers).await?;
            match &c {
                Caller::Principal(p) if p.has(permission) => Ok(c),
                Caller::Principal(_) => Err(Problem::forbidden(&format!("needs {permission}"))),
                _ => Err(Problem::unauthorized("sign in with pacod-id")),
            }
        }
    }
}

#[derive(Serialize, JsonSchema)]
pub struct Session {
    pub mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organisation_id: Option<String>,
    pub permissions: Vec<String>,
    pub broker_id: String,
    pub version: &'static str,
}

/// Who the console's user is and what they may do.
async fn session(State(app): State<Shared>, headers: HeaderMap) -> Result<Json<Session>, Problem> {
    let c = admin(&app, &headers, perm::ADMIN_READ).await?;
    let all = crate::identity::PERMISSIONS.iter().map(|p| p.to_string()).collect();
    Ok(Json(match c {
        Caller::Principal(p) => Session {
            mode: "oidc",
            sub: Some(p.sub),
            name: p.name,
            organisation_id: p.organisation_id,
            permissions: p.permissions,
            broker_id: app.cfg.id.clone(),
            version: env!("CARGO_PKG_VERSION"),
        },
        _ => Session {
            mode: "open",
            sub: None,
            name: None,
            organisation_id: None,
            permissions: all,
            broker_id: app.cfg.id.clone(),
            version: env!("CARGO_PKG_VERSION"),
        },
    }))
}

#[derive(Serialize, JsonSchema)]
pub struct Status {
    pub broker_id: String,
    pub version: &'static str,
    pub started_at: Timestamp,
    pub uptime_s: i64,
    pub generation: u64,
    pub index_built_at: Option<Timestamp>,
    pub resources: usize,
    pub providers: usize,
    pub leases: usize,
    pub pending: usize,
    pub blocked: usize,
    pub by_kind: std::collections::BTreeMap<String, usize>,
    pub by_status: std::collections::BTreeMap<String, usize>,
    pub reputation_computed_at: Option<Timestamp>,
    pub clients_rated: usize,
    pub auth: &'static str,
    pub identity_ready: Option<bool>,
    pub state_dir: bool,
}

async fn status(State(app): State<Shared>, headers: HeaderMap) -> Result<Json<Status>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    let idx = app.registry.index();
    let now = Timestamp::now();
    let mut by_status = std::collections::BTreeMap::new();
    for e in idx.entries().filter(|e| !e.live.is_expired(now)) {
        let s =
            serde_json::to_value(e.live.status()).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
        *by_status.entry(s).or_insert(0) += 1;
    }
    let scores = app.reputation.scores();
    Ok(Json(Status {
        broker_id: app.cfg.id.clone(),
        version: env!("CARGO_PKG_VERSION"),
        started_at: app.started_at,
        uptime_s: now.duration_since(app.started_at).as_secs(),
        generation: idx.generation,
        index_built_at: idx.built_at,
        resources: idx.len(),
        providers: idx.providers().count(),
        leases: app.registry.leases().len(),
        pending: app.registry.pending().len(),
        blocked: app.registry.blocked().len(),
        by_kind: idx.kinds().map(|(k, n)| (k.to_string(), n)).collect(),
        by_status,
        reputation_computed_at: scores.computed_at,
        clients_rated: scores.clients.len(),
        auth: if app.identity.is_some() { "oidc" } else { "open" },
        identity_ready: app.identity.as_ref().map(|i| i.ready()),
        state_dir: app.state.is_some(),
    }))
}

async fn config(State(app): State<Shared>, headers: HeaderMap) -> Result<Json<serde_json::Value>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    Ok(Json(app.cfg.summary()))
}

#[derive(Deserialize)]
struct ListParams {
    q: Option<String>,
    kind: Option<String>,
    provider: Option<String>,
    sort: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
}

/// Every resource, available or not, as an admin sees it.
async fn resources(
    State(app): State<Shared>,
    headers: HeaderMap,
    QueryParams(p): QueryParams<ListParams>,
) -> Result<Json<DiscoverResponse>, Problem> {
    let c = admin(&app, &headers, perm::ADMIN_READ).await?;
    let q = Query {
        q: p.q,
        kind: p.kind,
        provider: p.provider,
        sort: p.sort.as_deref().unwrap_or("default").parse().map_err(|e: String| Problem::bad_request(e))?,
        limit: Some(p.limit.unwrap_or(50).min(200)),
        cursor: p.cursor,
        include_unavailable: true,
        include_unpriced: true,
        ..Query::default()
    };
    let scores = app.scores();
    let viewer = Viewer::everything();
    let r = app
        .registry
        .index()
        .discover(&q, &Ctx { now: Timestamp::now(), viewer: &viewer, scores: &scores })
        .map_err(|e| Problem::invalid(e.to_string(), vec![]))?;
    // Admins get no tickets: they are not clients of the resources.
    let _ = c;
    Ok(Json(DiscoverResponse::of(&app, &Caller::Open { client: None }, r)))
}

async fn resource(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    let idx = app.registry.index();
    let e = idx.get(&id).ok_or_else(|| Problem::not_found(format!("no resource {id}")))?;
    let mut v = serde_json::to_value(ResourceView::of(&app, None, e)).unwrap_or_default();
    v["lease"] =
        serde_json::to_value(e.record.lease_id.as_deref().and_then(|l| app.registry.lease_of(l))).unwrap_or_default();
    v["reputation_evidence"] = serde_json::to_value(app.reputation.subject(Side::Resource, &id)).unwrap_or_default();
    Ok(Json(v))
}

#[derive(Deserialize)]
struct TimelineParams {
    from: Option<Timestamp>,
    to: Option<Timestamp>,
}

/// Prices and availability of one resource over a span: the next seven days by default.
#[derive(Serialize, JsonSchema)]
pub struct Timeline {
    pub from: Timestamp,
    pub to: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prices: Option<pacod_broker_model::PriceSummary>,
    pub availability: pacod_broker_model::AvailabilitySummary,
}

async fn timeline(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    QueryParams(p): QueryParams<TimelineParams>,
) -> Result<Json<Timeline>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    let idx = app.registry.index();
    let e = idx.get(&id).ok_or_else(|| Problem::not_found(format!("no resource {id}")))?;
    let now = Timestamp::now();
    let from = p.from.unwrap_or(now);
    let to = p.to.unwrap_or_else(|| pacod_broker_model::schedule::days_after(from, 7));
    let span = pacod_broker_model::schedule::Span::new(from, to).map_err(Problem::bad_request)?;
    let s = &e.record.spec;
    Ok(Json(Timeline {
        from,
        to,
        prices: s.pricing.as_ref().map(|pr| pr.during(span)),
        availability: s.availability.during(span, Some(e.live.model(now)), None),
    }))
}

#[derive(Serialize, JsonSchema)]
pub struct ProviderRow {
    #[serde(flatten)]
    pub summary: ProviderSummary,
    pub principal: String,
    pub organisation_id: Option<String>,
    pub source: pacod_broker_model::Source,
    pub resources: usize,
    pub leases: usize,
    pub blocked: bool,
    pub has_inventory: bool,
    pub reputation: Option<pacod_broker_model::Reputation>,
    pub updated_at: Timestamp,
}

async fn providers(State(app): State<Shared>, headers: HeaderMap) -> Result<Json<Vec<ProviderRow>>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    let idx = app.registry.index();
    let leases = app.registry.leases();
    let scores = app.reputation.scores();
    let mut rows: Vec<ProviderRow> = idx
        .providers()
        .map(|p| ProviderRow {
            summary: ProviderSummary::of(p),
            principal: p.principal.clone(),
            organisation_id: p.organisation_id.clone(),
            source: p.source,
            resources: idx.of_provider(&p.id).count(),
            leases: leases.iter().filter(|l| l.provider_id == p.id).count(),
            blocked: app.registry.is_blocked(&p.id),
            has_inventory: p.profile.inventory.is_some(),
            reputation: scores.providers.get(&p.id).cloned(),
            updated_at: p.updated_at,
        })
        .collect();
    rows.sort_by(|a, b| a.summary.name.cmp(&b.summary.name));
    Ok(Json(rows))
}

async fn provider(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    let idx = app.registry.index();
    let p = idx.provider(&id).ok_or_else(|| Problem::not_found(format!("no provider {id}")))?;
    let mut v = serde_json::to_value(ProviderView::of(&app, &idx, p, &Viewer::everything())).unwrap_or_default();
    v["principal"] = p.principal.clone().into();
    v["blocked"] = app.registry.is_blocked(&id).into();
    v["leases"] =
        serde_json::to_value(app.registry.leases().into_iter().filter(|l| l.provider_id == id).collect::<Vec<_>>())
            .unwrap_or_default();
    v["reputation_evidence"] = serde_json::to_value(app.reputation.subject(Side::Provider, &id)).unwrap_or_default();
    Ok(Json(v))
}

async fn block(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<String>) -> Result<StatusCode, Problem> {
    admin(&app, &headers, perm::ADMIN_MANAGE).await?;
    app.registry.block(&id);
    app.persist();
    Ok(StatusCode::NO_CONTENT)
}

async fn unblock(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<String>) -> Result<StatusCode, Problem> {
    admin(&app, &headers, perm::ADMIN_MANAGE).await?;
    app.registry.unblock(&id);
    app.persist();
    Ok(StatusCode::NO_CONTENT)
}

async fn approve(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<String>) -> Result<StatusCode, Problem> {
    admin(&app, &headers, perm::ADMIN_MANAGE).await?;
    app.registry.approve(&id);
    app.persist();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, JsonSchema)]
pub struct VerifyBody {
    pub verified: bool,
}

async fn verify(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Body(b): Body<VerifyBody>,
) -> Result<StatusCode, Problem> {
    admin(&app, &headers, perm::ADMIN_MANAGE).await?;
    app.registry.set_verified(&id, b.verified);
    app.persist();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, JsonSchema)]
pub struct IncidentBody {
    pub note: String,
}

/// Records an incident on a provider's side that an admin upheld; it costs trust.
async fn incident(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Body(b): Body<IncidentBody>,
) -> Result<StatusCode, Problem> {
    let c = admin(&app, &headers, perm::MODERATE).await?;
    let now = Timestamp::now();
    app.reputation.record(Evidence::ProviderIncident { provider: id.clone(), at: now });
    super::public::NOTES.push(super::public::Note {
        at: now,
        kind: "provider_incident".into(),
        subject: id,
        author: c.client_id().unwrap_or_else(|| "admin".into()),
        text: b.note,
        report_id: None,
    });
    Ok(StatusCode::NO_CONTENT)
}

async fn pending(State(app): State<Shared>, headers: HeaderMap) -> Result<Json<Vec<Pending>>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    Ok(Json(app.registry.pending()))
}

async fn leases(State(app): State<Shared>, headers: HeaderMap) -> Result<Json<Vec<LeaseInfo>>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    Ok(Json(app.registry.leases()))
}

async fn evict(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<String>) -> Result<StatusCode, Problem> {
    admin(&app, &headers, perm::ADMIN_MANAGE).await?;
    if app.registry.evict(&id) { Ok(StatusCode::NO_CONTENT) } else { Err(Problem::not_found(format!("no lease {id}"))) }
}

async fn static_list(State(app): State<Shared>, headers: HeaderMap) -> Result<Json<Vec<StaticProvider>>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    Ok(Json(app.registry.static_entries()))
}

/// Adds or replaces one provider's static entries; they are kept in `BROKER_STATE_DIR`.
async fn static_put(
    State(app): State<Shared>,
    headers: HeaderMap,
    Body(p): Body<StaticProvider>,
) -> Result<Json<crate::registry::Announced>, Problem> {
    admin(&app, &headers, perm::ADMIN_MANAGE).await?;
    let principal = p.principal();
    let a = Announcement { ttl_s: None, lease_id: None, provider: p.provider, resources: p.resources };
    let done = app.registry.put_static(&principal, p.verified, a)?;
    app.persist();
    Ok(Json(done))
}

async fn static_delete(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, Problem> {
    admin(&app, &headers, perm::ADMIN_MANAGE).await?;
    let n = app.registry.remove_static(&id);
    app.persist();
    Ok(Json(serde_json::json!({ "removed": n })))
}

fn side(s: &str) -> Result<Side, Problem> {
    match s {
        "resource" | "resources" => Ok(Side::Resource),
        "provider" | "providers" => Ok(Side::Provider),
        "client" | "clients" => Ok(Side::Client),
        _ => Err(Problem::bad_request("side: resource, provider or client")),
    }
}

#[derive(Deserialize)]
struct RepParams {
    side: Option<String>,
    limit: Option<usize>,
    /// `score` (lowest first, to find problems) or `confidence`.
    sort: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct RepRow {
    pub side: Side,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub reputation: pacod_broker_model::Reputation,
}

async fn reputation_list(
    State(app): State<Shared>,
    headers: HeaderMap,
    QueryParams(p): QueryParams<RepParams>,
) -> Result<Json<Vec<RepRow>>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    let s = side(p.side.as_deref().unwrap_or("provider"))?;
    let scores = app.reputation.scores();
    let idx = app.registry.index();
    let map = match s {
        Side::Resource => &scores.resources,
        Side::Provider => &scores.providers,
        Side::Client => &scores.clients,
    };
    let mut rows: Vec<RepRow> = map
        .iter()
        .map(|(id, r)| RepRow {
            side: s,
            id: id.clone(),
            name: match s {
                Side::Resource => idx.get(id).map(|e| e.record.spec.name.clone()),
                Side::Provider => idx.provider(id).map(|p| p.profile.name.clone()),
                Side::Client => None,
            },
            reputation: r.clone(),
        })
        .collect();
    match p.sort.as_deref() {
        Some("confidence") => rows.sort_by(|a, b| b.reputation.confidence.total_cmp(&a.reputation.confidence)),
        _ => rows.sort_by(|a, b| a.reputation.score.total_cmp(&b.reputation.score).then(a.id.cmp(&b.id))),
    }
    rows.truncate(p.limit.unwrap_or(200).min(2000));
    Ok(Json(rows))
}

async fn reputation_one(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((s, id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    let s = side(&s)?;
    let scores = app.reputation.scores();
    Ok(Json(serde_json::json!({
        "side": s,
        "id": id,
        "reputation": scores.get(s, &id),
        "evidence": app.reputation.subject(s, &id),
    })))
}

async fn reputation_action(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((s, id, action)): Path<(String, String, String)>,
) -> Result<StatusCode, Problem> {
    admin(&app, &headers, perm::MODERATE).await?;
    let side = side(&s)?;
    let at = Timestamp::now();
    let ev = match action.as_str() {
        "freeze" => Evidence::Freeze { side, subject: id, frozen: true, at },
        "unfreeze" => Evidence::Freeze { side, subject: id, frozen: false, at },
        "reset" => Evidence::Reset { side, subject: id, at },
        _ => return Err(Problem::bad_request("action: freeze, unfreeze or reset")),
    };
    app.reputation.record(ev);
    app.reputation.compute(at);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, JsonSchema)]
pub struct ReportRow {
    pub id: String,
    pub client: String,
    pub kind: pacod_broker_reputation::ClientReportKind,
    pub at: Timestamp,
}

async fn reports(State(app): State<Shared>, headers: HeaderMap) -> Result<Json<serde_json::Value>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    let rows: Vec<ReportRow> =
        app.reputation.reports().into_iter().map(|(id, client, kind, at)| ReportRow { id, client, kind, at }).collect();
    Ok(Json(serde_json::json!({ "client_reports": rows, "notes": NOTES.list() })))
}

/// Takes back a client report found unfounded.
async fn reject_report(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, Problem> {
    admin(&app, &headers, perm::MODERATE).await?;
    let Some((_, client, kind, _)) = app.reputation.reports().into_iter().find(|r| r.0 == id) else {
        return Err(Problem::not_found(format!("no counted report {id}")));
    };
    app.reputation.record(Evidence::ClientReportRejected { client, report: kind, id, at: Timestamp::now() });
    Ok(StatusCode::NO_CONTENT)
}

async fn events(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Problem> {
    admin(&app, &headers, perm::ADMIN_READ).await?;
    let s = BroadcastStream::new(app.registry.changes.subscribe()).map(|c| {
        Ok(match c {
            Ok(change) => Event::default().event("change").data(serde_json::to_string(&change).unwrap_or_default()),
            Err(_) => Event::default().event("resync").data("{}"),
        })
    });
    Ok(Sse::new(s.take_until(super::public::shutting_down())).keep_alive(KeepAlive::default()))
}

async fn openapi() -> Json<serde_json::Value> {
    Json(crate::openapi::document())
}
