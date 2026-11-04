//! The public API (docs/api.md): registration, discovery, the watch stream, reports, tickets.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query as QueryParams, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use futures_util::Stream;
use futures_util::StreamExt;
use http::{HeaderMap, StatusCode};
use jiff::Timestamp;
use pacod_broker_index::{Ctx, Query};
use pacod_broker_reputation::{ClientReportKind, Evidence, Outcome};
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use super::problem::Body;
use super::views::{DiscoverResponse, ProviderView, ResourceView};
use super::{Problem, Shared, caller, require};
use crate::auth::{Caller, perm};
use crate::registry::{Announcement, Change, Heartbeat};

/// Set while the broker shuts down: `/readyz` fails so load balancers stop sending.
pub static DRAINING: AtomicBool = AtomicBool::new(false);

pub fn router(app: Shared) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .route("/v1/announce", put(announce))
        .route("/v1/leases/{id}/heartbeat", post(heartbeat))
        .route("/v1/leases/{id}", delete(deregister))
        .route("/v1/discover", post(discover))
        .route("/v1/resources", get(list_resources))
        .route("/v1/resources/{id}", get(get_resource))
        .route("/v1/providers/{id}", get(get_provider))
        .route("/v1/watch", get(watch))
        .route("/v1/reports/usage", post(report_usage))
        .route("/v1/reports/feedback", post(report_feedback))
        .route("/v1/reports/client", post(report_client))
        .route("/v1/tickets/verify", post(verify_ticket))
        .route("/v1/reputation/me", get(reputation_me))
        .route("/v1/schema", get(schema))
        .route("/mcp", post(super::mcp::handle).get(super::mcp::get))
        .with_state(app)
}

async fn readyz(State(app): State<Shared>) -> Response {
    if DRAINING.load(Ordering::Relaxed) {
        return (StatusCode::SERVICE_UNAVAILABLE, "draining").into_response();
    }
    match &app.identity {
        Some(id) if !id.ready() => (StatusCode::SERVICE_UNAVAILABLE, "waiting for pacod-id").into_response(),
        _ => "ready".into_response(),
    }
}

async fn announce(
    State(app): State<Shared>,
    headers: HeaderMap,
    Body(a): Body<Announcement>,
) -> Result<Response, Problem> {
    let c = caller(&app, &headers).await?;
    require(&app, &c, perm::ANNOUNCE)?;
    let who = c.announcer(&a.provider.name).ok_or_else(|| Problem::unauthorized("sign in to announce"))?;
    let app2 = app.clone();
    // Validating and storing a large announcement is CPU work; keep it off the runtime's threads.
    let res = tokio::task::spawn_blocking(move || app2.registry.announce(&who, a, pacod_broker_model::Source::Lease))
        .await
        .map_err(|_| {
            Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "the announcement could not be handled")
        })?;
    match res {
        Ok(done) => Ok(Json(done).into_response()),
        Err(e @ crate::registry::RegistryError::Pending(_)) => Ok(Problem::from(e).into_response()),
        Err(e) => {
            metrics::counter!("broker_announces_refused_total").increment(1);
            Err(e.into())
        }
    }
}

#[derive(Deserialize)]
struct OwnerQuery {
    /// In open mode, the provider name the lease was announced under.
    provider: Option<String>,
}

fn owner(c: &Caller, q: &OwnerQuery) -> Option<String> {
    match c {
        // Open mode: anyone may keep a lease alive; nothing to check against.
        Caller::Open { .. } => None,
        _ => c.lease_owner(q.provider.as_deref()),
    }
}

async fn heartbeat(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    QueryParams(q): QueryParams<OwnerQuery>,
    body: Option<Json<Heartbeat>>,
) -> Result<Response, Problem> {
    let c = caller(&app, &headers).await?;
    require(&app, &c, perm::ANNOUNCE)?;
    let hb = body.map(|Json(b)| b).unwrap_or_default();
    let expires_at = app.registry.heartbeat(owner(&c, &q).as_deref(), &id, hb)?;
    Ok(Json(serde_json::json!({ "lease_id": id, "expires_at": expires_at })).into_response())
}

async fn deregister(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    QueryParams(q): QueryParams<OwnerQuery>,
) -> Result<StatusCode, Problem> {
    let c = caller(&app, &headers).await?;
    require(&app, &c, perm::ANNOUNCE)?;
    app.registry.deregister(owner(&c, &q).as_deref(), &id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Runs a query for a caller.
pub fn run_query(app: &App, c: &Caller, q: &Query) -> Result<DiscoverResponse, Problem> {
    let started = std::time::Instant::now();
    let scores = app.scores();
    let viewer = c.viewer(&app.cfg, c.client_id().and_then(|id| scores.0.client(&id)));
    let idx = app.registry.index();
    let r = idx
        .discover(q, &Ctx { now: Timestamp::now(), viewer: &viewer, scores: &scores })
        .map_err(|e| Problem::invalid(e.to_string(), vec![]))?;
    metrics::histogram!("broker_discover_seconds").record(started.elapsed().as_secs_f64());
    metrics::counter!("broker_discoveries_total").increment(1);
    Ok(DiscoverResponse::of(app, c, r))
}

use crate::app::App;

async fn discover(
    State(app): State<Shared>,
    headers: HeaderMap,
    Body(q): Body<Query>,
) -> Result<Json<DiscoverResponse>, Problem> {
    let c = caller(&app, &headers).await?;
    require(&app, &c, perm::DISCOVER)?;
    app.note_client(&c);
    Ok(Json(run_query(&app, &c, &q)?))
}

/// The query of `GET /v1/resources`: the common filters as parameters; lists separated by
/// commas.
#[derive(Debug, Deserialize)]
struct ListParams {
    kind: Option<String>,
    q: Option<String>,
    tag: Option<String>,
    protocol: Option<String>,
    capability: Option<String>,
    region: Option<String>,
    provider: Option<String>,
    at: Option<Timestamp>,
    sort: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
    include_unavailable: Option<bool>,
}

fn list(v: &Option<String>) -> Vec<String> {
    v.as_deref()
        .map(|s| s.split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

async fn list_resources(
    State(app): State<Shared>,
    headers: HeaderMap,
    QueryParams(p): QueryParams<ListParams>,
) -> Result<Json<DiscoverResponse>, Problem> {
    let c = caller(&app, &headers).await?;
    require(&app, &c, perm::DISCOVER)?;
    let q = Query {
        kind: p.kind.clone(),
        q: p.q.clone(),
        tags: list(&p.tag),
        protocols: list(&p.protocol),
        capabilities: list(&p.capability),
        regions: list(&p.region),
        provider: p.provider.clone(),
        at: p.at,
        sort: p.sort.as_deref().unwrap_or("default").parse().map_err(|e: String| Problem::bad_request(e))?,
        limit: p.limit,
        cursor: p.cursor.clone(),
        include_unavailable: p.include_unavailable.unwrap_or(false),
        ..Query::default()
    };
    app.note_client(&c);
    Ok(Json(run_query(&app, &c, &q)?))
}

async fn get_resource(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ResourceView>, Problem> {
    let c = caller(&app, &headers).await?;
    require(&app, &c, perm::DISCOVER)?;
    let idx = app.registry.index();
    let viewer = c.viewer(&app.cfg, None);
    let e = idx
        .get(&id)
        .filter(|e| !e.live.is_expired(Timestamp::now()) && viewer.can_see(&e.record.spec.access.visibility))
        .ok_or_else(|| Problem::not_found(format!("no resource {id}")))?;
    Ok(Json(ResourceView::of(&app, Some(&c), e)))
}

async fn get_provider(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ProviderView>, Problem> {
    let c = caller(&app, &headers).await?;
    require(&app, &c, perm::DISCOVER)?;
    let idx = app.registry.index();
    let viewer = c.viewer(&app.cfg, None);
    let p = idx.provider(&id).ok_or_else(|| Problem::not_found(format!("no provider {id}")))?;
    let view = ProviderView::of(&app, &idx, p, &viewer);
    // A provider with nothing the caller may see is not shown to it at all.
    if view.resources.is_empty() && !viewer.all {
        return Err(Problem::not_found(format!("no provider {id}")));
    }
    Ok(Json(view))
}

#[derive(Deserialize)]
struct WatchParams {
    after: Option<u64>,
}

/// Changes as server-sent events, so a gateway or a console can keep a copy current:
/// `upserted`, `removed`, `provider` and `generation` events, `resync` when the stream fell
/// behind (read the catalogue again).
async fn watch(
    State(app): State<Shared>,
    headers: HeaderMap,
    QueryParams(p): QueryParams<WatchParams>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Problem> {
    let c = caller(&app, &headers).await?;
    require(&app, &c, perm::DISCOVER)?;
    let viewer = c.viewer(&app.cfg, None);
    let current = app.registry.index().generation;
    let first = Event::default()
        .event(if p.after.is_some_and(|a| a < current) { "resync" } else { "hello" })
        .data(serde_json::json!({ "generation": current }).to_string());
    let app2 = app.clone();
    let changes = BroadcastStream::new(app.registry.changes.subscribe()).filter_map(move |c| {
        let app = app2.clone();
        let viewer = viewer.clone();
        async move {
            match c {
                Ok(change) => {
                    if let Change::Upserted { resource_id, .. } = &change {
                        // Hidden resources are not mentioned; the index may not hold it yet, in
                        // which case the next `generation` event covers it.
                        let idx = app.registry.index();
                        if idx.get(resource_id).is_some_and(|e| !viewer.can_see(&e.record.spec.access.visibility)) {
                            return None;
                        }
                    }
                    let name = match &change {
                        Change::Upserted { .. } => "upserted",
                        Change::Removed { .. } => "removed",
                        Change::Provider { .. } => "provider",
                        Change::Generation { .. } => "generation",
                    };
                    Some(Ok(Event::default().event(name).data(serde_json::to_string(&change).unwrap_or_default())))
                }
                Err(BroadcastStreamRecvError::Lagged(_)) => Some(Ok(Event::default().event("resync").data("{}"))),
            }
        }
    });
    let stream = futures_util::stream::once(async move { Ok(first) }).chain(changes);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// --- Reports -------------------------------------------------------------------------------

/// Free text people wrote with ratings and reports, for admins: the last thousand, in memory.
pub struct Notes(Mutex<VecDeque<Note>>);

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Note {
    pub at: Timestamp,
    pub kind: String,
    pub subject: String,
    pub author: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_id: Option<String>,
}

impl Notes {
    pub fn new() -> Self {
        Self(Mutex::new(VecDeque::with_capacity(64)))
    }

    pub fn push(&self, n: Note) {
        let mut q = self.0.lock();
        if q.len() >= 1000 {
            q.pop_front();
        }
        q.push_back(n);
    }

    pub fn list(&self) -> Vec<Note> {
        self.0.lock().iter().rev().cloned().collect()
    }
}

impl Default for Notes {
    fn default() -> Self {
        Self::new()
    }
}

pub static NOTES: std::sync::LazyLock<Notes> = std::sync::LazyLock::new(Notes::new);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UsageBatch {
    pub reports: Vec<UsageReport>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UsageReport {
    pub ticket: String,
    pub outcome: Outcome,
    #[serde(default)]
    pub ttft_ms: Option<f64>,
    #[serde(default)]
    pub ms_per_token: Option<f64>,
    /// What was charged for one unit of `rate` (per million tokens for token rates).
    #[serde(default)]
    pub price_charged: Option<f64>,
    /// The rate `price_charged` is for: `input`, `per_request`, …
    #[serde(default)]
    pub rate: Option<String>,
    /// When the call was made; now by default.
    #[serde(default)]
    pub at: Option<Timestamp>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Accepted {
    pub accepted: usize,
    /// Reports that did not count, by their position, with the reason.
    pub refused: Vec<(usize, String)>,
}

fn client_of(c: &Caller) -> Result<String, Problem> {
    c.client_id().ok_or_else(|| {
        Problem::unauthorized(
            "reports need an identified client: a pacod-id token, or in open mode the X-Broker-Client header",
        )
    })
}

async fn report_usage(
    State(app): State<Shared>,
    headers: HeaderMap,
    Body(b): Body<UsageBatch>,
) -> Result<Json<Accepted>, Problem> {
    let c = caller(&app, &headers).await?;
    require(&app, &c, perm::REPORT_USAGE)?;
    let client = client_of(&c)?;
    if b.reports.len() > 1000 {
        return Err(Problem::bad_request("at most 1000 reports in one batch"));
    }
    app.note_client(&c);
    let now = Timestamp::now();
    let idx = app.registry.index();
    let (mut accepted, mut refused) = (0, Vec::new());
    for (i, r) in b.reports.into_iter().enumerate() {
        let t = match app.tickets.check(&r.ticket, &client, None, now) {
            Ok(t) => t,
            Err(e) => {
                refused.push((i, e.to_string()));
                continue;
            }
        };
        let at = r.at.unwrap_or(now).min(now);
        if at.as_millisecond() < t.t {
            refused.push((i, "the call is dated before the ticket was issued".into()));
            continue;
        }
        let Some(e) = idx.get(&t.r) else {
            refused.push((i, "the resource is no longer announced".into()));
            continue;
        };
        let s = &e.record.spec;
        let listed = match (&r.rate, &s.pricing) {
            (Some(rate), Some(p)) => p.rate_at(rate, at),
            _ => None,
        };
        let counted = app.reputation.record(Evidence::Usage {
            resource: e.record.id.clone(),
            provider: e.record.provider_id.clone(),
            kind: s.kind.clone(),
            client: client.clone(),
            client_org: c.organisation().map(String::from),
            provider_org: e.provider.organisation_id.clone(),
            outcome: r.outcome,
            ttft_ms: r.ttft_ms,
            ms_per_token: r.ms_per_token,
            declared_ttft_ms: s.availability.latency.and_then(|l| l.ttft_ms),
            price_charged: r.price_charged,
            price_listed: listed,
            at,
        });
        if counted {
            accepted += 1;
        } else {
            refused.push((
                i,
                "not counted: the provider's own organisation, or more reports than one client counts per hour".into(),
            ));
        }
    }
    metrics::counter!("broker_reports_total", "kind" => "usage").increment(accepted as u64);
    Ok(Json(Accepted { accepted, refused }))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedbackReport {
    pub ticket: String,
    /// 1 (poor) to 5 (excellent).
    pub rating: u8,
    /// Kept for admins only, never shown to other callers.
    #[serde(default)]
    pub comment: Option<String>,
}

async fn report_feedback(
    State(app): State<Shared>,
    headers: HeaderMap,
    Body(b): Body<FeedbackReport>,
) -> Result<Json<Accepted>, Problem> {
    let c = caller(&app, &headers).await?;
    require(&app, &c, perm::REPORT_FEEDBACK)?;
    let client = client_of(&c)?;
    if !(1..=5).contains(&b.rating) {
        return Err(Problem::invalid("rating: 1 to 5", vec![]));
    }
    let now = Timestamp::now();
    let t = app
        .tickets
        .check(&b.ticket, &client, None, now)
        .map_err(|e| Problem::invalid(format!("ticket: {e}"), vec![]))?;
    let idx = app.registry.index();
    let e = idx.get(&t.r).ok_or_else(|| Problem::not_found("the resource is no longer announced"))?;
    let counted = app.reputation.record(Evidence::Feedback {
        resource: e.record.id.clone(),
        provider: e.record.provider_id.clone(),
        client: client.clone(),
        client_org: c.organisation().map(String::from),
        provider_org: e.provider.organisation_id.clone(),
        rating: b.rating,
        at: now,
    });
    if let Some(text) = b.comment.filter(|t| !t.trim().is_empty()) {
        NOTES.push(Note {
            at: now,
            kind: "feedback".into(),
            subject: e.record.id.clone(),
            author: client,
            text: text.chars().take(2000).collect(),
            report_id: None,
        });
    }
    metrics::counter!("broker_reports_total", "kind" => "feedback").increment(counted as u64);
    Ok(Json(Accepted {
        accepted: counted as usize,
        refused: if counted {
            vec![]
        } else {
            vec![(0, "not counted: the provider's own organisation, or a rating within the hour".into())]
        },
    }))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClientReport {
    /// The ticket the client presented to the provider.
    pub ticket: String,
    pub kind: ClientReportKind,
    #[serde(default)]
    pub details: Option<String>,
}

/// Checks that the resource a ticket names belongs to the calling provider.
fn ticket_for_caller(
    app: &App,
    c: &Caller,
    ticket: &str,
) -> Result<(crate::tickets::Ticket, Arc<pacod_broker_index::Entry>), Problem> {
    let t = app.tickets.open(ticket, Timestamp::now()).map_err(|e| Problem::invalid(format!("ticket: {e}"), vec![]))?;
    let idx = app.registry.index();
    let e = idx
        .get(&t.r)
        .cloned()
        .ok_or_else(|| Problem::not_found("the resource of the ticket is no longer announced"))?;
    if let Caller::Principal(p) = c
        && e.provider.principal != p.sub
    {
        return Err(Problem::forbidden("the ticket is for another provider's resource"));
    }
    Ok((t, e))
}

async fn report_client(
    State(app): State<Shared>,
    headers: HeaderMap,
    Body(b): Body<ClientReport>,
) -> Result<Json<serde_json::Value>, Problem> {
    let c = caller(&app, &headers).await?;
    require(&app, &c, perm::REPORT_CLIENT)?;
    let (t, e) = ticket_for_caller(&app, &c, &b.ticket)?;
    let id = format!("rep_{}", uuid::Uuid::now_v7().simple());
    let now = Timestamp::now();
    let counted = app.reputation.record(Evidence::ClientReport {
        client: t.c.clone(),
        provider: e.record.provider_id.clone(),
        report: b.kind,
        id: id.clone(),
        at: now,
    });
    if let Some(text) = b.details.filter(|t| !t.trim().is_empty()) {
        NOTES.push(Note {
            at: now,
            kind: "client_report".into(),
            subject: t.c.clone(),
            author: e.record.provider_id.clone(),
            text: text.chars().take(2000).collect(),
            report_id: Some(id.clone()),
        });
    }
    metrics::counter!("broker_reports_total", "kind" => "client").increment(counted as u64);
    Ok(Json(serde_json::json!({ "id": id, "counted": counted })))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifyTicket {
    pub ticket: String,
}

/// For providers: who presented this ticket, and how reputable they are.
async fn verify_ticket(
    State(app): State<Shared>,
    headers: HeaderMap,
    Body(b): Body<VerifyTicket>,
) -> Result<Json<serde_json::Value>, Problem> {
    let c = caller(&app, &headers).await?;
    require(&app, &c, perm::ANNOUNCE)?;
    let (t, e) = ticket_for_caller(&app, &c, &b.ticket)?;
    let rep = app.reputation.scores().clients.get(&t.c).cloned();
    Ok(Json(serde_json::json!({
        "client": t.c,
        "resource_id": e.record.id,
        "issued_at": Timestamp::from_millisecond(t.t).ok(),
        "client_reputation": rep,
    })))
}

async fn reputation_me(State(app): State<Shared>, headers: HeaderMap) -> Result<Json<serde_json::Value>, Problem> {
    let c = caller(&app, &headers).await?;
    let client = client_of(&c)?;
    let scores = app.reputation.scores();
    let pid = pacod_broker_model::provider_id(&app.cfg.id, &client);
    Ok(Json(serde_json::json!({
        "client": client,
        "as_client": scores.clients.get(&client),
        "provider_id": pid,
        "as_provider": scores.providers.get(&pid),
    })))
}

/// The JSON Schemas of what the API takes, for providers writing announcements.
async fn schema() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "announcement": schemars::schema_for!(Announcement),
        "query": schemars::schema_for!(Query),
        "heartbeat": schemars::schema_for!(Heartbeat),
        "usage": schemars::schema_for!(UsageBatch),
        "feedback": schemars::schema_for!(FeedbackReport),
        "client_report": schemars::schema_for!(ClientReport),
    }))
}
