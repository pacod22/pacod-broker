//! The registry: who announced what, under which lease, and the index built from it
//! (docs/concept.md §1).
//!
//! Writes (announce, deregister, a lease expiring, an admin action) change the records under one
//! lock and mark the index dirty; one task rebuilds the index at most every `BROKER_REBUILD_MS`
//! and swaps it in. Heartbeats touch only the live cells of their lease's resources. Reads never
//! take the lock: they load the current index.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;
use jiff::{SignedDuration, Timestamp};
use pacod_broker_index::{Entry, Index, Live, ProviderEntry};
use pacod_broker_model::{
    LiveStatus, Problems, ProviderProfile, ResourceRecord, ResourceSpec, Source, provider_id, resource_id,
};
use pacod_broker_reputation::{Engine, Evidence};
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, broadcast};

use crate::config::{Approval, Config};

/// Who is announcing: the principal and what the broker knows of it.
#[derive(Debug, Clone)]
pub struct Announcer {
    /// The pacod-id subject, or `anon:<provider name>` in open mode.
    pub principal: String,
    pub organisation_id: Option<String>,
    pub verified: bool,
}

/// What a provider sends to announce.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Announcement {
    /// Seconds the lease lasts without a heartbeat. The broker's default when absent.
    #[serde(default)]
    pub ttl_s: Option<u64>,
    /// Replace the resources of this lease (after a provider restarts with its lease id kept);
    /// otherwise a new lease is opened.
    #[serde(default)]
    pub lease_id: Option<String>,
    pub provider: ProviderProfile,
    #[serde(default)]
    pub resources: Vec<ResourceSpec>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Announced {
    pub lease_id: String,
    pub provider_id: String,
    pub ttl_s: u64,
    /// When to send the next heartbeat: a third of the lease.
    pub heartbeat_every_s: u64,
    pub expires_at: Timestamp,
    /// The id of each resource, by key.
    pub resources: HashMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// What a heartbeat may carry.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Heartbeat {
    #[serde(default)]
    pub status: Option<LiveStatus>,
    #[serde(default)]
    pub in_use: Option<u32>,
    #[serde(default)]
    pub ttft_ms: Option<f64>,
    /// Per resource, by key: the same fields for one resource.
    #[serde(default)]
    pub resources: HashMap<String, HeartbeatOne>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatOne {
    #[serde(default)]
    pub status: Option<LiveStatus>,
    #[serde(default)]
    pub in_use: Option<u32>,
    #[serde(default)]
    pub ttft_ms: Option<f64>,
}

/// A change, for the watch stream and the admin events.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Change {
    Upserted { resource_id: String, provider_id: String },
    Removed { resource_id: String, provider_id: String, reason: String },
    Provider { provider_id: String, what: String },
    Generation { generation: u64, resources: usize },
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("{0}")]
    Invalid(Problems),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Limit(String),
    /// The provider waits for an admin's approval.
    #[error("provider {0} waits for approval")]
    Pending(String),
}

/// A lease and what it covers.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LeaseInfo {
    pub id: String,
    pub provider_id: String,
    pub principal: String,
    pub ttl_s: u64,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
    pub last_heartbeat: Timestamp,
    pub resources: Vec<String>,
}

struct Lease {
    info: LeaseInfo,
    /// The live cell of each resource, by key.
    cells: HashMap<String, Arc<Live>>,
}

/// A provider waiting for approval, with what it tried to announce.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Pending {
    pub provider_id: String,
    pub principal: String,
    pub organisation_id: Option<String>,
    pub profile: ProviderProfile,
    pub resources: usize,
    pub first_seen: Timestamp,
}

#[derive(Default)]
struct Inner {
    providers: HashMap<String, Arc<ProviderEntry>>,
    /// Resource id → entry.
    entries: HashMap<String, Arc<Entry>>,
    leases: HashMap<String, Lease>,
    blocked: HashSet<String>,
    approved: HashSet<String>,
    /// Providers an admin vouched for.
    verified: HashSet<String>,
    pending: HashMap<String, Pending>,
    /// Uptime accounting: resource id → seconds up and expected since the last sample.
    uptime: HashMap<String, (f64, f64, String)>,
}

pub struct Registry {
    pub cfg: Arc<Config>,
    index: ArcSwap<Index>,
    inner: Mutex<Inner>,
    generation: AtomicU64,
    dirty: Notify,
    pub changes: broadcast::Sender<Change>,
    pub reputation: Arc<Engine>,
}

impl Registry {
    pub fn new(cfg: Arc<Config>, reputation: Arc<Engine>) -> Arc<Self> {
        let (changes, _) = broadcast::channel(4096);
        Arc::new(Self {
            cfg,
            index: ArcSwap::from_pointee(Index::default()),
            inner: Mutex::new(Inner::default()),
            generation: AtomicU64::new(0),
            dirty: Notify::new(),
            changes,
            reputation,
        })
    }

    /// The current index.
    pub fn index(&self) -> Arc<Index> {
        self.index.load_full()
    }

    fn notify(&self, c: Change) {
        // Nobody listening is fine.
        let _ = self.changes.send(c);
    }

    /// Rebuilds the index now from the records.
    pub fn rebuild(&self) -> Arc<Index> {
        let started = std::time::Instant::now();
        let (providers, entries) = {
            let inner = self.inner.lock();
            (inner.providers.values().cloned().collect::<Vec<_>>(), inner.entries.values().cloned().collect::<Vec<_>>())
        };
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let idx = Arc::new(Index::build(generation, providers, entries));
        self.index.store(idx.clone());
        metrics::histogram!("broker_index_rebuild_seconds").record(started.elapsed().as_secs_f64());
        metrics::gauge!("broker_index_generation").set(generation as f64);
        metrics::gauge!("broker_resources").set(idx.len() as f64);
        for (kind, n) in idx.kinds() {
            metrics::gauge!("broker_resources_by_kind", "kind" => kind.to_string()).set(n as f64);
        }
        metrics::gauge!("broker_providers").set(idx.providers().count() as f64);
        self.notify(Change::Generation { generation, resources: idx.len() });
        idx
    }

    fn mark_dirty(&self) {
        self.dirty.notify_one();
    }

    /// The rebuild task: waits for changes, gathers them for `BROKER_REBUILD_MS`, rebuilds.
    pub async fn run_rebuilder(self: Arc<Self>) {
        loop {
            self.dirty.notified().await;
            if !self.cfg.rebuild.is_zero() {
                tokio::time::sleep(self.cfg.rebuild).await;
            }
            let this = self.clone();
            // Building is CPU work; it must not hold up the runtime's other tasks.
            let _ = tokio::task::spawn_blocking(move || this.rebuild()).await;
        }
    }

    /// The sweeper: every second, ends expired leases; every minute, samples uptime.
    pub async fn run_sweeper(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut n: u64 = 0;
        loop {
            tick.tick().await;
            n += 1;
            let now = Timestamp::now();
            self.sweep(now);
            if n.is_multiple_of(60) {
                self.sample_uptime(now, 60.0);
            }
        }
    }

    /// Ends every lease that has expired.
    pub fn sweep(&self, now: Timestamp) -> usize {
        let expired: Vec<String> = {
            let inner = self.inner.lock();
            inner.leases.values().filter(|l| l.info.expires_at <= now).map(|l| l.info.id.clone()).collect()
        };
        for id in &expired {
            self.end_lease(id, false, "expired");
        }
        if !expired.is_empty() {
            metrics::counter!("broker_leases_expired_total").increment(expired.len() as u64);
        }
        expired.len()
    }

    /// Counts, for each resource, whether it was up when its schedule said it would be, over the
    /// last `secs` seconds, and records the sample as reputation evidence.
    pub fn sample_uptime(&self, now: Timestamp, secs: f64) {
        let idx = self.index();
        let notice = SignedDuration::try_from(self.cfg.reputation.maintenance_notice).unwrap_or(SignedDuration::ZERO);
        let mut inner = self.inner.lock();
        // Resources in the index were announced; those whose lease ended are counted as down
        // below, from the accounting kept for them.
        let mut seen = HashSet::new();
        for e in idx.entries() {
            let r = &e.record;
            seen.insert(r.id.clone());
            let a = &r.spec.availability;
            // Expected: the schedule says open (ignoring the live status). Planned maintenance
            // announced in time is not expected time; maintenance announced late is.
            let mut expected = a.at(now, None).is_open();
            let late_maintenance = a.short_notice(notice).any(|m| now >= m.from && now < m.to);
            if late_maintenance {
                expected = true;
            }
            if !expected {
                continue;
            }
            let up = !e.live.is_expired(now) && e.live.status().usable() && a.at(now, None).is_open();
            let acc = inner.uptime.entry(r.id.clone()).or_insert((0.0, 0.0, r.provider_id.clone()));
            acc.0 += if up { secs } else { 0.0 };
            acc.1 += secs;
        }
        // Resources that vanished while their accounting was open count as down for the sample.
        for (id, acc) in inner.uptime.iter_mut() {
            if !seen.contains(id) {
                acc.1 += secs;
            }
        }
        // Every hour of expected time becomes one piece of evidence.
        let ready: Vec<(String, f64, f64, String)> = inner
            .uptime
            .iter()
            .filter(|(_, a)| a.1 >= 3600.0)
            .map(|(id, a)| (id.clone(), a.0, a.1, a.2.clone()))
            .collect();
        for (id, ..) in &ready {
            inner.uptime.remove(id);
        }
        drop(inner);
        for (resource, up, expected, provider) in ready {
            self.reputation.record(Evidence::Uptime { resource, provider, up, expected, at: now });
        }
    }

    /// Announces a provider's resources under a lease.
    pub fn announce(&self, who: &Announcer, a: Announcement, source: Source) -> Result<Announced, RegistryError> {
        let now = Timestamp::now();
        let cfg = &self.cfg;
        let mut p = Problems::new();
        a.provider.validate(&mut p, cfg.max_description_bytes);
        let mut keys = HashSet::new();
        for (i, r) in a.resources.iter().enumerate() {
            let path = format!("resources[{i}]");
            r.validate(&mut p, &path, cfg.max_description_bytes);
            r.validate_runs_on(a.provider.inventory.as_ref(), &mut p, &path);
            p.check(keys.insert(r.key.as_str()), format!("{path}.key"), "not unique in the announcement");
        }
        if a.resources.len() > cfg.max_resources_per_provider {
            p.add("resources", format_args!("at most {} per provider", cfg.max_resources_per_provider));
        }
        let ttl = Duration::from_secs(a.ttl_s.unwrap_or(cfg.lease_ttl_default.as_secs()));
        if ttl < cfg.lease_ttl_min || ttl > cfg.lease_ttl_max {
            p.add("ttl_s", format_args!("from {} to {}", cfg.lease_ttl_min.as_secs(), cfg.lease_ttl_max.as_secs()));
        }
        if !p.is_empty() {
            return Err(RegistryError::Invalid(p));
        }

        let pid = provider_id(&cfg.id, &who.principal);
        let mut inner = self.inner.lock();
        if inner.blocked.contains(&pid) {
            return Err(RegistryError::Forbidden(format!("provider {pid} is blocked")));
        }
        if cfg.approval == Approval::NewProviders && source == Source::Lease && !inner.approved.contains(&pid) {
            inner.pending.entry(pid.clone()).or_insert_with(|| Pending {
                provider_id: pid.clone(),
                principal: who.principal.clone(),
                organisation_id: who.organisation_id.clone(),
                profile: a.provider.clone(),
                resources: a.resources.len(),
                first_seen: now,
            });
            drop(inner);
            self.notify(Change::Provider { provider_id: pid.clone(), what: "pending".into() });
            return Err(RegistryError::Pending(pid));
        }

        // The lease: the one named (it must be this provider's), or a new one.
        let lease_id = match &a.lease_id {
            Some(id) => match inner.leases.get(id) {
                Some(l) if l.info.provider_id == pid => id.clone(),
                Some(_) => return Err(RegistryError::Forbidden(format!("lease {id} belongs to another provider"))),
                None => {
                    return Err(RegistryError::NotFound(format!(
                        "lease {id} is unknown or has expired; announce without it"
                    )));
                }
            },
            None => {
                let open = inner.leases.values().filter(|l| l.info.provider_id == pid).count();
                if source == Source::Lease && open >= cfg.max_leases_per_provider {
                    return Err(RegistryError::Limit(format!(
                        "provider {pid} holds {open} leases, the most allowed; reuse one with lease_id or let one end"
                    )));
                }
                format!("lse_{}", uuid::Uuid::now_v7().simple())
            }
        };
        let others = inner.entries.len() - inner.leases.get(&lease_id).map_or(0, |l| l.cells.len());
        if others + a.resources.len() > cfg.max_resources {
            return Err(RegistryError::Limit(format!(
                "the broker holds its maximum of {} resources",
                cfg.max_resources
            )));
        }

        // Prices already in force cannot change; maintenance keeps when it was first announced.
        let mut warnings = Vec::new();
        let mut specs = a.resources;
        let mut p = Problems::new();
        for (i, spec) in specs.iter_mut().enumerate() {
            let rid = resource_id(&pid, &spec.key);
            if let Some(old) = inner.entries.get(&rid) {
                if let (Some(new), Some(prev)) = (&spec.pricing, &old.record.spec.pricing) {
                    new.check_not_retroactive(prev, now, &mut p, &format!("resources[{i}].pricing"));
                }
                if let Some(s) = spec.availability.schedule.as_mut() {
                    let prev = old.record.spec.availability.schedule.as_ref();
                    for m in &mut s.maintenance {
                        m.announced_at = prev
                            .and_then(|ps| ps.maintenance.iter().find(|x| x.from == m.from && x.to == m.to))
                            .and_then(|x| x.announced_at)
                            .or(Some(now));
                    }
                }
            } else if let Some(s) = spec.availability.schedule.as_mut() {
                for m in &mut s.maintenance {
                    m.announced_at = Some(now);
                }
            }
            if spec.interfaces.is_empty() {
                warnings.push(format!("resources[{i}] ({}) has no interface: consumers cannot call it", spec.key));
            }
        }
        if !p.is_empty() {
            return Err(RegistryError::Invalid(p));
        }

        // The provider record.
        let registered_at = inner.providers.get(&pid).map_or(now, |p| p.registered_at);
        let verified = who.verified || inner.verified.contains(&pid);
        let provider = Arc::new(ProviderEntry {
            id: pid.clone(),
            principal: who.principal.clone(),
            organisation_id: who.organisation_id.clone(),
            verified,
            profile: a.provider,
            source,
            origin: cfg.id.clone(),
            registered_at,
            updated_at: now,
        });
        let changed_verification = inner.providers.get(&pid).is_none_or(|p| p.verified != verified);
        inner.providers.insert(pid.clone(), provider.clone());

        // Resources the lease held before and no longer announces are removed.
        let expires_at =
            now.checked_add(SignedDuration::try_from(ttl).unwrap_or(SignedDuration::from_secs(30))).unwrap_or(now);
        // What the announcement replaces: the lease's resources, or the provider's static ones.
        let before: HashMap<String, Arc<Live>> = match source {
            Source::Lease => inner.leases.remove(&lease_id).map(|l| l.cells).unwrap_or_default(),
            _ => inner
                .entries
                .values()
                .filter(|e| e.record.provider_id == pid && e.record.source == source)
                .map(|e| (e.record.spec.key.clone(), e.live.clone()))
                .collect(),
        };
        let mut cells = HashMap::new();
        let mut ids = HashMap::new();
        let mut changed = Vec::new();
        for spec in specs {
            let rid = resource_id(&pid, &spec.key);
            let old = inner.entries.get(&rid).cloned();
            // A resource another lease of the same provider held moves to this one.
            if let Some(o) = &old
                && let Some(other) = o.record.lease_id.as_ref().filter(|l| **l != lease_id)
                && let Some(l) = inner.leases.get_mut(other)
            {
                l.cells.remove(&spec.key);
                l.info.resources.retain(|r| *r != rid);
            }
            let live = before.get(&spec.key).cloned().unwrap_or_else(|| {
                Arc::new(match source {
                    Source::Lease => Live::leased(expires_at, spec.availability.status),
                    _ => Live::permanent(spec.availability.status),
                })
            });
            if source == Source::Lease {
                live.extend(expires_at, now);
            }
            if old
                .as_ref()
                .is_some_and(|o| o.record.spec.pricing != spec.pricing || o.record.spec.interfaces != spec.interfaces)
            {
                changed.push(rid.clone());
            }
            let record = ResourceRecord {
                id: rid.clone(),
                provider_id: pid.clone(),
                origin: cfg.id.clone(),
                source,
                lease_id: (source == Source::Lease).then(|| lease_id.clone()),
                registered_at: old.as_ref().map_or(now, |o| o.record.registered_at),
                updated_at: now,
                spec,
            };
            ids.insert(record.spec.key.clone(), rid.clone());
            cells.insert(record.spec.key.clone(), live.clone());
            inner.entries.insert(rid, Arc::new(Entry { record, provider: provider.clone(), live }));
        }
        let mut removed = Vec::new();
        for (key, _) in before.iter().filter(|(k, _)| !cells.contains_key(*k)) {
            let rid = resource_id(&pid, key);
            inner.entries.remove(&rid);
            removed.push(rid);
        }
        // Entries of this provider in other leases point at the old provider record; refresh them.
        let stale: Vec<String> = inner
            .entries
            .values()
            .filter(|e| e.record.provider_id == pid && !Arc::ptr_eq(&e.provider, &provider))
            .map(|e| e.record.id.clone())
            .collect();
        for id in stale {
            if let Some(e) = inner.entries.get(&id).cloned() {
                inner.entries.insert(
                    id,
                    Arc::new(Entry { record: e.record.clone(), provider: provider.clone(), live: e.live.clone() }),
                );
            }
        }
        let info = LeaseInfo {
            id: lease_id.clone(),
            provider_id: pid.clone(),
            principal: who.principal.clone(),
            ttl_s: ttl.as_secs(),
            created_at: now,
            expires_at,
            last_heartbeat: now,
            resources: ids.values().cloned().collect(),
        };
        if source == Source::Lease {
            inner.leases.insert(lease_id.clone(), Lease { info, cells });
        }
        drop(inner);

        if changed_verification {
            self.reputation.record(Evidence::ProviderVerified { provider: pid.clone(), verified, at: now });
        }
        for rid in changed {
            self.reputation.record(Evidence::Changed { resource: rid, provider: pid.clone(), at: now });
        }
        for rid in &removed {
            self.notify(Change::Removed {
                resource_id: rid.clone(),
                provider_id: pid.clone(),
                reason: "no longer announced".into(),
            });
        }
        for rid in ids.values() {
            self.notify(Change::Upserted { resource_id: rid.clone(), provider_id: pid.clone() });
        }
        metrics::counter!("broker_announces_total", "source" => source_label(source)).increment(1);
        self.mark_dirty();
        Ok(Announced {
            lease_id,
            provider_id: pid,
            ttl_s: ttl.as_secs(),
            heartbeat_every_s: (ttl.as_secs() / 3).max(1),
            expires_at,
            resources: ids,
            warnings,
        })
    }

    /// Keeps a lease alive and updates the live state of its resources.
    /// `principal` is the caller that must own the lease; `None` in open mode, where anyone may.
    pub fn heartbeat(
        &self,
        principal: Option<&str>,
        lease_id: &str,
        hb: Heartbeat,
    ) -> Result<Timestamp, RegistryError> {
        let now = Timestamp::now();
        let mut inner = self.inner.lock();
        let lease = inner.leases.get_mut(lease_id).ok_or_else(|| {
            RegistryError::NotFound(format!("lease {lease_id} is unknown or has expired; announce again"))
        })?;
        if principal.is_some_and(|p| p != lease.info.principal) {
            return Err(RegistryError::Forbidden(format!("lease {lease_id} belongs to another principal")));
        }
        let unknown: Vec<&String> = hb.resources.keys().filter(|k| !lease.cells.contains_key(*k)).collect();
        if !unknown.is_empty() {
            return Err(RegistryError::NotFound(format!("lease {lease_id} has no resource {:?}", unknown[0])));
        }
        let expires_at = now.checked_add(SignedDuration::from_secs(lease.info.ttl_s as i64)).unwrap_or(now);
        lease.info.expires_at = expires_at;
        lease.info.last_heartbeat = now;
        let mut flaps = Vec::new();
        for (key, cell) in &lease.cells {
            cell.extend(expires_at, now);
            let one = hb.resources.get(key);
            let status = one.and_then(|o| o.status).or(hb.status);
            if let Some(s) = status {
                if s.usable() != cell.status().usable() {
                    flaps.push(resource_id(&lease.info.provider_id, key));
                }
                cell.set_status(s);
            }
            if let Some(n) = one.and_then(|o| o.in_use).or(hb.in_use) {
                cell.set_in_use(Some(n));
            }
            if let Some(t) = one.and_then(|o| o.ttft_ms).or(hb.ttft_ms) {
                cell.set_ttft_ms(Some(t));
            }
        }
        let provider = lease.info.provider_id.clone();
        drop(inner);
        for rid in flaps {
            self.reputation.record(Evidence::StatusFlap { resource: rid, provider: provider.clone(), at: now });
        }
        metrics::counter!("broker_heartbeats_total").increment(1);
        Ok(expires_at)
    }

    /// Ends a lease at the provider's request: a clean goodbye.
    pub fn deregister(&self, principal: Option<&str>, lease_id: &str) -> Result<(), RegistryError> {
        {
            let inner = self.inner.lock();
            let lease = inner
                .leases
                .get(lease_id)
                .ok_or_else(|| RegistryError::NotFound(format!("lease {lease_id} is unknown")))?;
            if principal.is_some_and(|p| p != lease.info.principal) {
                return Err(RegistryError::Forbidden(format!("lease {lease_id} belongs to another principal")));
            }
        }
        self.end_lease(lease_id, true, "deregistered");
        Ok(())
    }

    /// Ends a lease: removes its resources and records how it ended.
    pub fn end_lease(&self, lease_id: &str, clean: bool, reason: &str) -> bool {
        let now = Timestamp::now();
        let mut inner = self.inner.lock();
        let Some(lease) = inner.leases.remove(lease_id) else { return false };
        let mut removed = Vec::new();
        for (key, cell) in &lease.cells {
            cell.expire();
            let rid = resource_id(&lease.info.provider_id, key);
            if inner.entries.get(&rid).is_some_and(|e| e.record.lease_id.as_deref() == Some(lease_id)) {
                inner.entries.remove(&rid);
                removed.push(rid);
            }
        }
        let pid = lease.info.provider_id.clone();
        let still_present = inner.entries.values().any(|e| e.record.provider_id == pid);
        if !still_present {
            inner.providers.remove(&pid);
        }
        drop(inner);
        self.reputation.record(Evidence::LeaseEnded {
            provider: pid.clone(),
            resources: removed.clone(),
            clean,
            at: now,
        });
        for rid in removed {
            self.notify(Change::Removed { resource_id: rid, provider_id: pid.clone(), reason: reason.into() });
        }
        self.mark_dirty();
        true
    }

    /// Replaces the static entries of one source with these providers and resources (the
    /// catalogue file, or entries kept in the state directory).
    pub fn put_static(&self, principal: &str, verified: bool, a: Announcement) -> Result<Announced, RegistryError> {
        let who = Announcer { principal: principal.to_string(), organisation_id: None, verified };
        self.announce(&who, a, Source::Static)
    }

    /// Removes a provider's static entries.
    pub fn remove_static(&self, provider_id: &str) -> usize {
        let mut inner = self.inner.lock();
        let ids: Vec<String> = inner
            .entries
            .values()
            .filter(|e| e.record.provider_id == provider_id && e.record.source == Source::Static)
            .map(|e| e.record.id.clone())
            .collect();
        for id in &ids {
            inner.entries.remove(id);
        }
        if !inner.entries.values().any(|e| e.record.provider_id == provider_id) {
            inner.providers.remove(provider_id);
        }
        drop(inner);
        for id in &ids {
            self.notify(Change::Removed {
                resource_id: id.clone(),
                provider_id: provider_id.into(),
                reason: "static entry removed".into(),
            });
        }
        self.mark_dirty();
        ids.len()
    }

    pub fn leases(&self) -> Vec<LeaseInfo> {
        let mut v: Vec<LeaseInfo> = self.inner.lock().leases.values().map(|l| l.info.clone()).collect();
        v.sort_by_key(|a| a.created_at);
        v
    }

    pub fn lease_of(&self, id: &str) -> Option<LeaseInfo> {
        self.inner.lock().leases.get(id).map(|l| l.info.clone())
    }

    /// Blocks a provider: ends its leases and refuses its announcements.
    pub fn block(&self, provider_id: &str) {
        let leases: Vec<String> = {
            let mut inner = self.inner.lock();
            inner.blocked.insert(provider_id.to_string());
            inner.pending.remove(provider_id);
            inner.leases.values().filter(|l| l.info.provider_id == provider_id).map(|l| l.info.id.clone()).collect()
        };
        for l in leases {
            self.end_lease(&l, true, "provider blocked");
        }
        self.notify(Change::Provider { provider_id: provider_id.into(), what: "blocked".into() });
    }

    pub fn unblock(&self, provider_id: &str) -> bool {
        let done = self.inner.lock().blocked.remove(provider_id);
        self.notify(Change::Provider { provider_id: provider_id.into(), what: "unblocked".into() });
        done
    }

    pub fn blocked(&self) -> Vec<String> {
        let mut v: Vec<String> = self.inner.lock().blocked.iter().cloned().collect();
        v.sort();
        v
    }

    pub fn approve(&self, provider_id: &str) -> bool {
        let mut inner = self.inner.lock();
        let was = inner.pending.remove(provider_id).is_some();
        inner.approved.insert(provider_id.to_string());
        drop(inner);
        self.notify(Change::Provider { provider_id: provider_id.into(), what: "approved".into() });
        was
    }

    pub fn pending(&self) -> Vec<Pending> {
        let mut v: Vec<Pending> = self.inner.lock().pending.values().cloned().collect();
        v.sort_by_key(|a| a.first_seen);
        v
    }

    /// Approved, blocked and verified providers, for the state file.
    pub fn decisions(&self) -> crate::state::Decisions {
        let inner = self.inner.lock();
        let sorted = |s: &HashSet<String>| {
            let mut v: Vec<String> = s.iter().cloned().collect();
            v.sort();
            v
        };
        crate::state::Decisions {
            approved: sorted(&inner.approved),
            blocked: sorted(&inner.blocked),
            verified: sorted(&inner.verified),
        }
    }

    pub fn restore_decisions(&self, d: crate::state::Decisions) {
        let mut inner = self.inner.lock();
        inner.approved.extend(d.approved);
        inner.blocked.extend(d.blocked);
        inner.verified.extend(d.verified);
    }

    pub fn is_blocked(&self, provider_id: &str) -> bool {
        self.inner.lock().blocked.contains(provider_id)
    }

    /// An admin vouches for a provider, or stops doing so.
    pub fn set_verified(&self, provider_id: &str, verified: bool) {
        let mut inner = self.inner.lock();
        if verified {
            inner.verified.insert(provider_id.to_string());
        } else {
            inner.verified.remove(provider_id);
        }
        if let Some(p) = inner.providers.get(provider_id).cloned() {
            let np = Arc::new(ProviderEntry { verified, updated_at: Timestamp::now(), ..(*p).clone() });
            inner.providers.insert(provider_id.to_string(), np.clone());
            let ids: Vec<String> = inner
                .entries
                .values()
                .filter(|e| e.record.provider_id == provider_id)
                .map(|e| e.record.id.clone())
                .collect();
            for id in ids {
                if let Some(e) = inner.entries.get(&id).cloned() {
                    inner.entries.insert(
                        id,
                        Arc::new(Entry { record: e.record.clone(), provider: np.clone(), live: e.live.clone() }),
                    );
                }
            }
        }
        drop(inner);
        self.reputation.record(Evidence::ProviderVerified {
            provider: provider_id.into(),
            verified,
            at: Timestamp::now(),
        });
        self.notify(Change::Provider {
            provider_id: provider_id.into(),
            what: if verified { "verified" } else { "unverified" }.into(),
        });
        self.mark_dirty();
    }

    /// Ends a lease by an admin's decision; it does not count against the provider.
    pub fn evict(&self, lease_id: &str) -> bool {
        self.end_lease(lease_id, true, "evicted by an admin")
    }

    /// Static entries, grouped by provider, as they can be written back to a file.
    pub fn static_entries(&self) -> Vec<StaticProvider> {
        let inner = self.inner.lock();
        let mut by: HashMap<String, StaticProvider> = HashMap::new();
        for e in inner.entries.values().filter(|e| e.record.source == Source::Static) {
            let p = by.entry(e.record.provider_id.clone()).or_insert_with(|| StaticProvider {
                principal: e.provider.principal.clone(),
                verified: e.provider.verified,
                provider: e.provider.profile.clone(),
                resources: Vec::new(),
            });
            p.resources.push(e.record.spec.clone());
        }
        let mut v: Vec<StaticProvider> = by.into_values().collect();
        v.sort_by(|a, b| a.principal.cmp(&b.principal));
        for p in &mut v {
            p.resources.sort_by(|a, b| a.key.cmp(&b.key));
        }
        v
    }
}

/// A provider and its resources as a catalogue file or the state directory holds them.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StaticProvider {
    /// Who the entries belong to; `static:<provider name>` by default.
    #[serde(default)]
    pub principal: String,
    /// Whether the broker vouches for the provider.
    #[serde(default)]
    pub verified: bool,
    pub provider: ProviderProfile,
    #[serde(default)]
    pub resources: Vec<ResourceSpec>,
}

impl StaticProvider {
    pub fn principal(&self) -> String {
        if self.principal.is_empty() { format!("static:{}", self.provider.name) } else { self.principal.clone() }
    }
}

fn source_label(s: Source) -> &'static str {
    match s {
        Source::Lease => "lease",
        Source::Static => "static",
        Source::Import => "import",
    }
}

#[cfg(test)]
mod tests;
