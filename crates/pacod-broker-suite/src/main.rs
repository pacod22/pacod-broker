//! broker-suite: runs the scenario and load suite against a broker binary and writes the report.
//!
//!   broker-suite                      every scenario (tests/scenarios)
//!   broker-suite --only lifecycle     scenarios whose name contains "lifecycle"
//!   broker-suite --load --scale 0.2   the load runs (tests/load), at a fifth of their size

use std::path::PathBuf;

use clap::Parser;
use pacod_broker_suite::{Report, default_broker, run_loads, run_scenarios};

#[derive(Parser)]
#[command(
    name = "broker-suite",
    version,
    about = "Scenarios and load runs of providers and consumers against pacod-broker"
)]
struct Cli {
    /// The broker binary (default: BROKER_BIN, target/release, target/debug).
    #[arg(long)]
    broker: Option<PathBuf>,
    /// The repository root.
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// Run the load runs instead of the scenarios.
    #[arg(long)]
    load: bool,
    /// Run both.
    #[arg(long, conflicts_with = "load")]
    all: bool,
    /// Only the scenarios or runs whose name contains one of these.
    #[arg(long)]
    only: Vec<String>,
    /// Scenarios at once.
    #[arg(long, default_value_t = 4)]
    jobs: usize,
    /// Load runs' size: counts and durations multiplied by this.
    #[arg(long, default_value_t = 1.0)]
    scale: f64,
    /// Where the report and the brokers' logs go.
    #[arg(long, default_value = "target/suite")]
    out: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let bin = cli.broker.clone().unwrap_or_else(|| default_broker(&cli.root));
    anyhow::ensure!(bin.exists(), "no broker binary at {} (cargo build -p pacod-broker)", bin.display());
    let bin = std::fs::canonicalize(&bin)?;
    let work = cli.out.join("work");
    let mut report = Report { broker: bin.display().to_string(), ..Report::default() };
    if !cli.load {
        report.scenarios = run_scenarios(&cli.root.join("tests/scenarios"), &cli.only, &bin, &work, cli.jobs).await?;
        for s in &report.scenarios {
            println!("{} {} ({:.1} s)", if s.passed { "pass" } else { "FAIL" }, s.name, s.ms as f64 / 1000.0);
            if !s.passed {
                if let Some(e) = &s.error {
                    println!("     {e}");
                }
                for st in s.steps.iter().filter(|x| !x.ok) {
                    println!(
                        "     step {}: {}",
                        st.index,
                        st.error.as_deref().unwrap_or("").replace('\n', "\n       ")
                    );
                }
            }
        }
    }
    if cli.load || cli.all {
        report.load = run_loads(&cli.root.join("tests/load"), &cli.only, &bin, &work, cli.scale).await?;
        for l in &report.load {
            println!(
                "{} load {} ({} providers, {} consumers, {:.0} s)",
                if l.passed { "pass" } else { "FAIL" },
                l.name,
                l.providers,
                l.consumers,
                l.seconds
            );
            if let Some(e) = &l.error {
                println!("     {e}");
            }
            for o in &l.operations {
                println!(
                    "     {:<10} {:>8} ops {:>8.0}/s  p50 {:>7.2} ms  p99 {:>7.2} ms  errors {}",
                    o.name, o.count, o.per_second, o.p50_ms, o.p99_ms, o.errors
                );
            }
            println!(
                "     broker: discover p99 {} and heartbeat p99 {} inside, rebuild p99 {}, RSS peak {}, CPU {}",
                l.server_discover_p99_ms.map_or("—".into(), |v| format!("{v:.2} ms")),
                l.server_heartbeat_p99_ms.map_or("—".into(), |v| format!("{v:.2} ms")),
                l.rebuild_p99_ms.map_or("—".into(), |v| format!("{v:.1} ms")),
                l.rss_peak_mb.map_or("—".into(), |v| format!("{v:.0} MB")),
                l.cpu_seconds.map_or("—".into(), |v| format!("{v:.1} s"))
            );
            for t in l.targets.iter().filter(|t| !t.met) {
                if t.measured.is_nan() {
                    println!("     target {} {} not measured", t.name, t.wanted);
                } else {
                    println!("     target {} {} not met: {:.2}", t.name, t.wanted, t.measured);
                }
            }
        }
    }
    report.write(&cli.out)?;
    println!("report: {}", cli.out.join("report.md").display());
    if !report.passed() {
        std::process::exit(1);
    }
    Ok(())
}
