//! The listeners and background tasks of a running broker.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::get;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::api;
use crate::app::App;

pub struct Server {
    pub app: Arc<App>,
    pub public_addr: SocketAddr,
    pub admin_addr: Option<SocketAddr>,
    pub metrics_addr: Option<SocketAddr>,
    stop: watch::Sender<bool>,
    servers: Vec<JoinHandle<()>>,
    tasks: Vec<JoinHandle<()>>,
}

/// Binds an address, or a host name with a port (an alias on a Docker network), resolved now.
async fn bind(what: &str, addr: &str) -> anyhow::Result<TcpListener> {
    let addrs: Vec<SocketAddr> =
        tokio::net::lookup_host(addr).await.map_err(|e| anyhow::anyhow!("{what} {addr}: {e}"))?.collect();
    let mut last = None;
    for a in addrs {
        match TcpListener::bind(a).await {
            Ok(l) => return Ok(l),
            Err(e) => last = Some(e),
        }
    }
    Err(anyhow::anyhow!("{what} {addr}: {}", last.map_or("no address".into(), |e| e.to_string())))
}

fn serve(listener: TcpListener, router: Router, mut stop: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let shutdown = async move {
            let _ = stop.wait_for(|s| *s).await;
        };
        if let Err(e) = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown)
            .await
        {
            tracing::error!(error = %e, "a listener stopped");
        }
    })
}

/// The public router with its limits and logging, for the server and for tests.
pub fn public_router(app: Arc<App>) -> Router {
    let limit = app.cfg.max_body_bytes;
    api::public::router(app).layer(axum::middleware::from_fn(api::observe)).layer(DefaultBodyLimit::max(limit))
}

pub fn admin_router(app: Arc<App>) -> Router {
    let limit = app.cfg.max_body_bytes;
    api::admin::router(app).layer(axum::middleware::from_fn(api::observe)).layer(DefaultBodyLimit::max(limit))
}

impl Server {
    pub async fn start(app: Arc<App>) -> anyhow::Result<Self> {
        let (stop, rx) = watch::channel(false);
        let cfg = app.cfg.clone();
        let public = bind("BROKER_LISTEN", &cfg.listen).await?;
        let public_addr = public.local_addr()?;
        let mut servers = vec![serve(public, public_router(app.clone()), rx.clone())];

        let mut admin_addr = None;
        if let Some(a) = &cfg.admin_listen {
            let l = bind("BROKER_ADMIN_LISTEN", a).await?;
            admin_addr = Some(l.local_addr()?);
            servers.push(serve(l, admin_router(app.clone()), rx.clone()));
        }
        let mut metrics_addr = None;
        if let (Some(a), Some(handle)) = (&cfg.metrics_listen, app.metrics.clone()) {
            let l = bind("BROKER_METRICS_LISTEN", a).await?;
            metrics_addr = Some(l.local_addr()?);
            let r = Router::new()
                .route("/metrics", get(move || std::future::ready(handle.render())))
                .route("/healthz", get(|| async { "ok" }));
            servers.push(serve(l, r, rx.clone()));
        }

        let tasks = vec![
            tokio::spawn(app.registry.clone().run_rebuilder()),
            tokio::spawn(app.registry.clone().run_sweeper()),
            tokio::spawn(app.clone().run_reputation()),
        ];
        tracing::info!(
            public = %public_addr,
            admin = admin_addr.map(|a| a.to_string()).unwrap_or_else(|| "off".into()),
            metrics = metrics_addr.map(|a| a.to_string()).unwrap_or_else(|| "off".into()),
            auth = if app.identity.is_some() { "oidc" } else { "open" },
            broker_id = %cfg.id,
            resources = app.registry.index().len(),
            "pacod-broker is up"
        );
        Ok(Self { app, public_addr, admin_addr, metrics_addr, stop, servers, tasks })
    }

    /// Stops taking requests, finishes those running, and writes what is kept.
    pub async fn shutdown(self) {
        api::public::DRAINING.store(true, std::sync::atomic::Ordering::Relaxed);
        api::public::SHUTDOWN.send_replace(true);
        let _ = self.stop.send(true);
        for s in self.servers {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(10), s).await;
        }
        for t in self.tasks {
            t.abort();
        }
        let app = self.app.clone();
        let _ = tokio::task::spawn_blocking(move || {
            app.persist();
            app.compact();
        })
        .await;
    }

    /// Reads the catalogue file again (SIGHUP).
    pub fn reload_catalogue(&self) {
        let Some(f) = &self.app.cfg.catalog_file else {
            tracing::info!("SIGHUP: no BROKER_CATALOG_FILE to read");
            return;
        };
        match crate::state::read_catalogue(f) {
            Ok(c) => {
                let (n, problems) = crate::state::load_static(&self.app.registry, c.providers);
                for p in &problems {
                    tracing::warn!("catalogue: {p}");
                }
                tracing::info!(resources = n, refused = problems.len(), "catalogue read again");
            }
            Err(e) => tracing::error!(error = %e, "catalogue could not be read; the previous entries stay"),
        }
    }
}
