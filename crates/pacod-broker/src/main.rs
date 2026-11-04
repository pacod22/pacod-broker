use std::path::PathBuf;

use clap::{Parser, Subcommand};
use mimalloc::MiMalloc;

use pacod_broker::app::App;
use pacod_broker::config;
use pacod_broker::env::Vars;
use pacod_broker::server::Server;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Parser)]
#[command(
    name = "pacod-broker",
    version,
    about = "Discovery of AI and compute resources: providers announce, consumers discover, reputation for both. No database, no administration needed."
)]
struct Cli {
    /// An env file to read; variables of the process win over it.
    #[arg(long, global = true, env = "BROKER_ENV_FILE")]
    env_file: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the broker (the default).
    Serve,
    /// Read the configuration and the catalogue file and say what they hold, without any
    /// secret. Exits with 1 when the broker would refuse to start.
    Check,
    /// Print the OpenAPI document of both APIs.
    Openapi,
    /// Print the JSON Schema of an announcement.
    Schema,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match run(cli.command.unwrap_or(Command::Serve), cli.env_file) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn read(env_file: Option<&std::path::Path>) -> anyhow::Result<(Vars, config::Config)> {
    let vars = Vars::load(env_file)?;
    match config::read(&vars) {
        Ok(cfg) => Ok((vars, cfg)),
        // Said before logging is set up: the log format may be among the mistakes.
        Err(e) => anyhow::bail!("{e}"),
    }
}

fn run(command: Command, env_file: Option<PathBuf>) -> anyhow::Result<()> {
    match command {
        Command::Serve => {
            let (vars, cfg) = read(env_file.as_deref())?;
            let settings = pacod_broker_logging::Settings::new("pacod-broker", "info")
                .service(vars.get(pacod_broker_logging::SERVICE_VAR))
                .level(vars.get(pacod_broker_logging::LEVEL_VAR))
                .format("BROKER_LOG_FORMAT", vars.get("BROKER_LOG_FORMAT"));
            pacod_broker_logging::install(settings, std::io::stdout);
            for w in &cfg.warnings {
                tracing::warn!("{w}");
            }
            let workers = std::thread::available_parallelism().map_or(2, usize::from);
            // Announcements, rebuilds and reputation runs go to the blocking pool, and every
            // announcement takes the registry's lock, so threads beyond a few per core only wait.
            // Tokio's default of 512 let a burst of announcements start hundreds of threads, each
            // with its own allocator heap, and kept rebuilds queued behind them.
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(workers)
                .max_blocking_threads((workers * 2).max(4))
                .enable_all()
                .build()?;
            runtime.block_on(async {
                let metrics = if cfg.metrics_listen.is_some() { Some(pacod_broker::metrics::install()?) } else { None };
                let app = App::build(cfg, metrics)?;
                let server = Server::start(app).await?;
                wait_for_signal(&server).await;
                tracing::info!("shutting down");
                server.shutdown().await;
                Ok::<_, anyhow::Error>(())
            })?;
            runtime.shutdown_timeout(std::time::Duration::from_secs(5));
            Ok(())
        }
        Command::Check => {
            let (_, cfg) = read(env_file.as_deref())?;
            let mut summary = cfg.summary();
            if let Some(f) = &cfg.catalog_file {
                let c = pacod_broker::state::read_catalogue(f)?;
                let mut problems = Vec::new();
                let mut n = 0;
                for p in &c.providers {
                    let mut pr = pacod_broker_model::Problems::new();
                    p.provider.validate(&mut pr, cfg.max_description_bytes);
                    for (i, r) in p.resources.iter().enumerate() {
                        r.validate(&mut pr, &format!("{}.resources[{i}]", p.provider.name), cfg.max_description_bytes);
                        r.validate_runs_on(
                            p.provider.inventory.as_ref(),
                            &mut pr,
                            &format!("{}.resources[{i}]", p.provider.name),
                        );
                    }
                    n += p.resources.len();
                    problems.extend(pr.list().iter().cloned());
                }
                summary["catalogue"] =
                    serde_json::json!({ "providers": c.providers.len(), "resources": n, "problems": problems });
                println!("{}", serde_json::to_string_pretty(&summary)?);
                if !problems.is_empty() {
                    anyhow::bail!("the catalogue file has {} problem(s)", problems.len());
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            }
            Ok(())
        }
        Command::Openapi => {
            println!("{}", serde_json::to_string_pretty(&pacod_broker::openapi::document())?);
            Ok(())
        }
        Command::Schema => {
            println!("{}", serde_json::to_string_pretty(&schemars::schema_for!(pacod_broker::registry::Announcement))?);
            Ok(())
        }
    }
}

async fn wait_for_signal(server: &Server) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut term), Ok(mut hup)) = (signal(SignalKind::terminate()), signal(SignalKind::hangup())) else {
            let _ = tokio::signal::ctrl_c().await;
            return;
        };
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => return,
                _ = term.recv() => return,
                _ = hup.recv() => server.reload_catalogue(),
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = server;
        let _ = tokio::signal::ctrl_c().await;
    }
}
