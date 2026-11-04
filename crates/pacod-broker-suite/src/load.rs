//! Load runs (docs/testing.md §4): many providers and consumers at once against one broker,
//! with latency percentiles per operation, the broker's memory and CPU, and targets.
//!
//! Providers each hold one lease with `resources_each` resources made from a template, and
//! heartbeat as the broker asks; with `churn`, providers drop out without a goodbye (so their
//! leases run out) and come back. Consumers run closed loops of discovery queries, and every
//! result is checked for a lease that had already run out.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use pacod_broker_client::{Announcement, BrokerClient, Heartbeat, Query};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::broker::{BrokerProcess, histogram_quantile, histogram_quantile_of, metric_sum, raise_open_files};
use crate::expect::set;
use crate::scenario::{BrokerSetup, duration};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadRun {
    pub name: String,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub broker: BrokerSetup,
    /// An announcement; its provider profile and first resource are the templates.
    pub template: String,
    pub providers: ProviderLoad,
    pub consumers: ConsumerLoad,
    #[serde(default = "warmup")]
    pub warmup: String,
    pub duration: String,
    #[serde(default)]
    pub targets: BTreeMap<String, f64>,
    #[serde(skip)]
    pub dir: PathBuf,
}

fn warmup() -> String {
    "3s".into()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderLoad {
    pub count: usize,
    #[serde(default = "one")]
    pub resources_each: usize,
    #[serde(default = "ttl")]
    pub ttl_s: u64,
    /// Kinds the resources take in turn; the template's kind when empty.
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub churn: Option<Churn>,
}

fn one() -> usize {
    1
}

fn ttl() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Churn {
    /// How long a provider stays up before it drops out: a range, `"2s..6s"`.
    pub up: String,
    /// How long it stays away (longer than the TTL, so its lease runs out).
    pub down: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerLoad {
    pub count: usize,
    pub queries: Vec<WeightedQuery>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeightedQuery {
    #[serde(default = "one")]
    pub weight: usize,
    pub query: Value,
}

impl LoadRun {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let mut r: LoadRun = serde_saphyr::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        r.dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        Ok(r)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OpStats {
    pub name: String,
    pub count: usize,
    pub per_second: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetResult {
    pub name: String,
    pub wanted: String,
    pub measured: f64,
    pub met: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadResult {
    pub name: String,
    pub about: String,
    pub scale: f64,
    pub providers: usize,
    pub resources_each: usize,
    pub consumers: usize,
    pub seconds: f64,
    pub operations: Vec<OpStats>,
    /// `None` when the broker's memory or CPU could not be read.
    pub rss_peak_mb: Option<f64>,
    pub cpu_seconds: Option<f64>,
    pub rebuild_p99_ms: Option<f64>,
    /// Discovery inside the broker (its own histogram), without HTTP and the load generator's
    /// share of the CPU.
    pub server_discover_p99_ms: Option<f64>,
    /// Heartbeats inside the broker (its HTTP histogram for the route).
    pub server_heartbeat_p99_ms: Option<f64>,
    pub leases_expired: f64,
    pub stale_hits: u64,
    pub targets: Vec<TargetResult>,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Latencies and errors per operation, recorded only while `measuring`.
#[derive(Default)]
struct Recorder {
    measuring: AtomicBool,
    ops: Mutex<BTreeMap<&'static str, (Vec<f64>, usize)>>,
}

impl Recorder {
    fn record(&self, op: &'static str, started: Instant, ok: bool) {
        if !self.measuring.load(Ordering::Relaxed) {
            return;
        }
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut ops = self.ops.lock();
        let e = ops.entry(op).or_default();
        if ok {
            e.0.push(ms);
        } else {
            e.1 += 1;
        }
    }

    fn stats(&self, seconds: f64) -> Vec<OpStats> {
        let ops = self.ops.lock();
        ops.iter()
            .map(|(name, (lat, errors))| {
                let mut v = lat.clone();
                v.sort_by(f64::total_cmp);
                let q = |p: f64| if v.is_empty() { 0.0 } else { v[((v.len() as f64 - 1.0) * p).round() as usize] };
                OpStats {
                    name: name.to_string(),
                    count: v.len(),
                    per_second: v.len() as f64 / seconds.max(0.001),
                    p50_ms: q(0.50),
                    p95_ms: q(0.95),
                    p99_ms: q(0.99),
                    max_ms: v.last().copied().unwrap_or(0.0),
                    errors: *errors,
                }
            })
            .collect()
    }
}

fn range(s: &str) -> anyhow::Result<(Duration, Duration)> {
    let (a, b) = s.split_once("..").unwrap_or((s, s));
    Ok((duration(a)?, duration(b)?))
}

/// A number from 0 to 1, the same for the same inputs: runs are repeatable.
fn spread(a: usize, b: usize) -> f64 {
    let mut x = (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (b as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x % 10_000) as f64 / 10_000.0
}

fn announcement(template: &Value, run: &LoadRun, p: usize) -> anyhow::Result<Announcement> {
    let base = template["resources"][0].clone();
    let mut resources = Vec::new();
    for j in 0..run.providers.resources_each {
        let mut r = base.clone();
        set(&mut r, "key", json!(format!("r{j}")))?;
        let name = format!("{}-{p}-{j}", base["name"].as_str().unwrap_or("resource"));
        set(&mut r, "name", json!(name))?;
        if !run.providers.kinds.is_empty() {
            set(&mut r, "kind", json!(run.providers.kinds[(p + j) % run.providers.kinds.len()]))?;
        }
        if r["pricing"]["periods"][0]["rates"]["input"].is_number() {
            set(&mut r, "pricing.periods[0].rates.input", json!((spread(p, j) * 4.0 * 1000.0).round() / 1000.0))?;
        }
        set(&mut r, "capabilities.tools", json!((p + j).is_multiple_of(2)))?;
        resources.push(r);
    }
    let mut a =
        json!({ "ttl_s": run.providers.ttl_s, "provider": template["provider"].clone(), "resources": resources });
    set(&mut a, "provider.name", json!(format!("load-{p}")))?;
    Ok(serde_json::from_value(a)?)
}

pub async fn run_load(run: &LoadRun, bin: &Path, work: &Path, scale: f64) -> LoadResult {
    let providers = ((run.providers.count as f64 * scale).round() as usize).max(1);
    let consumers = ((run.consumers.count as f64 * scale).round() as usize).max(1);
    let length = duration(&run.duration)
        .map(|d| d.mul_f64(scale.max(0.05)).max(Duration::from_secs(2)))
        .unwrap_or(Duration::from_secs(10));
    let mut result = LoadResult {
        name: run.name.clone(),
        about: run.about.clone(),
        scale,
        providers,
        resources_each: run.providers.resources_each,
        consumers,
        seconds: length.as_secs_f64(),
        operations: Vec::new(),
        rss_peak_mb: None,
        cpu_seconds: None,
        rebuild_p99_ms: None,
        server_discover_p99_ms: None,
        server_heartbeat_p99_ms: None,
        leases_expired: 0.0,
        stale_hits: 0,
        targets: Vec::new(),
        passed: false,
        error: None,
    };
    match drive(run, bin, work, providers, consumers, length, &mut result).await {
        Ok(()) => {
            evaluate(run, &mut result);
        }
        Err(e) => result.error = Some(format!("{e:#}")),
    }
    result
}

async fn drive(
    run: &LoadRun,
    bin: &Path,
    work: &Path,
    providers: usize,
    consumers: usize,
    length: Duration,
    result: &mut LoadResult,
) -> anyhow::Result<()> {
    let template: Value = serde_saphyr::from_str(&std::fs::read_to_string(run.dir.join(&run.template))?)
        .map_err(|e| anyhow::anyhow!("{}: {e}", run.template))?;
    let mut env = run.broker.env.clone();
    env.entry("BROKER_LEASE_TTL_MIN".into()).or_insert_with(|| "2".into());
    env.entry("RUST_LOG".into()).or_insert_with(|| "warn".into());
    let max = (providers * run.providers.resources_each * 2).max(100_000);
    env.entry("BROKER_MAX_RESOURCES".into()).or_insert_with(|| max.to_string());
    // Every provider and consumer holds a connection, on the runner's side and on the broker's,
    // which inherits the runner's limit. Short of descriptors, new connections fail while the
    // ones already open go on, so the run would measure nonsense rather than fail.
    let needed = (providers + consumers + 64) as u64;
    if let Some(limit) = raise_open_files() {
        anyhow::ensure!(
            limit >= needed,
            "the open-file limit is {limit}, and this run needs about {needed} (a connection per provider and consumer); \
             raise it (ulimit -n {}) or run at a smaller --scale",
            needed.next_power_of_two()
        );
    }
    let _ = std::fs::remove_dir_all(work);
    let broker = BrokerProcess::start(bin, env, work, false).await?;
    let client = BrokerClient::new(&broker.public)?;
    let rec = Arc::new(Recorder::default());
    let stop = Arc::new(AtomicBool::new(false));
    let stale = Arc::new(AtomicU64::new(0));
    let churn = run
        .providers
        .churn
        .as_ref()
        .map(|c| Ok::<_, anyhow::Error>((range(&c.up)?, duration(&c.down)?)))
        .transpose()?;

    // Providers: announce, then heartbeat; with churn, drop out and come back.
    let mut tasks = Vec::new();
    for p in 0..providers {
        let a = announcement(&template, run, p)?;
        let (client, rec, stop) = (client.clone(), rec.clone(), stop.clone());
        let name = a.provider.name.clone();
        tasks.push(tokio::spawn(async move {
            let mut round = 0usize;
            'life: while !stop.load(Ordering::Relaxed) {
                let t = Instant::now();
                let announced = match client.announce(&a).await {
                    Ok(x) => {
                        rec.record("announce", t, true);
                        x
                    }
                    Err(_) => {
                        rec.record("announce", t, false);
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        continue;
                    }
                };
                let every = Duration::from_secs(announced.heartbeat_every_s.max(1));
                // Providers start their heartbeats spread over the interval, as real ones would.
                tokio::time::sleep(every.mul_f64(spread(p, round))).await;
                let up_for = churn.map(|((lo, hi), _)| lo + (hi.saturating_sub(lo)).mul_f64(spread(p, round + 7)));
                let born = Instant::now();
                loop {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    if let (Some(up), Some(((_, _), down))) = (up_for, churn)
                        && born.elapsed() >= up
                    {
                        // A crash: no goodbye; the lease runs out while away.
                        tokio::time::sleep(down).await;
                        round += 1;
                        continue 'life;
                    }
                    let t = Instant::now();
                    match client.heartbeat(&announced.lease_id, &Heartbeat::default(), Some(&name)).await {
                        Ok(()) => rec.record("heartbeat", t, true),
                        Err(e) if e.is_not_found() => {
                            rec.record("heartbeat", t, false);
                            continue 'life;
                        }
                        Err(_) => rec.record("heartbeat", t, false),
                    }
                    tokio::time::sleep(every).await;
                }
            }
        }));
    }

    // Consumers: closed loops over the weighted queries.
    let mut queries: Vec<Query> = Vec::new();
    for w in &run.consumers.queries {
        let q: Query = serde_json::from_value(w.query.clone()).map_err(|e| anyhow::anyhow!("query: {e}"))?;
        for _ in 0..w.weight.max(1) {
            queries.push(q.clone());
        }
    }
    anyhow::ensure!(!queries.is_empty(), "consumers need at least one query");
    let queries = Arc::new(queries);
    for c in 0..consumers {
        let (client, rec, stop, stale, queries) = (
            client.clone().with_client_name(format!("load-consumer-{c}")),
            rec.clone(),
            stop.clone(),
            stale.clone(),
            queries.clone(),
        );
        tasks.push(tokio::spawn(async move {
            let mut i = c;
            while !stop.load(Ordering::Relaxed) {
                let q = &queries[i % queries.len()];
                i += 1;
                let t = Instant::now();
                match client.discover(q).await {
                    Ok(r) => {
                        rec.record("discover", t, true);
                        // A lease that ran out more than a second before the answer must not be in it.
                        let limit = jiff::Timestamp::now()
                            .checked_sub(jiff::SignedDuration::from_secs(1))
                            .unwrap_or(jiff::Timestamp::now());
                        let n = r.hits.iter().filter(|h| h.live.expires_at.is_some_and(|e| e < limit)).count();
                        stale.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    Err(_) => rec.record("discover", t, false),
                }
            }
        }));
    }

    // Warm up, then measure.
    tokio::time::sleep(duration(&run.warmup)?).await;
    let cpu0 = broker.cpu_seconds();
    rec.measuring.store(true, Ordering::Relaxed);
    let began = Instant::now();
    let mut rss_peak: Option<f64> = None;
    while began.elapsed() < length {
        if let Some(mb) = broker.rss_mb() {
            rss_peak = Some(rss_peak.map_or(mb, |p| p.max(mb)));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    rec.measuring.store(false, Ordering::Relaxed);
    let seconds = began.elapsed().as_secs_f64();
    let cpu1 = broker.cpu_seconds();
    stop.store(true, Ordering::Relaxed);
    // Without the broker's own histograms the targets inside it cannot be judged: say why.
    let mut problems = Vec::new();
    let metrics = broker.try_metrics_text().await.unwrap_or_else(|e| {
        problems.push(format!("the broker's /metrics could not be read: {e:#}"));
        String::new()
    });
    if rss_peak.is_none() || cpu0.is_none() || cpu1.is_none() {
        problems.push("the broker's memory and CPU could not be read (neither /proc nor ps answered)".to_string());
    }
    // Kept next to the broker's log, for a closer look at a run.
    let _ = std::fs::write(work.join("metrics.txt"), &metrics);
    for t in tasks {
        t.abort();
    }

    result.seconds = seconds;
    result.operations = rec.stats(seconds);
    result.rss_peak_mb = rss_peak;
    result.cpu_seconds = cpu0.zip(cpu1).map(|(a, b)| b - a);
    if !problems.is_empty() {
        result.error = Some(problems.join("; "));
    }
    result.rebuild_p99_ms = histogram_quantile(&metrics, "broker_index_rebuild_seconds", 0.99).map(|s| s * 1000.0);
    result.server_discover_p99_ms = histogram_quantile(&metrics, "broker_discover_seconds", 0.99).map(|s| s * 1000.0);
    result.server_heartbeat_p99_ms =
        histogram_quantile_of(&metrics, "broker_http_request_seconds", "route=\"/v1/leases/{id}/heartbeat\"", 0.99)
            .map(|s| s * 1000.0);
    result.leases_expired = metric_sum(&metrics, "broker_leases_expired_total").unwrap_or(0.0);
    result.stale_hits = stale.load(Ordering::Relaxed);
    Ok(())
}

fn evaluate(run: &LoadRun, r: &mut LoadResult) {
    let op = |name: &str| r.operations.iter().find(|o| o.name == name).cloned();
    let mut out = Vec::new();
    for (name, wanted) in &run.targets {
        let (measured, at_most) = match name.as_str() {
            "discover_p99_ms" => (op("discover").map_or(f64::NAN, |o| o.p99_ms), true),
            "heartbeat_p99_ms" => (op("heartbeat").map_or(f64::NAN, |o| o.p99_ms), true),
            "announce_p99_ms" => (op("announce").map_or(f64::NAN, |o| o.p99_ms), true),
            "discover_per_second" => (op("discover").map_or(0.0, |o| o.per_second), false),
            "errors" => (
                r.operations
                    .iter()
                    .filter(|o| o.name != "heartbeat" || run.providers.churn.is_none())
                    .map(|o| o.errors as f64)
                    .sum(),
                true,
            ),
            "rss_mb" => (r.rss_peak_mb.unwrap_or(f64::NAN), true),
            "rebuild_p99_ms" => (r.rebuild_p99_ms.unwrap_or(0.0), true),
            "server_discover_p99_ms" => (r.server_discover_p99_ms.unwrap_or(f64::NAN), true),
            "server_heartbeat_p99_ms" => (r.server_heartbeat_p99_ms.unwrap_or(f64::NAN), true),
            "leases_expired" => (r.leases_expired, true),
            "stale_hits" => (r.stale_hits as f64, true),
            other => {
                out.push(TargetResult {
                    name: other.into(),
                    wanted: "unknown target".into(),
                    measured: f64::NAN,
                    met: false,
                });
                continue;
            }
        };
        let met = if at_most { measured <= *wanted } else { measured >= *wanted };
        out.push(TargetResult {
            name: name.clone(),
            wanted: format!("{} {wanted}", if at_most { "≤" } else { "≥" }),
            measured,
            met,
        });
    }
    r.passed = r.error.is_none() && out.iter().all(|t| t.met);
    r.targets = out;
}
