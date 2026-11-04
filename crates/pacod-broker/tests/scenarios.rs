//! The scenario suite (tests/scenarios, docs/testing.md) under `cargo test`, against the binary
//! this build just made, so `make verify` covers it. The load runs are not here: they measure,
//! and belong on a quiet machine (`make suite-load`).

use std::path::PathBuf;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_scenario_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_pacod-broker"));
    let work = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("scenarios");
    let only: Vec<String> = std::env::var("SCENARIO").map(|s| vec![s]).unwrap_or_default();
    let results =
        pacod_broker_suite::run_scenarios(&root.join("tests/scenarios"), &only, &bin, &work, 4).await.unwrap();
    assert!(!results.is_empty(), "no scenarios found");
    let mut failed = Vec::new();
    for r in &results {
        if !r.passed {
            let mut why = r.error.clone().unwrap_or_default();
            for s in r.steps.iter().filter(|s| !s.ok) {
                why.push_str(&format!("\n  step {}: {}", s.index, s.error.as_deref().unwrap_or("")));
            }
            if !r.broker_log_tail.is_empty() {
                why.push_str(&format!("\n  broker log:\n{}", r.broker_log_tail));
            }
            failed.push(format!("{}: {why}", r.name));
        }
    }
    assert!(failed.is_empty(), "{} of {} scenarios failed:\n{}", failed.len(), results.len(), failed.join("\n"));
}
