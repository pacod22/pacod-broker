//! What the APIs answer with: resources and providers as callers see them.

use jiff::Timestamp;
use pacod_broker_index::{Entry, Hit, Index, LiveView, ProviderEntry, Results, Viewer};
use pacod_broker_model::availability::At;
use pacod_broker_model::pricing::EffectivePrice;
use pacod_broker_model::{Inventory, ProviderProfile, Reputation, ResourceRecord, Source};
use schemars::JsonSchema;
use serde::Serialize;

use crate::app::App;
use crate::auth::Caller;

/// A provider, briefly, as it appears with each resource.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProviderSummary {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub summary: String,
    pub verified: bool,
}

impl ProviderSummary {
    pub fn of(p: &ProviderEntry) -> Self {
        Self {
            id: p.id.clone(),
            name: p.profile.name.clone(),
            title: p.profile.text.title.clone(),
            summary: p.profile.text.summary.clone(),
            verified: p.verified,
        }
    }
}

/// One discovery result.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HitView {
    pub resource: ResourceRecord,
    pub provider: ProviderSummary,
    #[serde(flatten)]
    pub hit: Hit,
    /// Proof that this client was shown this resource now: quote it in usage reports and ratings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DiscoverResponse {
    pub generation: u64,
    pub at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub during: Option<pacod_broker_model::schedule::Span>,
    pub total: usize,
    pub hits: Vec<HitView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl DiscoverResponse {
    pub fn of(app: &App, caller: &Caller, r: Results) -> Self {
        let client = caller.client_id();
        let now = Timestamp::now();
        let hits = r
            .hits
            .into_iter()
            .map(|h| {
                let ticket = client.as_ref().map(|c| app.tickets.issue(c, &h.id, now));
                HitView {
                    resource: h.entry.record.clone(),
                    provider: ProviderSummary::of(&h.entry.provider),
                    hit: h,
                    ticket,
                }
            })
            .collect();
        Self { generation: r.generation, at: r.at, during: r.during, total: r.total, hits, next_cursor: r.next_cursor }
    }
}

/// One resource, read on its own.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ResourceView {
    #[serde(flatten)]
    pub record: ResourceRecord,
    pub provider: ProviderSummary,
    pub live: LiveView,
    /// The price now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_now: Option<EffectivePrice>,
    pub available_now: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity_now: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reputation: Option<Reputation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_reputation: Option<Reputation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
}

impl ResourceView {
    pub fn of(app: &App, caller: Option<&Caller>, e: &Entry) -> Self {
        let now = Timestamp::now();
        let s = &e.record.spec;
        let at = s.availability.at(now, Some(e.live.model(now)));
        let scores = app.reputation.scores();
        Self {
            record: e.record.clone(),
            provider: ProviderSummary::of(&e.provider),
            live: e.live.view(),
            price_now: s.pricing.as_ref().and_then(|p| p.at(now)),
            available_now: at.is_open(),
            capacity_now: match at {
                At::Closed => Some(0),
                At::Open(c) => c,
            },
            reputation: scores.resources.get(&e.record.id).cloned(),
            provider_reputation: scores.providers.get(&e.record.provider_id).cloned(),
            ticket: caller.and_then(Caller::client_id).map(|c| app.tickets.issue(&c, &e.record.id, now)),
        }
    }
}

/// A provider, read on its own.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProviderView {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organisation_id: Option<String>,
    pub verified: bool,
    pub source: Source,
    /// The profile; the inventory only when the caller may see it.
    pub profile: ProviderProfile,
    pub registered_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reputation: Option<Reputation>,
    /// The provider's resources the caller may see: id, kind, name, summary.
    pub resources: Vec<ResourceBrief>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ResourceBrief {
    pub id: String,
    pub key: String,
    pub kind: String,
    pub name: String,
    pub summary: String,
}

impl ProviderView {
    pub fn of(app: &App, idx: &Index, p: &ProviderEntry, viewer: &Viewer) -> Self {
        let mut profile = p.profile.clone();
        if profile.inventory.as_ref().is_some_and(|i: &Inventory| !viewer.can_see(&i.visibility)) {
            profile.inventory = None;
        }
        let now = Timestamp::now();
        let resources = idx
            .of_provider(&p.id)
            .filter(|e| !e.live.is_expired(now) && viewer.can_see(&e.record.spec.access.visibility))
            .map(|e| ResourceBrief {
                id: e.record.id.clone(),
                key: e.record.spec.key.clone(),
                kind: e.record.spec.kind.clone(),
                name: e.record.spec.name.clone(),
                summary: e.record.spec.text.summary.clone(),
            })
            .collect();
        Self {
            id: p.id.clone(),
            organisation_id: p.organisation_id.clone(),
            verified: p.verified,
            source: p.source,
            profile,
            registered_at: p.registered_at,
            updated_at: p.updated_at,
            reputation: app.reputation.scores().providers.get(&p.id).cloned(),
            resources,
        }
    }
}
