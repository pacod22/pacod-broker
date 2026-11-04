//! Running one scenario: a broker (and in oidc mode a mock pacod-id), the providers and
//! consumers it names, and its steps in order. The first failing step ends the scenario.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use pacod_broker_client::{
    Announcement, BrokerClient, ClientReportKind, LiveHandle, Outcome, Provider, Query, Running, UsageReport,
    WatchEvent,
};
use pacod_broker_model::{LiveStatus, provider_id, resource_id};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};

use crate::broker::{BrokerProcess, metric_sum};
use crate::expect::{at, meets, set};
use crate::idmock::IdMock;
use crate::scenario::{Expect, Scenario, Step, duration};

#[derive(Debug, Clone, Serialize)]
pub struct StepResult {
    pub index: usize,
    pub step: String,
    pub ok: bool,
    pub ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    pub name: String,
    pub about: String,
    pub file: String,
    pub passed: bool,
    pub ms: u128,
    pub steps: Vec<StepResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub broker_log_tail: String,
    pub rss_mb: Option<f64>,
}

pub(crate) struct Run<'a> {
    sc: &'a Scenario,
    broker: BrokerProcess,
    id: Option<IdMock>,
    origin: String,
    work: PathBuf,
    announcements: BTreeMap<String, Value>,
    /// The pacod-id principal each provider announces as (oidc).
    principal_of_provider: HashMap<String, String>,
    providers: HashMap<String, Running>,
    live: HashMap<String, LiveHandle>,
    /// Providers whose resources should be found (started or announced, not stopped).
    present: HashSet<String>,
    consumers: HashMap<String, BrokerClient>,
    watches: HashMap<String, Arc<Mutex<Vec<WatchEvent>>>>,
    http: reqwest::Client,
}

/// Runs a scenario against the broker binary `bin`, working in `work`.
pub async fn run_scenario(sc: &Scenario, file: &Path, bin: &Path, work: &Path) -> ScenarioResult {
    let started = Instant::now();
    let mut result = ScenarioResult {
        name: sc.name.clone(),
        about: sc.about.clone(),
        file: file.display().to_string(),
        passed: false,
        ms: 0,
        steps: Vec::new(),
        error: None,
        broker_log_tail: String::new(),
        rss_mb: None,
    };
    let mut run = match Run::setup(sc, bin, work).await {
        Ok(r) => r,
        Err(e) => {
            result.error = Some(format!("setting up: {e:#}"));
            result.ms = started.elapsed().as_millis();
            return result;
        }
    };
    let mut failed = false;
    for (i, step) in sc.steps.iter().enumerate() {
        let t = Instant::now();
        let label = label(step);
        let outcome = run.step(step).await;
        let ok = outcome.is_ok();
        result.steps.push(StepResult {
            index: i + 1,
            step: label,
            ok,
            ms: t.elapsed().as_millis(),
            error: outcome.err().map(|e| format!("{e:#}")),
        });
        if !ok {
            failed = true;
            break;
        }
    }
    result.rss_mb = run.broker.rss_mb();
    if failed {
        result.broker_log_tail = run.broker.log_tail(25);
    }
    run.teardown().await;
    result.passed = !failed;
    result.ms = started.elapsed().as_millis();
    result
}

/// A short description of a step, for reports.
fn label(step: &Step) -> String {
    let v = serde_json::to_value(format!("{step:?}")).unwrap_or(Value::Null);
    let s = v.as_str().unwrap_or_default();
    let s = s.replace("Object {", "{").replace("String(", "(");
    if s.len() > 140 { format!("{}…", &s[..140]) } else { s }
}

impl<'a> Run<'a> {
    async fn setup(sc: &'a Scenario, bin: &Path, work: &Path) -> anyhow::Result<Self> {
        let mut env = sc.broker.env.clone();
        env.entry("BROKER_REBUILD_MS".into()).or_insert_with(|| "5".into());
        env.entry("BROKER_REPUTATION_EVERY".into()).or_insert_with(|| "1".into());
        env.entry("BROKER_LEASE_TTL_MIN".into()).or_insert_with(|| "2".into());
        let id = match &sc.identity {
            Some(identity) if sc.broker.is_oidc() => {
                let m = IdMock::start(identity).await?;
                env.insert("BROKER_AUTH".into(), "oidc".into());
                env.insert("BROKER_ID_ISSUER".into(), m.issuer().to_string());
                env.insert("BROKER_ID_INTERNAL_URL".into(), m.url.clone());
                env.insert("BROKER_ID_CLIENT_SECRET".into(), "a-secret-of-at-least-thirty-two-characters".into());
                Some(m)
            }
            None if sc.broker.is_oidc() => anyhow::bail!("auth: oidc needs an `identity`"),
            _ => None,
        };
        if let Some(t) = &sc.broker.admin_token {
            env.insert("BROKER_ADMIN_TOKEN".into(), t.clone());
        }
        let _ = std::fs::remove_dir_all(work);
        std::fs::create_dir_all(work)?;
        if let Some(c) = &sc.broker.catalog {
            let target = work.join("catalog.yaml");
            std::fs::copy(sc.dir.join(c), &target).map_err(|e| anyhow::anyhow!("{c}: {e}"))?;
            env.insert("BROKER_CATALOG_FILE".into(), target.display().to_string());
        }
        let origin = env.get("BROKER_ID").cloned().unwrap_or_else(|| "broker".into());
        let broker = BrokerProcess::start(bin, env, work, sc.broker.state_dir).await?;
        if let Some(m) = &id {
            // The broker serves only once its copy of the roles is current.
            let http = reqwest::Client::new();
            let mut ready = false;
            for _ in 0..200 {
                if http.get(format!("{}/readyz", broker.public)).send().await.is_ok_and(|r| r.status().is_success()) {
                    ready = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            anyhow::ensure!(ready, "the broker did not become ready with the mock pacod-id");
            anyhow::ensure!(m.catalog().is_some(), "the broker did not push its manifest");
        }
        let mut run = Run {
            sc,
            broker,
            id,
            origin,
            work: work.to_path_buf(),
            announcements: BTreeMap::new(),
            principal_of_provider: HashMap::new(),
            providers: HashMap::new(),
            live: HashMap::new(),
            present: HashSet::new(),
            consumers: HashMap::new(),
            watches: HashMap::new(),
            http: reqwest::Client::new(),
        };
        run.prepare_announcements()?;
        for c in &sc.consumers {
            let client = match (&run.id, &c.principal) {
                (Some(m), Some(p)) if c.fresh_token => {
                    BrokerClient::new(&run.broker.public)?.with_token(m.fresh_token(p)?)
                }
                _ => run.client_for(c.principal.as_deref(), Some(&c.name), c.anonymous)?,
            };
            run.consumers.insert(c.name.clone(), client);
        }
        Ok(run)
    }

    async fn teardown(&mut self) {
        for (_, r) in self.providers.drain() {
            let _ = r.abandon().await;
        }
        self.broker.stop(true).await;
    }

    fn prepare_announcements(&mut self) -> anyhow::Result<()> {
        for p in &self.sc.providers {
            let path = self.sc.dir.join(&p.from);
            let text = std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            let base: Value = serde_saphyr::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            let n = p.count.unwrap_or(1);
            for i in 0..n {
                let name = if p.count.is_some() { format!("{}-{}", p.name, i + 1) } else { p.name.clone() };
                let mut a = base.clone();
                set(&mut a, "provider.name", json!(name))?;
                for (path, v) in &p.set {
                    set(&mut a, path, v.clone())?;
                }
                for (path, (lo, hi)) in &p.vary {
                    let x = if n > 1 { lo + (hi - lo) * i as f64 / (n - 1) as f64 } else { *lo };
                    // Whole bounds give whole numbers, so counts and milliseconds stay integers.
                    let v = if lo.fract() == 0.0 && hi.fract() == 0.0 {
                        json!(x.round() as i64)
                    } else {
                        json!((x * 10_000.0).round() / 10_000.0)
                    };
                    set(&mut a, path, v)?;
                }
                // Checked now, so a broken template fails the setup, not a step.
                serde_json::from_value::<Announcement>(a.clone())
                    .map_err(|e| anyhow::anyhow!("provider {name}: {e}"))?;
                self.announcements.insert(name.clone(), a);
                if let Some(principal) = &p.principal {
                    self.principal_of_provider.insert(name, principal.clone());
                }
            }
        }
        Ok(())
    }
}

impl Run<'_> {
    fn client_for(&self, principal: Option<&str>, name: Option<&str>, anonymous: bool) -> anyhow::Result<BrokerClient> {
        let c = BrokerClient::new(&self.broker.public)?;
        if anonymous {
            return Ok(c);
        }
        match (&self.id, principal) {
            (Some(m), Some(p)) => Ok(c.with_token(m.token(p)?)),
            (Some(_), None) => anyhow::bail!("in oidc mode `{}` needs `as: <principal>`", name.unwrap_or("?")),
            (None, _) => Ok(match name {
                Some(n) => c.with_client_name(n),
                None => c,
            }),
        }
    }

    fn provider_principal(&self, name: &str) -> String {
        match (&self.id, self.principal_of_provider.get(name)) {
            (Some(m), Some(p)) => m.principal(p).map(|x| x.sub).unwrap_or_default(),
            _ if self.announcements.contains_key(name) => format!("anon:{name}"),
            _ => format!("static:{name}"),
        }
    }

    fn provider_id(&self, name: &str) -> String {
        provider_id(&self.origin, &self.provider_principal(name))
    }

    /// `provider/key` as the broker's resource id.
    fn rid(&self, reference: &str) -> anyhow::Result<String> {
        if reference.starts_with("res_") {
            return Ok(reference.to_string());
        }
        let (p, key) = reference.split_once('/').ok_or_else(|| anyhow::anyhow!("{reference:?}: provider/key"))?;
        Ok(resource_id(&self.provider_id(p), key))
    }

    fn client_id(&self, consumer: &str) -> String {
        let principal = self.sc.consumers.iter().find(|c| c.name == consumer).and_then(|c| c.principal.clone());
        match (&self.id, principal) {
            (Some(m), Some(p)) => m.principal(&p).map(|x| x.sub).unwrap_or_default(),
            _ => format!("anon:{consumer}"),
        }
    }

    fn consumer(&self, name: &str) -> anyhow::Result<&BrokerClient> {
        self.consumers.get(name).ok_or_else(|| anyhow::anyhow!("no consumer {name:?}"))
    }

    fn provider_client(&self, name: &str) -> anyhow::Result<BrokerClient> {
        let principal = self.principal_of_provider.get(name).cloned();
        self.client_for(principal.as_deref(), None, false)
    }

    fn announcement(&self, name: &str) -> anyhow::Result<Announcement> {
        let a = self.announcements.get(name).ok_or_else(|| anyhow::anyhow!("no provider {name:?}"))?;
        Ok(serde_json::from_value(a.clone())?)
    }

    fn names(&self, list: Vec<String>) -> Vec<String> {
        // `fleet` names every provider of a `count`ed group: fleet-1, fleet-2, …
        let mut out = Vec::new();
        for n in list {
            if self.announcements.contains_key(&n) {
                out.push(n);
            } else {
                let prefix = format!("{n}-");
                out.extend(self.announcements.keys().filter(|k| k.starts_with(&prefix)).cloned());
            }
        }
        out
    }

    async fn admin_call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> anyhow::Result<(u16, Value)> {
        let mut r = self.http.request(method, format!("{}/api/v1{path}", self.broker.admin));
        if let Some(m) = &self.id {
            r = r.bearer_auth(m.token("suite-admin")?);
        } else if let Some(t) = &self.sc.broker.admin_token {
            r = r.bearer_auth(t);
        }
        if let Some(b) = body {
            r = r.json(&b);
        }
        let res = r.send().await?;
        let status = res.status().as_u16();
        let v = res.json::<Value>().await.unwrap_or(Value::Null);
        Ok((status, v))
    }

    async fn ticket(&self, consumer: &str, reference: &str) -> anyhow::Result<String> {
        let id = self.rid(reference)?;
        let c = self.consumer(consumer)?;
        // An announcement reaches the index with the next rebuild, a few milliseconds later.
        let end = Instant::now() + Duration::from_secs(2);
        let view = loop {
            match c.resource(&id).await {
                Ok(v) => break v,
                Err(e) if e.status() == Some(404) && Instant::now() < end => {
                    tokio::time::sleep(Duration::from_millis(20)).await
                }
                Err(e) => anyhow::bail!("{consumer} reading {reference}: {e}"),
            }
        };
        view.ticket.ok_or_else(|| anyhow::anyhow!("{consumer} got no ticket for {reference}: is it identified?"))
    }

    /// Repeats `f` until it succeeds or `within` passes; the last failure is reported.
    async fn eventually<F, Fut>(&self, within: Option<&String>, default: Duration, mut f: F) -> anyhow::Result<()>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<()>>,
    {
        let limit = within.map(|w| duration(w)).transpose()?.unwrap_or(default);
        let end = Instant::now() + limit;
        loop {
            match f().await {
                Ok(()) => return Ok(()),
                Err(e) if Instant::now() >= end => return Err(e.context(format!("still not so after {limit:?}"))),
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    }

    fn check_status<T>(r: Result<T, pacod_broker_client::Error>, want: Option<u16>) -> anyhow::Result<Option<T>> {
        match (r, want) {
            (Ok(v), None) => Ok(Some(v)),
            (Ok(_), Some(s)) => anyhow::bail!("expected status {s}, the call succeeded"),
            (Err(e), Some(s)) if e.status() == Some(s) => Ok(None),
            (Err(e), Some(s)) => anyhow::bail!("expected status {s}, got: {e}"),
            (Err(e), None) => Err(anyhow::anyhow!("{e}")),
        }
    }

    fn check(&self, answer: &Value, e: &Expect, query: Option<&Value>) -> anyhow::Result<()> {
        let ids: Vec<String> = answer["hits"]
            .as_array()
            .map(|h| h.iter().filter_map(|x| x["id"].as_str().map(String::from)).collect())
            .unwrap_or_default();
        let show = |list: &[String]| -> String { list.iter().map(|i| self.name_of(i)).collect::<Vec<_>>().join(", ") };
        if let Some(t) = &e.total {
            meets(answer.get("total"), t).map_err(|m| anyhow::anyhow!("total: {m} (found: {})", show(&ids)))?;
        }
        if let Some(want) = &e.ids {
            let want: HashSet<String> = want.iter().map(|r| self.rid(r)).collect::<anyhow::Result<_>>()?;
            let got: HashSet<String> = ids.iter().cloned().collect();
            anyhow::ensure!(
                want == got,
                "ids: found [{}], expected [{}]",
                show(&ids),
                show(&want.into_iter().collect::<Vec<_>>())
            );
        }
        if let Some(order) = &e.order {
            let want: Vec<String> = order.iter().map(|r| self.rid(r)).collect::<anyhow::Result<_>>()?;
            anyhow::ensure!(
                ids.len() >= want.len() && ids[..want.len()] == want[..],
                "order: found [{}], expected [{}…]",
                show(&ids),
                show(&want)
            );
        }
        for r in &e.contains {
            anyhow::ensure!(ids.contains(&self.rid(r)?), "{r} is missing (found: {})", show(&ids));
        }
        for r in &e.excludes {
            anyhow::ensure!(!ids.contains(&self.rid(r)?), "{r} should not be there (found: {})", show(&ids));
        }
        for (path, want) in &e.fields {
            meets(at(answer, path), want).map_err(|m| anyhow::anyhow!("{path}: {m}"))?;
        }
        if e.oracle {
            let want = crate::oracle::expected(self, query.unwrap_or(&Value::Null))?;
            let got: HashSet<String> = ids.iter().cloned().collect();
            let total = answer["total"].as_u64().unwrap_or(0) as usize;
            anyhow::ensure!(
                total == want.len() && (ids.len() < total || got == want),
                "oracle: the broker found {total} [{}], the scenario's own filter {} [{}]",
                show(&ids),
                want.len(),
                show(&want.into_iter().collect::<Vec<_>>())
            );
        }
        Ok(())
    }

    /// A resource id as the scenario names it, for messages.
    fn name_of(&self, id: &str) -> String {
        for (name, a) in &self.announcements {
            for r in a["resources"].as_array().into_iter().flatten() {
                if let Some(k) = r["key"].as_str()
                    && resource_id(&self.provider_id(name), k) == id
                {
                    return format!("{name}/{k}");
                }
            }
        }
        id.to_string()
    }

    pub(crate) fn live_announcements(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.announcements.iter().filter(|(n, _)| self.present.contains(*n))
    }

    pub(crate) fn rid_of(&self, provider: &str, key: &str) -> String {
        resource_id(&self.provider_id(provider), key)
    }

    async fn step(&mut self, step: &Step) -> anyhow::Result<()> {
        match step {
            Step::Start(names) => {
                for n in self.names(names.list()) {
                    let p = Provider::new(self.provider_client(&n)?, self.announcement(&n)?);
                    self.live.insert(n.clone(), p.live());
                    let running = p.start().await.map_err(|e| anyhow::anyhow!("{n}: {e}"))?;
                    self.providers.insert(n.clone(), running);
                    self.present.insert(n);
                }
            }
            Step::Announce { provider, status } => {
                let r = self.provider_client(provider)?.announce(&self.announcement(provider)?).await;
                if Self::check_status(r, *status)?.is_some() {
                    self.present.insert(provider.clone());
                }
            }
            Step::Stop(names) => {
                for n in self.names(names.list()) {
                    let r = self.providers.remove(&n).ok_or_else(|| anyhow::anyhow!("{n} is not running"))?;
                    r.stop().await.map_err(|e| anyhow::anyhow!("{n}: {e}"))?;
                    self.present.remove(&n);
                }
            }
            Step::Crash(names) => {
                for n in self.names(names.list()) {
                    let r = self.providers.remove(&n).ok_or_else(|| anyhow::anyhow!("{n} is not running"))?;
                    r.abandon().await;
                    self.present.remove(&n);
                }
            }
            Step::Live { provider, resource, status, in_use, ttft_ms } => {
                let h = self.live.get(provider).ok_or_else(|| anyhow::anyhow!("{provider} was not started"))?;
                let status: Option<LiveStatus> =
                    status.as_ref().map(|s| serde_json::from_value(json!(s))).transpose()?;
                match resource {
                    Some(k) => h.set_resource(k.clone(), status, *in_use, *ttft_ms),
                    None => {
                        if let Some(s) = status {
                            h.set_status(s);
                        }
                        if let Some(n) = in_use {
                            h.set_in_use(*n);
                        }
                        if let Some(t) = ttft_ms {
                            h.set_ttft_ms(*t);
                        }
                    }
                }
            }
            Step::Discover { consumer, query, expect, status, within } => {
                let q: Query = serde_json::from_value(if query.is_null() { json!({}) } else { query.clone() })
                    .map_err(|e| anyhow::anyhow!("query: {e}"))?;
                let c = self.consumer(consumer)?.clone();
                if status.is_some() {
                    Self::check_status(c.discover(&q).await, *status)?;
                    return Ok(());
                }
                let this = &*self;
                this.eventually(within.as_ref(), Duration::from_secs(5), || {
                    let (c, q) = (c.clone(), q.clone());
                    async move {
                        let r = c.discover(&q).await.map_err(|e| anyhow::anyhow!("{e}"))?;
                        this.check(&serde_json::to_value(&r)?, expect, Some(query))
                    }
                })
                .await?;
            }
            Step::Resource { consumer, id, expect, status, within } => {
                let rid = self.rid(id)?;
                let c = self.consumer(consumer)?.clone();
                if status.is_some() {
                    Self::check_status(c.resource(&rid).await, *status)?;
                    return Ok(());
                }
                let this = &*self;
                this.eventually(within.as_ref(), Duration::from_secs(5), || {
                    let (c, rid) = (c.clone(), rid.clone());
                    async move {
                        let r = c.resource(&rid).await.map_err(|e| anyhow::anyhow!("{e}"))?;
                        this.check(&serde_json::to_value(&r)?, expect, None)
                    }
                })
                .await?;
            }
            Step::ProviderView { consumer, provider, expect, status, within } => {
                let pid = self.provider_id(provider);
                let c = self.consumer(consumer)?.clone();
                if status.is_some() {
                    Self::check_status(c.provider(&pid).await, *status)?;
                    return Ok(());
                }
                let this = &*self;
                this.eventually(within.as_ref(), Duration::from_secs(5), || {
                    let (c, pid) = (c.clone(), pid.clone());
                    async move {
                        let v = c.provider(&pid).await.map_err(|e| anyhow::anyhow!("{e}"))?;
                        this.check(&serde_json::to_value(&v)?, expect, None)
                    }
                })
                .await?;
            }
            Step::PriceFeed { consumer, query, expect, status, within } => {
                let c = self.consumer(consumer)?.clone();
                let params: Vec<(String, String)> = query.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                let feed = |c: BrokerClient, params: Vec<(String, String)>| async move {
                    let p: Vec<(&str, &str)> = params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    c.price_feed(&p).await
                };
                if status.is_some() {
                    Self::check_status(feed(c, params).await, *status)?;
                    return Ok(());
                }
                let this = &*self;
                this.eventually(within.as_ref(), Duration::from_secs(5), || {
                    let (c, params) = (c.clone(), params.clone());
                    async move {
                        let v = feed(c, params).await.map_err(|e| anyhow::anyhow!("{e}"))?;
                        this.check(&v, expect, None)
                    }
                })
                .await?;
            }
            Step::Watch { consumer } => {
                let events = Arc::new(Mutex::new(Vec::new()));
                let mut stream = Box::pin(self.consumer(consumer)?.watch(None).await?);
                let sink = events.clone();
                tokio::spawn(async move {
                    while let Some(Ok(e)) = stream.next().await {
                        sink.lock().push(e);
                    }
                });
                self.watches.insert(consumer.clone(), events);
            }
            Step::ExpectEvents { consumer, contains, within } => {
                let events =
                    self.watches.get(consumer).ok_or_else(|| anyhow::anyhow!("{consumer} is not watching"))?.clone();
                self.eventually(within.as_ref(), Duration::from_secs(5), || {
                    let events = events.clone();
                    async move {
                        let have: HashSet<String> = events
                            .lock()
                            .iter()
                            .map(|e| {
                                serde_json::to_value(e)
                                    .ok()
                                    .and_then(|v| v["type"].as_str().map(String::from))
                                    .unwrap_or_default()
                            })
                            .collect();
                        let missing: Vec<&String> = contains.iter().filter(|c| !have.contains(*c)).collect();
                        anyhow::ensure!(missing.is_empty(), "events missing: {missing:?} (have {have:?})");
                        Ok(())
                    }
                })
                .await?;
            }
            Step::Report {
                consumer,
                on,
                outcome,
                ttft_ms,
                ms_per_token,
                price_charged,
                rate,
                times,
                ticket_of,
                no_ticket,
                accepted,
                status,
            } => {
                let ticket = if *no_ticket {
                    "tkt_none.none".to_string()
                } else {
                    self.ticket(ticket_of.as_deref().unwrap_or(consumer), on).await?
                };
                let outcome: Outcome = outcome.parse().map_err(anyhow::Error::msg)?;
                let reports: Vec<UsageReport> = (0..*times)
                    .map(|_| UsageReport {
                        ticket: ticket.clone(),
                        outcome,
                        ttft_ms: *ttft_ms,
                        ms_per_token: *ms_per_token,
                        price_charged: *price_charged,
                        rate: rate.clone(),
                        at: None,
                    })
                    .collect();
                let mut counted = 0;
                // The broker takes at most 1000 reports a call.
                for chunk in reports.chunks(1000) {
                    if let Some(a) = Self::check_status(self.consumer(consumer)?.report_usage(chunk).await, *status)? {
                        counted += a.accepted;
                    }
                }
                if let Some(want) = accepted {
                    anyhow::ensure!(counted == *want, "{counted} of {times} reports counted, expected {want}");
                }
            }
            Step::Rate { consumer, on, rating, ticket_of, accepted, status } => {
                let ticket = self.ticket(ticket_of.as_deref().unwrap_or(consumer), on).await?;
                if let Some(a) =
                    Self::check_status(self.consumer(consumer)?.rate(&ticket, *rating, None).await, *status)?
                    && let Some(want) = accepted
                {
                    anyhow::ensure!(a.accepted == *want, "{} ratings counted, expected {want}", a.accepted);
                }
            }
            Step::ReportClient { provider, about, on, kind, status } => {
                let ticket = self.ticket(about, on).await?;
                let kind: ClientReportKind = kind.parse().map_err(anyhow::Error::msg)?;
                Self::check_status(
                    self.provider_client(provider)?.report_client(&ticket, kind, Some("scenario")).await,
                    *status,
                )?;
            }
            Step::Reputation { expect, within } => {
                let mut subjects = Vec::new();
                for (subject, fields) in expect {
                    let (side, who) = subject
                        .split_once(' ')
                        .ok_or_else(|| anyhow::anyhow!("{subject:?}: `resource p/key`, `provider p` or `client c`"))?;
                    let id = match side {
                        "resource" => self.rid(who)?,
                        "provider" => self.provider_id(who),
                        "client" => self.client_id(who),
                        _ => anyhow::bail!("{subject:?}: resource, provider or client"),
                    };
                    subjects.push((subject.clone(), side.to_string(), id, fields.clone()));
                }
                let this = &*self;
                this.eventually(within.as_ref(), Duration::from_secs(8), || {
                    let subjects = subjects.clone();
                    async move {
                        for (subject, side, id, fields) in subjects {
                            let (st, v) = this
                                .admin_call(reqwest::Method::GET, &format!("/reputation/{side}/{id}"), None)
                                .await?;
                            anyhow::ensure!(st == 200, "{subject}: the admin API answered {st}");
                            let rep = &v["reputation"];
                            anyhow::ensure!(!rep.is_null(), "{subject}: no reputation yet");
                            for (path, want) in &fields {
                                meets(at(rep, path), want).map_err(|m| anyhow::anyhow!("{subject} {path}: {m}"))?;
                            }
                        }
                        Ok(())
                    }
                })
                .await?;
            }
            Step::Admin { action, target, side, status, file } => {
                self.admin(action, target.as_deref(), side.as_deref(), *status, file.as_deref()).await?
            }
            Step::AdminAccess { who, token, status } => {
                let mut r = self.http.get(format!("{}/api/v1/status", self.broker.admin));
                if let Some(t) = token {
                    r = r.bearer_auth(t);
                } else if let (Some(m), Some(w)) = (&self.id, who) {
                    r = r.bearer_auth(m.token(w)?);
                }
                let got = r.send().await?.status().as_u16();
                anyhow::ensure!(got == *status, "the admin API answered {got}, expected {status}");
            }
            Step::RestartBroker { graceful } => self.broker.restart(*graceful).await?,
            Step::ReloadCatalog { file } => {
                let target = self.work.join("catalog.yaml");
                std::fs::copy(self.sc.dir.join(file), &target)?;
                self.broker.reload();
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            Step::IdDown => self.id.as_ref().ok_or_else(|| anyhow::anyhow!("not an oidc scenario"))?.down(),
            Step::IdUp => self.id.as_ref().ok_or_else(|| anyhow::anyhow!("not an oidc scenario"))?.up(),
            Step::Wait(d) => tokio::time::sleep(duration(d)?).await,
            Step::WaitFor { consumer, gone, present, within } => {
                let c = self.consumer(consumer)?.clone();
                let (target, want_gone) = match (gone, present) {
                    (Some(g), None) => (g, true),
                    (None, Some(p)) => (p, false),
                    _ => anyhow::bail!("wait_for: `gone` or `present`"),
                };
                let rid = self.rid(target)?;
                self.eventually(within.as_ref(), Duration::from_secs(10), || {
                    let (c, rid) = (c.clone(), rid.clone());
                    async move {
                        let r = c.resource(&rid).await;
                        match (r, want_gone) {
                            (Err(e), true) if e.is_not_found() => Ok(()),
                            (Ok(_), false) => Ok(()),
                            (Ok(_), true) => anyhow::bail!("{target} is still there"),
                            (Err(e), _) => anyhow::bail!("{target}: {e}"),
                        }
                    }
                })
                .await?;
            }
            Step::Metrics { expect, within } => {
                // Counters move after the fact (the sweeper runs once a second), so allow a moment.
                let this = &*self;
                this.eventually(within.as_ref(), Duration::from_secs(3), || async move {
                    let text = this.broker.metrics_text().await;
                    for (name, want) in expect {
                        let v = metric_sum(&text, name).map(|x| json!(x));
                        meets(v.as_ref(), want).map_err(|m| anyhow::anyhow!("{name}: {m}"))?;
                    }
                    Ok(())
                })
                .await?;
            }
        }
        Ok(())
    }

    async fn admin(
        &mut self,
        action: &str,
        target: Option<&str>,
        side: Option<&str>,
        status: Option<u16>,
        file: Option<&str>,
    ) -> anyhow::Result<()> {
        use reqwest::Method;
        let need = || target.ok_or_else(|| anyhow::anyhow!("admin {action}: `target` is needed"));
        let (st, v) = match action {
            "block" | "unblock" | "approve" => {
                self.admin_call(Method::POST, &format!("/providers/{}/{action}", self.provider_id(need()?)), None)
                    .await?
            }
            "verify" | "unverify" => {
                self.admin_call(
                    Method::POST,
                    &format!("/providers/{}/verify", self.provider_id(need()?)),
                    Some(json!({ "verified": action == "verify" })),
                )
                .await?
            }
            "incident" => {
                self.admin_call(
                    Method::POST,
                    &format!("/providers/{}/incident", self.provider_id(need()?)),
                    Some(json!({ "note": "scenario" })),
                )
                .await?
            }
            "evict" => {
                let pid = self.provider_id(need()?);
                let (_, leases) = self.admin_call(Method::GET, "/leases", None).await?;
                let lease = leases
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|l| l["provider_id"] == pid.as_str())
                    .and_then(|l| l["id"].as_str())
                    .map(String::from);
                let lease = lease.ok_or_else(|| anyhow::anyhow!("{} holds no lease", need().unwrap_or_default()))?;
                self.admin_call(Method::DELETE, &format!("/leases/{lease}"), None).await?
            }
            "freeze" | "unfreeze" | "reset" => {
                let side = side.unwrap_or("resource");
                let id = match side {
                    "resource" => self.rid(need()?)?,
                    "provider" => self.provider_id(need()?),
                    _ => self.client_id(need()?),
                };
                self.admin_call(Method::POST, &format!("/reputation/{side}/{id}/{action}"), None).await?
            }
            "reject_report" => {
                let client = self.client_id(need()?);
                let (_, reports) = self.admin_call(Method::GET, "/reports", None).await?;
                let id = reports["client_reports"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|r| r["client"] == client.as_str())
                    .and_then(|r| r["id"].as_str())
                    .map(String::from);
                let id = id.ok_or_else(|| anyhow::anyhow!("no counted report about {}", need().unwrap_or_default()))?;
                self.admin_call(Method::POST, &format!("/reports/{id}/reject"), None).await?
            }
            "put_static" => {
                let f = file.ok_or_else(|| anyhow::anyhow!("put_static: `file`"))?;
                let text = std::fs::read_to_string(self.sc.dir.join(f))?;
                let mut doc: Value = serde_saphyr::from_str(&text).map_err(|e| anyhow::anyhow!("{f}: {e}"))?;
                if let Some(n) = target {
                    set(&mut doc, "provider.name", json!(n))?;
                }
                if let Some(o) = doc.as_object_mut() {
                    o.remove("ttl_s");
                    o.remove("lease_id");
                }
                self.admin_call(Method::PUT, "/static", Some(doc)).await?
            }
            "delete_static" => {
                self.admin_call(Method::DELETE, &format!("/static/{}", self.provider_id(need()?)), None).await?
            }
            _ => anyhow::bail!("unknown admin action {action:?}"),
        };
        let ok = (200..300).contains(&st);
        match status {
            Some(want) => anyhow::ensure!(st == want, "admin {action}: {st}, expected {want}: {v}"),
            None => anyhow::ensure!(ok, "admin {action}: {st} {v}"),
        }
        Ok(())
    }
}
