//! broker-cli: pacod-broker from the command line, for scripts and operators.
//!
//!   broker-cli discover --kind model --capability tools --currency EUR --sort price:input
//!   broker-cli announce provider.yaml            # keeps the lease alive until Ctrl-C
//!   broker-cli report-usage --ticket tkt_… --outcome ok --ttft-ms 310
//!   broker-cli rate --ticket tkt_… --rating 5
//!   broker-cli watch
//!
//! The broker comes from --url or BROKER_URL; a pacod-id token from --token or BROKER_TOKEN; in
//! open mode a client name from --client or BROKER_CLIENT.

use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use pacod_broker_client::{
    Announcement, BrokerClient, ClientReportKind, DiscoverResponse, Hit, Outcome, Provider, ProviderEvent, Query,
    UsageReport,
};

#[derive(Parser)]
#[command(
    name = "broker-cli",
    version,
    about = "pacod-broker from the command line: discover, announce, report, watch"
)]
struct Cli {
    /// The broker's public API.
    #[arg(long, global = true, env = "BROKER_URL", default_value = "http://127.0.0.1:8060")]
    url: String,
    /// A pacod-id access token or service-account token (pidt_…).
    #[arg(long, global = true, env = "BROKER_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Open mode: the name this client gives itself, so that its tickets and reports are its own.
    #[arg(long, global = true, env = "BROKER_CLIENT")]
    client: Option<String>,
    /// Print the broker's JSON instead of a table.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Find resources.
    Discover(Box<DiscoverArgs>),
    /// One resource, with its price and availability now.
    Resource { id: String },
    /// One provider, with its inventory where visible.
    Provider { id: String },
    /// Announce from a YAML or JSON file and keep the lease alive until Ctrl-C.
    Announce {
        file: PathBuf,
        /// Seconds the lease lasts without a heartbeat.
        #[arg(long)]
        ttl: Option<u64>,
        /// Announce once and leave; the lease then runs out after its TTL.
        #[arg(long)]
        once: bool,
        /// Only check the file with the broker's own rules; send nothing.
        #[arg(long)]
        check: bool,
    },
    /// Report how a call went.
    ReportUsage {
        #[arg(long)]
        ticket: String,
        /// ok, client_error, server_error, timeout, rate_limited or refused.
        #[arg(long)]
        outcome: Outcome,
        #[arg(long)]
        ttft_ms: Option<f64>,
        #[arg(long)]
        ms_per_token: Option<f64>,
        /// What was charged for one unit of --rate.
        #[arg(long, requires = "rate")]
        price_charged: Option<f64>,
        #[arg(long)]
        rate: Option<String>,
    },
    /// Rate a resource, 1 to 5.
    Rate {
        #[arg(long)]
        ticket: String,
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        rating: u8,
        #[arg(long)]
        comment: Option<String>,
    },
    /// Providers: report about a client that presented a ticket.
    ReportClient {
        #[arg(long)]
        ticket: String,
        /// abuse, policy_violation, non_payment, no_show, late_cancel or good_standing.
        #[arg(long)]
        kind: ClientReportKind,
        #[arg(long)]
        details: Option<String>,
    },
    /// Providers: who presented a ticket.
    VerifyTicket { ticket: String },
    /// Follow changes to the catalogue.
    Watch {
        /// The generation already held.
        #[arg(long)]
        after: Option<u64>,
    },
    /// Your own reputation.
    Me,
    /// The JSON Schemas of what the API takes.
    Schema,
}

#[derive(clap::Args)]
struct DiscoverArgs {
    /// A whole query as a JSON file; the flags below are added to it.
    #[arg(long)]
    query: Option<PathBuf>,
    #[arg(long)]
    kind: Option<String>,
    /// Words to search for.
    #[arg(long, short)]
    q: Option<String>,
    #[arg(long = "tag")]
    tags: Vec<String>,
    #[arg(long = "capability")]
    capabilities: Vec<String>,
    #[arg(long = "protocol")]
    protocols: Vec<String>,
    #[arg(long = "region")]
    regions: Vec<String>,
    #[arg(long)]
    currency: Option<String>,
    /// rate=value, such as input=0.5.
    #[arg(long = "max-price")]
    max_price: Vec<String>,
    /// An instant, RFC 3339.
    #[arg(long)]
    at: Option<String>,
    /// A span, FROM..TO in RFC 3339.
    #[arg(long)]
    during: Option<String>,
    #[arg(long)]
    gpu: Option<String>,
    /// name or name@requirement, such as vllm@>=0.7.
    #[arg(long = "software")]
    software: Vec<String>,
    #[arg(long)]
    min_reputation: Option<f64>,
    /// default, relevance, name, ttft, reputation or price:<rate>.
    #[arg(long)]
    sort: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
}

impl DiscoverArgs {
    fn query(&self) -> anyhow::Result<Query> {
        let mut q: Query = match &self.query {
            Some(p) => serde_json::from_str(&std::fs::read_to_string(p).with_context(|| p.display().to_string())?)
                .with_context(|| format!("{}: not a query", p.display()))?,
            None => Query::default(),
        };
        q.kind = self.kind.clone().or(q.kind);
        q.q = self.q.clone().or(q.q);
        q.tags.extend(self.tags.iter().cloned());
        q.capabilities.extend(self.capabilities.iter().cloned());
        q.protocols.extend(self.protocols.iter().cloned());
        q.regions.extend(self.regions.iter().cloned());
        q.currency = self.currency.clone().or(q.currency);
        for m in &self.max_price {
            let (rate, v) = m.split_once('=').context("--max-price is rate=value")?;
            q.max_price.insert(rate.to_string(), v.parse().context("--max-price value")?);
        }
        if let Some(a) = &self.at {
            q.at = Some(a.parse().context("--at: RFC 3339")?);
        }
        if let Some(d) = &self.during {
            let (f, t) = d.split_once("..").context("--during is FROM..TO")?;
            q.during =
                Some(pacod_broker_model::schedule::Span::new(f.parse()?, t.parse()?).map_err(anyhow::Error::msg)?);
        }
        if let Some(g) = &self.gpu {
            q.hardware.get_or_insert_with(Default::default).gpu_model = Some(g.clone());
        }
        for s in &self.software {
            let (name, version) = match s.split_once('@') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (s.clone(), None),
            };
            q.software.push(pacod_broker_client::types::SoftwareQuery { name, version });
        }
        q.min_reputation = self.min_reputation.or(q.min_reputation);
        if let Some(s) = &self.sort {
            q.sort = s.parse().map_err(anyhow::Error::msg)?;
        }
        q.limit = self.limit.or(q.limit);
        Ok(q)
    }
}

fn print_json<T: serde::Serialize>(v: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn price(h: &Hit) -> String {
    match (&h.price, &h.price_summary) {
        (Some(p), _) if p.free => "free".into(),
        (Some(p), _) => p.rates.named().next().map_or("—".into(), |(k, v)| format!("{k} {v} {}", p.currency)),
        (None, Some(s)) => {
            s.rates.iter().next().map_or("—".into(), |(k, r)| format!("{k} avg {:.4} {}", r.avg, s.currency))
        }
        _ => "—".into(),
    }
}

fn table(r: &DiscoverResponse) {
    println!("{} found (generation {})", r.total, r.generation);
    println!("{:<34} {:<12} {:<14} {:<26} {:<11} {:>6}  ID", "RESOURCE", "KIND", "PROVIDER", "PRICE", "STATUS", "REP");
    for h in &r.hits {
        let status = if h.available { format!("{:?}", h.live.status).to_lowercase() } else { "unavailable".into() };
        let rep = h.reputation.as_ref().map_or("new".into(), |x| format!("{:.0}%", x.score * 100.0));
        println!(
            "{:<34} {:<12} {:<14} {:<26} {:<11} {:>6}  {}",
            trunc(&h.resource.spec.name, 34),
            trunc(&h.resource.spec.kind, 12),
            trunc(&h.provider.name, 14),
            trunc(&price(h), 26),
            status,
            rep,
            h.id
        );
    }
    if let Some(c) = &r.next_cursor {
        println!("more: --query with \"cursor\": \"{c}\"");
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "…" }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut broker = BrokerClient::new(&cli.url)?;
    if let Some(t) = &cli.token {
        broker = broker.with_token(t);
    } else if let Some(c) = &cli.client {
        broker = broker.with_client_name(c);
    }
    match cli.command {
        Command::Discover(args) => {
            let r = broker.discover(&args.query()?).await?;
            if cli.json {
                print_json(&r)?;
            } else {
                table(&r);
            }
        }
        Command::Resource { id } => print_json(&broker.resource(&id).await?)?,
        Command::Provider { id } => print_json(&broker.provider(&id).await?)?,
        Command::Announce { file, ttl, once, check } => {
            let text = std::fs::read_to_string(&file).with_context(|| file.display().to_string())?;
            let mut a = Announcement::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
            if ttl.is_some() {
                a.ttl_s = ttl;
            }
            if let Err(p) = a.validate(16 * 1024) {
                bail!("{} has problems:\n  - {}", file.display(), p.list().join("\n  - "));
            }
            if check {
                println!("{}: {} resources, no problem found", file.display(), a.resources.len());
                return Ok(());
            }
            if once {
                return print_json(&broker.announce(&a).await?);
            }
            let json = cli.json;
            let running = Provider::new(broker, a)
                .start_with(move |e| match e {
                    ProviderEvent::Announced(a) | ProviderEvent::Reannounced(a) if json => {
                        let _ = print_json(&a);
                    }
                    ProviderEvent::Announced(a) => {
                        eprintln!(
                            "announced {} resources under {}; heartbeat every {} s (Ctrl-C ends the lease)",
                            a.resources.len(),
                            a.lease_id,
                            a.heartbeat_every_s
                        );
                        for w in a.warnings {
                            eprintln!("warning: {w}");
                        }
                    }
                    ProviderEvent::Reannounced(a) => {
                        eprintln!("the broker had forgotten the lease; announced again under {}", a.lease_id)
                    }
                    ProviderEvent::Retrying(why) => eprintln!("retrying: {why}"),
                    ProviderEvent::Pending => eprintln!("waiting for an admin's approval"),
                })
                .await?;
            tokio::signal::ctrl_c().await?;
            running.stop().await?;
            eprintln!("lease ended");
        }
        Command::ReportUsage { ticket, outcome, ttft_ms, ms_per_token, price_charged, rate } => {
            let r = UsageReport { ticket, outcome, ttft_ms, ms_per_token, price_charged, rate, at: None };
            print_json(&broker.report_usage(&[r]).await?)?;
        }
        Command::Rate { ticket, rating, comment } => {
            print_json(&broker.rate(&ticket, rating, comment.as_deref()).await?)?
        }
        Command::ReportClient { ticket, kind, details } => {
            print_json(&broker.report_client(&ticket, kind, details.as_deref()).await?)?
        }
        Command::VerifyTicket { ticket } => print_json(&broker.verify_ticket(&ticket).await?)?,
        Command::Watch { after } => {
            let mut s = Box::pin(broker.watch(after).await?);
            while let Some(e) = s.next().await {
                println!("{}", serde_json::to_string(&e?)?);
            }
        }
        Command::Me => print_json(&broker.me().await?)?,
        Command::Schema => print_json(&broker.schema().await?)?,
    }
    Ok(())
}
