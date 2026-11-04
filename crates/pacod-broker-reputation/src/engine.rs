//! The engine: evidence into aggregates, aggregates into scores.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use arc_swap::ArcSwap;
use jiff::Timestamp;
use pacod_broker_model::{Component, Label, Reputation};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::aggregate::{Decayed, DecayedValue};
use crate::evidence::{ClientReportKind, Evidence, Outcome, Side};

const DAY_MS: f64 = 86_400_000.0;
const HOUR_MS: i64 = 3_600_000;
/// Evidence count at which confidence reaches 1 − 1/e.
const CONFIDENCE_SCALE: f64 = 20.0;
/// Days of tenure after which tenure counts in full.
const TENURE_DAYS: f64 = 90.0;
/// Usage reports one client may count per resource per hour; more are accepted but not counted.
const USAGE_PER_HOUR: u32 = 120;

/// The engine's settings, from the broker's configuration.
#[derive(Debug, Clone)]
pub struct Settings {
    pub half_life_days: f64,
    pub prior_weight: f64,
    pub provider_weights: Vec<(String, f64)>,
    pub client_weights: Vec<(String, f64)>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            half_life_days: 30.0,
            prior_weight: 10.0,
            provider_weights: [
                ("availability", 0.20),
                ("reliability", 0.20),
                ("performance", 0.15),
                ("accuracy", 0.15),
                ("lease_discipline", 0.05),
                ("stability", 0.05),
                ("quality", 0.15),
                ("trust", 0.05),
            ]
            .map(|(k, v)| (k.to_string(), v))
            .to_vec(),
            client_weights: [
                ("identity", 0.25),
                ("conduct", 0.30),
                ("settlement", 0.20),
                ("commitment", 0.10),
                ("feedback_honesty", 0.10),
                ("tenure", 0.05),
            ]
            .map(|(k, v)| (k.to_string(), v))
            .to_vec(),
        }
    }
}

/// Where accepted evidence is written, one JSON line each.
pub trait Sink: Send + Sync {
    fn append(&self, line: &[u8]);
}

/// A sink that keeps the lines, for tests.
#[derive(Default)]
pub struct MemorySink(pub Mutex<Vec<Vec<u8>>>);

impl Sink for MemorySink {
    fn append(&self, line: &[u8]) {
        self.0.lock().push(line.to_vec());
    }
}

/// The aggregates of one subject.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Subject {
    /// Component → decayed observations.
    pub parts: BTreeMap<String, Decayed>,
    /// Observed time to first token and per-token latency (performance).
    pub ttft: DecayedValue,
    pub mpt: DecayedValue,
    /// The resource's kind, for ranking performance within it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The provider of a resource, so provider scores can gather their resources' evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
    /// Changes of prices or interfaces (stability), decayed.
    pub changes: DecayedValue,
    #[serde(default)]
    pub frozen: bool,
    /// The latest identity level of a client, or verification of a provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<f64>,
}

/// Everything the engine holds: written as the snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub resources: HashMap<String, Subject>,
    pub providers: HashMap<String, Subject>,
    pub clients: HashMap<String, Subject>,
    /// Client reports counted, by id, so an admin can take one back.
    #[serde(default)]
    pub reports: HashMap<String, (String, ClientReportKind, i64)>,
    pub written_at: Option<Timestamp>,
    /// The sequence number of the last evidence the snapshot holds; replay skips up to it.
    #[serde(default)]
    pub seq: u64,
}

/// One line of the evidence log.
#[derive(Serialize, Deserialize)]
struct Line {
    seq: u64,
    ev: Evidence,
}

/// Scores as discovery reads them.
#[derive(Debug, Clone, Default)]
pub struct Scores {
    pub resources: HashMap<String, Reputation>,
    pub providers: HashMap<String, Reputation>,
    pub clients: HashMap<String, Reputation>,
    pub computed_at: Option<Timestamp>,
}

impl Scores {
    pub fn get(&self, side: Side, id: &str) -> Option<&Reputation> {
        match side {
            Side::Resource => self.resources.get(id),
            Side::Provider => self.providers.get(id),
            Side::Client => self.clients.get(id),
        }
    }

    /// A client's (score, confidence), for providers' thresholds and rater weights.
    pub fn client(&self, id: &str) -> Option<(f64, f64)> {
        self.clients.get(id).map(|r| (r.score, r.confidence))
    }
}

struct State {
    snap: Snapshot,
    /// (rater, subject) → (hour, count), to bound how much one rater counts.
    rate: HashMap<(String, String), (i64, u32)>,
}

pub struct Engine {
    settings: Settings,
    state: Mutex<State>,
    scores: ArcSwap<Scores>,
    sink: Option<Arc<dyn Sink>>,
}

impl Engine {
    pub fn new(settings: Settings, sink: Option<Arc<dyn Sink>>) -> Self {
        Self {
            settings,
            state: Mutex::new(State { snap: Snapshot::default(), rate: HashMap::new() }),
            scores: ArcSwap::from_pointee(Scores::default()),
            sink,
        }
    }

    fn half_life_ms(&self) -> f64 {
        self.settings.half_life_days * DAY_MS
    }

    /// The scores computed last.
    pub fn scores(&self) -> Arc<Scores> {
        self.scores.load_full()
    }

    /// Restores a snapshot and replays the log lines written after it.
    pub fn restore(&self, snapshot: Option<Snapshot>, log: impl IntoIterator<Item = Vec<u8>>) -> (usize, usize) {
        let mut st = self.state.lock();
        if let Some(s) = snapshot {
            st.snap = s;
        }
        drop(st);
        let after = self.state.lock().snap.seq;
        let (mut ok, mut bad) = (0, 0);
        for line in log {
            match serde_json::from_slice::<Line>(&line) {
                Ok(l) if l.seq <= after => {}
                Ok(l) => {
                    self.apply(&l.ev, Some(l.seq));
                    ok += 1;
                }
                // A torn last line after a crash is expected; anything else is reported.
                Err(_) => bad += 1,
            }
        }
        (ok, bad)
    }

    /// The aggregates now, for the snapshot file.
    pub fn snapshot(&self) -> Snapshot {
        let mut s = self.state.lock().snap.clone();
        s.written_at = Some(Timestamp::now());
        s
    }

    /// Client reports that counted and have not been taken back: id → (client, kind, when).
    pub fn reports(&self) -> Vec<(String, String, ClientReportKind, Timestamp)> {
        let st = self.state.lock();
        let mut v: Vec<_> = st
            .snap
            .reports
            .iter()
            .map(|(id, (c, k, at))| (id.clone(), c.clone(), *k, Timestamp::from_millisecond(*at).unwrap_or_default()))
            .collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.3));
        v
    }

    /// The aggregates of one subject, for the admin API.
    pub fn subject(&self, side: Side, id: &str) -> Option<Subject> {
        let st = self.state.lock();
        let m = match side {
            Side::Resource => &st.snap.resources,
            Side::Provider => &st.snap.providers,
            Side::Client => &st.snap.clients,
        };
        m.get(id).cloned()
    }

    /// Records evidence: applies it and writes it to the log. Returns whether it counted (a
    /// report from the provider's own organisation, or beyond the rate a rater counts, does not).
    pub fn record(&self, ev: Evidence) -> bool {
        let Some(seq) = self.apply_live(&ev) else { return false };
        if let Some(sink) = &self.sink
            && let Ok(mut line) = serde_json::to_vec(&Line { seq, ev })
        {
            line.push(b'\n');
            sink.append(&line);
        }
        true
    }

    fn apply_live(&self, ev: &Evidence) -> Option<u64> {
        self.apply(ev, None)
    }

    fn rater_weight(&self, client: &str) -> f64 {
        // A rater with a good record counts up to 1.5 times, one with a poor record as little as
        // 0.5 times; a new rater counts once.
        match self.scores.load().client(client) {
            Some((score, conf)) => 1.0 + (score - 0.5) * conf,
            None => 1.0,
        }
    }

    /// Applies evidence. `replayed` holds the sequence number while replaying the log, when rate
    /// limits were already applied the first time.
    fn apply(&self, ev: &Evidence, replayed: Option<u64>) -> Option<u64> {
        let live = replayed.is_none();
        let hl = self.half_life_ms();
        let at = ev.at().as_millisecond();
        let mut guard = self.state.lock();
        let State { snap, rate } = &mut *guard;
        let mut limited = |rater: &str, subject: &str, max: u32| -> bool {
            if !live {
                return false;
            }
            let hour = at / HOUR_MS;
            let e = rate.entry((rater.to_string(), subject.to_string())).or_insert((hour, 0));
            if e.0 != hour {
                *e = (hour, 0);
            }
            e.1 += 1;
            e.1 > max
        };
        let add = |s: &mut Subject, part: &str, x: f64, w: f64| {
            s.parts.entry(part.to_string()).or_default().add(x, w, at, hl);
        };
        match ev {
            Evidence::LeaseEnded { provider, resources, clean, .. } => {
                let x = if *clean { 1.0 } else { 0.0 };
                add(touch(&mut snap.providers, provider, at), "lease_discipline", x, 1.0);
                for r in resources {
                    let s = touch(&mut snap.resources, r, at);
                    s.provider = Some(provider.clone());
                    add(s, "lease_discipline", x, 1.0);
                }
            }
            Evidence::Uptime { resource, provider, up, expected, .. } => {
                if *expected <= 0.0 {
                    return None;
                }
                let x = (up / expected).clamp(0.0, 1.0);
                // One hour of expected time counts as one observation.
                let w = expected / 3600.0;
                let s = touch(&mut snap.resources, resource, at);
                s.provider = Some(provider.clone());
                add(s, "availability", x, w);
                add(touch(&mut snap.providers, provider, at), "availability", x, w);
            }
            Evidence::StatusFlap { resource, provider, .. } => {
                add(touch(&mut snap.resources, resource, at), "lease_discipline", 0.0, 0.2);
                add(touch(&mut snap.providers, provider, at), "lease_discipline", 0.0, 0.2);
            }
            Evidence::Changed { resource, provider, .. } => {
                touch(&mut snap.resources, resource, at).changes.add(1.0, 1.0, at, hl);
                touch(&mut snap.providers, provider, at).changes.add(1.0, 1.0, at, hl);
            }
            Evidence::ProviderVerified { provider, verified, .. } => {
                touch(&mut snap.providers, provider, at).identity = Some(if *verified { 1.0 } else { 0.5 });
            }
            Evidence::ProviderIncident { provider, .. } => {
                add(touch(&mut snap.providers, provider, at), "trust", 0.0, 3.0);
            }
            Evidence::Usage {
                resource,
                provider,
                kind,
                client,
                client_org,
                provider_org,
                outcome,
                ttft_ms,
                ms_per_token,
                declared_ttft_ms,
                price_charged,
                price_listed,
                witness_org,
                ..
            } => {
                // Neither the client nor a witness may raise its own organisation's reputation.
                if (client_org.is_some() && client_org == provider_org)
                    || (witness_org.is_some() && witness_org == provider_org)
                {
                    return None;
                }
                if limited(client, resource, USAGE_PER_HOUR) {
                    return None;
                }
                let w = self.rater_weight(client);
                let rel = match outcome {
                    Outcome::Ok => Some(1.0),
                    Outcome::ClientError => None,
                    Outcome::RateLimited => Some(0.5),
                    Outcome::ServerError | Outcome::Timeout | Outcome::Refused => Some(0.0),
                };
                // Accuracy of claims: observed latency against the declared one (with a quarter
                // of slack), and the price charged against the price listed.
                let mut acc = Vec::new();
                if let (Some(obs), Some(dec)) = (ttft_ms, declared_ttft_ms)
                    && *obs > 0.0
                    && *dec > 0.0
                {
                    acc.push(if *obs <= dec * 1.25 { 1.0 } else { dec * 1.25 / obs });
                }
                if let (Some(ch), Some(li)) = (price_charged, price_listed)
                    && *ch >= 0.0
                    && *li >= 0.0
                {
                    acc.push(if *ch <= li * 1.001 + 1e-12 { 1.0 } else { li / ch });
                }
                let rate = |s: &mut Subject| {
                    if let Some(x) = rel {
                        add(s, "reliability", x, w);
                    }
                    for x in &acc {
                        add(s, "accuracy", *x, w);
                    }
                };
                rate(touch(&mut snap.providers, provider, at));
                let r = touch(&mut snap.resources, resource, at);
                rate(r);
                r.kind = Some(kind.clone());
                r.provider = Some(provider.clone());
                if *outcome == Outcome::Ok {
                    if let Some(t) = ttft_ms {
                        r.ttft.add(*t, w, at, hl);
                    }
                    if let Some(t) = ms_per_token {
                        r.mpt.add(*t, w, at, hl);
                    }
                }
                touch(&mut snap.clients, client, at);
            }
            Evidence::Feedback { resource, provider, client, client_org, provider_org, rating, .. } => {
                if client_org.is_some() && client_org == provider_org {
                    return None;
                }
                if !(1..=5).contains(rating) || limited(&format!("{client}#rating"), resource, 1) {
                    return None;
                }
                let x = (*rating as f64 - 1.0) / 4.0;
                let w = self.rater_weight(client);
                // Honesty of the rater: agreement with the consensus before this rating.
                let consensus =
                    self.scores.load().resources.get(resource).filter(|r| r.confidence >= 0.3).map(|r| r.score);
                add(touch(&mut snap.resources, resource, at), "quality", x, w);
                add(touch(&mut snap.providers, provider, at), "quality", x, w);
                let c = touch(&mut snap.clients, client, at);
                if let Some(cons) = consensus {
                    add(c, "feedback_honesty", 1.0 - (x - cons).abs(), 1.0);
                }
            }
            Evidence::ClientReport { client, provider, report, id, .. } => {
                if limited(provider, client, 3) {
                    return None;
                }
                let w = self.rater_weight(provider).max(0.5);
                let c = touch(&mut snap.clients, client, at);
                for (part, x, wt) in client_report_parts(*report) {
                    add(c, part, x, w * wt);
                }
                snap.reports.insert(id.clone(), (client.clone(), *report, at));
            }
            Evidence::ClientReportRejected { client, report, id, .. } => {
                snap.reports.remove(id)?;
                // Taking a report back: an observation of the opposite with the same weight.
                let c = touch(&mut snap.clients, client, at);
                for (part, x, wt) in client_report_parts(*report) {
                    add(c, part, 1.0 - x, wt);
                }
            }
            Evidence::ClientIdentity { client, level, .. } => {
                touch(&mut snap.clients, client, at).identity = Some(level.clamp(0.0, 1.0));
            }
            Evidence::Freeze { side, subject, frozen, .. } => {
                touch(map_of(snap, *side), subject, at).frozen = *frozen;
            }
            Evidence::Reset { side, subject, .. } => {
                map_of(snap, *side).remove(subject.as_str());
            }
        }
        snap.seq = replayed.unwrap_or(snap.seq + 1).max(snap.seq);
        Some(snap.seq)
    }

    /// Turns the aggregates into scores and swaps them in.
    pub fn compute(&self, now: Timestamp) -> Arc<Scores> {
        let hl = self.half_life_ms();
        let k = self.settings.prior_weight;
        let now_ms = now.as_millisecond();
        let prev = self.scores.load_full();
        let st = self.state.lock();

        // Performance: within each kind, the percentile of the observed latencies; lower is
        // better. Resources with no observation get the prior.
        let mut by_kind: HashMap<&str, Vec<(&str, f64)>> = HashMap::new();
        for (id, s) in &st.snap.resources {
            if let (Some(kind), Some(t)) = (s.kind.as_deref(), s.ttft.mean()) {
                let per_token = s.mpt.mean().unwrap_or(0.0);
                // One number per resource: the time to first token plus 200 tokens.
                by_kind.entry(kind).or_default().push((id.as_str(), t + 200.0 * per_token));
            }
        }
        let mut perf: HashMap<&str, (f64, f64)> = HashMap::new();
        for list in by_kind.values_mut() {
            list.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let n = list.len();
            for (rank, (id, _)) in list.iter().enumerate() {
                let x = if n == 1 { 0.5 } else { 1.0 - rank as f64 / (n - 1) as f64 };
                let w = st.snap.resources[*id].ttft.w.min(50.0);
                perf.insert(id, (x, w));
            }
        }

        let provider_weights = &self.settings.provider_weights;
        let client_weights = &self.settings.client_weights;
        let score = |s: &Subject, weights: &[(String, f64)], derived: &HashMap<&str, (f64, f64)>| -> Reputation {
            let mut components = BTreeMap::new();
            let (mut total, mut wsum, mut n) = (0.0, 0.0, 0.0);
            for (name, weight) in weights {
                let (x, samples) = match derived.get(name.as_str()) {
                    Some(&(x, samples)) => (x, samples),
                    None => {
                        let d = s.parts.get(name).map(|d| d.at(now_ms, hl)).unwrap_or_default();
                        (d.smoothed(0.5, k), d.n)
                    }
                };
                components
                    .insert(name.clone(), Component { score: round(x), weight: *weight, samples: round(samples) });
                total += weight * x;
                wsum += weight;
                n += samples;
            }
            let score = if wsum > 0.0 { total / wsum } else { 0.5 };
            let confidence = 1.0 - (-n / CONFIDENCE_SCALE).exp();
            Reputation {
                score: round(score),
                confidence: round(confidence),
                label: Label::of(score, confidence),
                components,
                computed_at: now,
                frozen: false,
            }
        };
        let tenure = |s: &Subject| -> (f64, f64) {
            let days = ((now_ms - s.first_seen).max(0) as f64) / DAY_MS;
            ((days / TENURE_DAYS).min(1.0), days.min(TENURE_DAYS) / 10.0)
        };
        let stability = |s: &Subject| -> (f64, f64) {
            let (t, samples) = tenure(s);
            let churn = s.changes.sum * 0.5f64.powf(((now_ms - s.changes.at).max(0) as f64) / hl);
            ((t + 1.0 / (1.0 + churn)) / 2.0, samples)
        };

        let mut out = Scores { computed_at: Some(now), ..Scores::default() };
        for (id, s) in &st.snap.resources {
            let mut d: HashMap<&str, (f64, f64)> = HashMap::new();
            d.insert(
                "performance",
                perf.get(id.as_str()).map_or((0.5, 0.0), |(x, w)| ((x * w + 0.5 * k) / (w + k), *w)),
            );
            d.insert("stability", stability(s));
            let provider_trust = s.provider.as_ref().and_then(|p| st.snap.providers.get(p));
            d.insert("trust", trust(provider_trust, now_ms, hl, k));
            let r = keep_if_frozen(s, prev.resources.get(id), score(s, provider_weights, &d));
            out.resources.insert(id.clone(), r);
        }
        for (id, s) in &st.snap.providers {
            // A provider's performance is the mean of its resources', weighted by evidence.
            let (mut px, mut pw) = (0.0, 0.0);
            for (rid, rs) in &st.snap.resources {
                if rs.provider.as_deref() == Some(id.as_str())
                    && let Some((x, w)) = perf.get(rid.as_str())
                {
                    px += x * w;
                    pw += w;
                }
            }
            let mut d: HashMap<&str, (f64, f64)> = HashMap::new();
            d.insert("performance", if pw > 0.0 { ((px + 0.5 * k) / (pw + k), pw) } else { (0.5, 0.0) });
            d.insert("stability", stability(s));
            d.insert("trust", trust(Some(s), now_ms, hl, k));
            let r = keep_if_frozen(s, prev.providers.get(id), score(s, provider_weights, &d));
            out.providers.insert(id.clone(), r);
        }
        for (id, s) in &st.snap.clients {
            let mut d: HashMap<&str, (f64, f64)> = HashMap::new();
            d.insert("identity", (s.identity.unwrap_or(0.0), if s.identity.is_some() { 1.0 } else { 0.0 }));
            d.insert("tenure", tenure(s));
            let r = keep_if_frozen(s, prev.clients.get(id), score(s, client_weights, &d));
            out.clients.insert(id.clone(), r);
        }
        drop(st);
        let out = Arc::new(out);
        self.scores.store(out.clone());
        out
    }
}

/// The subject `id` in `m`, created when new, with its first and last evidence times kept.
fn touch<'a>(m: &'a mut HashMap<String, Subject>, id: &str, at: i64) -> &'a mut Subject {
    let s = m.entry(id.to_string()).or_default();
    if s.first_seen == 0 || at < s.first_seen {
        s.first_seen = at;
    }
    s.last_seen = s.last_seen.max(at);
    s
}

fn map_of(snap: &mut Snapshot, side: Side) -> &mut HashMap<String, Subject> {
    match side {
        Side::Resource => &mut snap.resources,
        Side::Provider => &mut snap.providers,
        Side::Client => &mut snap.clients,
    }
}

/// How a report about a client counts: (component, observation, weight).
fn client_report_parts(r: ClientReportKind) -> Vec<(&'static str, f64, f64)> {
    match r {
        ClientReportKind::Abuse => vec![("conduct", 0.0, 2.0)],
        ClientReportKind::PolicyViolation => vec![("conduct", 0.0, 1.0)],
        ClientReportKind::NonPayment => vec![("settlement", 0.0, 2.0)],
        ClientReportKind::NoShow => vec![("commitment", 0.0, 1.0)],
        ClientReportKind::LateCancel => vec![("commitment", 0.5, 1.0)],
        ClientReportKind::GoodStanding => {
            vec![("conduct", 1.0, 0.5), ("settlement", 1.0, 0.5), ("commitment", 1.0, 0.3)]
        }
    }
}

/// Trust: whether the provider is verified, less upheld incidents.
fn trust(p: Option<&Subject>, now_ms: i64, hl: f64, k: f64) -> (f64, f64) {
    let Some(p) = p else { return (0.5, 0.0) };
    let base = p.identity.unwrap_or(0.5);
    let incidents = p.parts.get("trust").map(|d| d.at(now_ms, hl)).unwrap_or_default();
    // Incidents are observations of 0 with weight; they pull the base down.
    let x = (base * k + incidents.wx) / (k + incidents.w);
    (x, if p.identity.is_some() { 1.0 } else { 0.0 } + incidents.n)
}

fn keep_if_frozen(s: &Subject, prev: Option<&Reputation>, new: Reputation) -> Reputation {
    match (s.frozen, prev) {
        (true, Some(p)) => Reputation { frozen: true, ..p.clone() },
        (true, None) => Reputation { frozen: true, ..new },
        (false, _) => new,
    }
}

fn round(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests;
