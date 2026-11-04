//! The broker's shared state: what every handler reaches.

use std::sync::Arc;

use jiff::Timestamp;
use pacod_broker_index::Scores as IndexScores;
use pacod_broker_model::Reputation;
use pacod_broker_reputation::{Engine, Scores, Settings};

use crate::auth::{AuthError, Caller};
use crate::config::{AuthMode, Config};
use crate::identity::Identity;
use crate::registry::Registry;
use crate::state::{FileSink, StateDir};
use crate::tickets::Tickets;

pub struct App {
    pub cfg: Arc<Config>,
    pub registry: Arc<Registry>,
    pub reputation: Arc<Engine>,
    pub tickets: Tickets,
    pub identity: Option<Arc<Identity>>,
    pub state: Option<StateDir>,
    pub sink: Option<Arc<FileSink>>,
    pub metrics: Option<metrics_exporter_prometheus::PrometheusHandle>,
    pub started_at: Timestamp,
    /// True once this broker stops: `/readyz` fails so load balancers stop sending, and
    /// long-lived streams (watch, admin events) end, so a graceful shutdown does not wait for
    /// watchers to hang up. Per broker, not per process: tests run several in one process.
    pub stopping: tokio::sync::watch::Sender<bool>,
}

/// The reputation scores in the form the index reads them.
pub struct ScoresView(pub Arc<Scores>);

impl IndexScores for ScoresView {
    fn resource(&self, id: &str) -> Option<&Reputation> {
        self.0.resources.get(id)
    }

    fn provider(&self, id: &str) -> Option<&Reputation> {
        self.0.providers.get(id)
    }
}

impl App {
    /// Whether this broker is stopping.
    pub fn is_stopping(&self) -> bool {
        *self.stopping.borrow()
    }

    /// A future that completes when this broker stops.
    pub fn stopped(&self) -> impl std::future::Future<Output = ()> + Send + use<> {
        let mut rx = self.stopping.subscribe();
        async move {
            let _ = rx.wait_for(|v| *v).await;
        }
    }

    /// Builds the broker's state from its configuration: opens the state directory, restores
    /// reputation and the static entries.
    pub fn build(
        cfg: Config,
        metrics: Option<metrics_exporter_prometheus::PrometheusHandle>,
    ) -> anyhow::Result<Arc<Self>> {
        let cfg = Arc::new(cfg);
        let state = cfg.state_dir.as_deref().map(StateDir::open).transpose()?;
        let sink = state.as_ref().map(StateDir::log_sink).transpose()?;
        let settings = Settings {
            half_life_days: cfg.reputation.half_life_days,
            prior_weight: cfg.reputation.prior_weight,
            provider_weights: cfg.reputation.provider_weights.clone(),
            client_weights: cfg.reputation.client_weights.clone(),
        };
        let reputation =
            Arc::new(Engine::new(settings, sink.clone().map(|s| s as Arc<dyn pacod_broker_reputation::Sink>)));
        if let Some(st) = &state {
            let (snap, log) = st.load_reputation()?;
            let (ok, bad) = reputation.restore(snap, log);
            if bad > 0 {
                tracing::warn!(
                    replayed = ok,
                    unreadable = bad,
                    "some lines of the reputation log could not be read; they were skipped"
                );
            } else {
                tracing::info!(replayed = ok, "reputation restored");
            }
            reputation.compute(Timestamp::now());
        }
        let key = match (&cfg.ticket_key, &state) {
            (Some(k), _) => k.expose().as_bytes().to_vec(),
            (None, Some(st)) => st.ticket_key()?,
            (None, None) => crate::tickets::random_key(),
        };
        let tickets = Tickets::new(&key, cfg.ticket_ttl);
        let registry = Registry::new(cfg.clone(), reputation.clone());
        if let Some(st) = &state {
            registry.restore_decisions(st.load_decisions()?);
            let (n, problems) = crate::state::load_static(&registry, st.load_static()?);
            for p in problems {
                tracing::warn!("a static entry of the state directory was refused: {p}");
            }
            if n > 0 {
                tracing::info!(resources = n, "static entries restored");
            }
        }
        if let Some(f) = &cfg.catalog_file {
            let c = crate::state::read_catalogue(f)?;
            let (n, problems) = crate::state::load_static(&registry, c.providers);
            if !problems.is_empty() {
                anyhow::bail!("the catalogue file {} has problems:\n  - {}", f.display(), problems.join("\n  - "));
            }
            tracing::info!(resources = n, file = %f.display(), "catalogue loaded");
        }
        registry.rebuild();
        let identity = match cfg.auth {
            AuthMode::Open => None,
            AuthMode::Oidc => Some(Identity::start(&cfg)?),
        };
        Ok(Arc::new(Self {
            cfg,
            registry,
            reputation,
            tickets,
            identity,
            state,
            sink,
            metrics,
            started_at: Timestamp::now(),
            stopping: tokio::sync::watch::channel(false).0,
        }))
    }

    /// Who is calling.
    pub async fn caller(&self, headers: &http::HeaderMap) -> Result<Caller, AuthError> {
        match &self.identity {
            None => Ok(Caller::Open { client: crate::auth::open_client(headers) }),
            Some(id) => match crate::auth::bearer(headers) {
                None => Ok(Caller::Anonymous),
                Some(token) => id.authenticate(token).await.map(Caller::Principal),
            },
        }
    }

    pub fn scores(&self) -> ScoresView {
        ScoresView(self.reputation.scores())
    }

    /// Records a client's identity level once per score computation, so its reputation knows
    /// who it is.
    pub fn note_client(&self, caller: &Caller) {
        if let Some(c) = caller.client_id() {
            let level = caller.identity_level();
            let known = self.reputation.scores().clients.get(&c).map(|r| r.components.get("identity").map(|x| x.score));
            if known.flatten() != Some(level) {
                self.reputation.record(pacod_broker_reputation::Evidence::ClientIdentity {
                    client: c,
                    level,
                    at: Timestamp::now(),
                });
            }
        }
    }

    /// Writes the static entries and decisions to the state directory, if there is one.
    pub fn persist(&self) {
        let Some(st) = &self.state else { return };
        if let Err(e) = st.save_static(&self.registry.static_entries()) {
            tracing::error!(error = %e, "cannot write the static entries");
        }
        if let Err(e) = st.save_decisions(&self.registry.decisions()) {
            tracing::error!(error = %e, "cannot write the provider decisions");
        }
    }

    /// Writes the reputation snapshot and restarts the log.
    pub fn compact(&self) {
        if let (Some(st), Some(sink)) = (&self.state, &self.sink)
            && let Err(e) = st.compact(&self.reputation, sink)
        {
            tracing::error!(error = %e, "cannot write the reputation snapshot");
        }
    }

    /// The reputation task: recompute every `BROKER_REPUTATION_EVERY`, compact daily.
    pub async fn run_reputation(self: Arc<Self>) {
        let mut tick = tokio::time::interval(self.cfg.reputation.every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_compact = std::time::Instant::now();
        loop {
            tick.tick().await;
            let me = self.clone();
            let started = std::time::Instant::now();
            let _ = tokio::task::spawn_blocking(move || me.reputation.compute(Timestamp::now())).await;
            metrics::histogram!("broker_reputation_compute_seconds").record(started.elapsed().as_secs_f64());
            if last_compact.elapsed() >= std::time::Duration::from_secs(86_400) {
                let me = self.clone();
                let _ = tokio::task::spawn_blocking(move || me.compact()).await;
                last_compact = std::time::Instant::now();
            }
        }
    }
}
