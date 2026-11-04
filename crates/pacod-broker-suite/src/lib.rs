//! The broker's scenario and load suite (docs/testing.md).
//!
//! Scenarios (`tests/scenarios/*.yaml`) set up a broker, providers and consumers, then run steps
//! whose answers must meet expectations. Load runs (`tests/load/*.yaml`) put many providers and
//! consumers on one broker and measure it against targets. Both drive the real broker binary
//! through the Rust client; oidc scenarios bring a mock of pacod-id's internal API.

pub mod broker;
pub mod expect;
pub mod idmock;
pub mod load;
mod oracle;
pub mod report;
pub mod run;
pub mod scenario;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::StreamExt;

pub use load::{LoadResult, LoadRun, run_load};
pub use report::Report;
pub use run::{ScenarioResult, run_scenario};
pub use scenario::Scenario;

/// The YAML files of a directory (not its subdirectories), sorted.
pub fn files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "yaml" || x == "yml"))
        .collect();
    v.sort();
    Ok(v)
}

/// Runs the scenarios of `dir` whose names contain `only` (all when empty), `jobs` at a time.
pub async fn run_scenarios(
    dir: &Path,
    only: &[String],
    bin: &Path,
    work: &Path,
    jobs: usize,
) -> anyhow::Result<Vec<ScenarioResult>> {
    // Many brokers and clients at once need more descriptors than macOS's default of 256.
    broker::raise_open_files();
    let mut list = Vec::new();
    for f in files(dir)? {
        let s = Scenario::load(&f)?;
        if only.is_empty() || only.iter().any(|o| s.name.contains(o.as_str())) {
            list.push((f, s));
        }
    }
    let bin = Arc::new(bin.to_path_buf());
    let work = Arc::new(work.to_path_buf());
    let results: Vec<ScenarioResult> = futures_util::stream::iter(list)
        .map(|(f, s)| {
            let (bin, work) = (bin.clone(), work.clone());
            async move {
                let dir = work.join(&s.name);
                run_scenario(&s, &f, &bin, &dir).await
            }
        })
        .buffer_unordered(jobs.max(1))
        .collect()
        .await;
    let mut results = results;
    results.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(results)
}

/// Runs the load runs of `dir` whose names contain `only`, one after another.
pub async fn run_loads(
    dir: &Path,
    only: &[String],
    bin: &Path,
    work: &Path,
    scale: f64,
) -> anyhow::Result<Vec<LoadResult>> {
    // Many brokers and clients at once need more descriptors than macOS's default of 256.
    broker::raise_open_files();
    let mut out = Vec::new();
    for f in files(dir)? {
        let r = LoadRun::load(&f)?;
        if only.is_empty() || only.iter().any(|o| r.name.contains(o.as_str())) {
            out.push(run_load(&r, bin, &work.join(format!("load-{}", r.name)), scale).await);
        }
    }
    Ok(out)
}

/// The broker binary: `BROKER_BIN`, else the newer of the release and debug builds. Newer, not
/// release first: a stale release build would test old code without saying so.
pub fn default_broker(root: &Path) -> PathBuf {
    if let Ok(b) = std::env::var("BROKER_BIN") {
        return PathBuf::from(b);
    }
    let candidates = [root.join("target/release/pacod-broker"), root.join("target/debug/pacod-broker")];
    candidates
        .iter()
        .filter_map(|p| Some((std::fs::metadata(p).ok()?.modified().ok()?, p.clone())))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
        .unwrap_or_else(|| candidates[1].clone())
}
