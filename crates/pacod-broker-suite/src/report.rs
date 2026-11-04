//! The suite's report: Markdown for people, JSON for CI.

use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;

use crate::load::LoadResult;
use crate::run::ScenarioResult;

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub broker: String,
    pub scenarios: Vec<ScenarioResult>,
    pub load: Vec<LoadResult>,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.scenarios.iter().all(|s| s.passed) && self.load.iter().all(|l| l.passed)
    }

    pub fn write(&self, dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("report.json"), serde_json::to_vec_pretty(self)?)?;
        std::fs::write(dir.join("report.md"), self.markdown())?;
        Ok(())
    }

    pub fn markdown(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "# pacod-broker suite\n");
        let _ = writeln!(s, "Broker: `{}`\n", self.broker);
        if !self.scenarios.is_empty() {
            let passed = self.scenarios.iter().filter(|x| x.passed).count();
            let _ = writeln!(s, "## Scenarios: {passed} of {} passed\n", self.scenarios.len());
            let _ = writeln!(s, "| Scenario | Result | Steps | Time | Broker RSS | About |\n|---|---|---|---|---|---|");
            for r in &self.scenarios {
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {:.1} s | {} | {} |",
                    r.name,
                    if r.passed { "pass" } else { "**FAIL**" },
                    r.steps.len(),
                    r.ms as f64 / 1000.0,
                    r.rss_mb.map_or("—".into(), |m| format!("{m:.0} MB")),
                    r.about.replace('|', "\\|").replace('\n', " ")
                );
            }
            for r in self.scenarios.iter().filter(|r| !r.passed) {
                let _ = writeln!(s, "\n### {} failed\n", r.name);
                if let Some(e) = &r.error {
                    let _ = writeln!(s, "{e}\n");
                }
                for st in r.steps.iter().filter(|st| !st.ok) {
                    let _ = writeln!(
                        s,
                        "Step {} `{}`:\n\n```\n{}\n```\n",
                        st.index,
                        st.step,
                        st.error.as_deref().unwrap_or("")
                    );
                }
                if !r.broker_log_tail.is_empty() {
                    let _ = writeln!(
                        s,
                        "<details><summary>Broker log</summary>\n\n```\n{}\n```\n</details>",
                        r.broker_log_tail
                    );
                }
            }
        }
        if !self.load.is_empty() {
            let passed = self.load.iter().filter(|x| x.passed).count();
            let _ = writeln!(s, "\n## Load runs: {passed} of {} met their targets\n", self.load.len());
            for l in &self.load {
                let _ = writeln!(
                    s,
                    "### {} ({})\n\n{} providers × {} resources, {} consumers, {:.0} s, scale {}\n",
                    l.name,
                    if l.passed { "pass" } else { "**FAIL**" },
                    l.providers,
                    l.resources_each,
                    l.consumers,
                    l.seconds,
                    l.scale
                );
                let _ = writeln!(
                    s,
                    "| Operation | Count | Per second | p50 ms | p95 ms | p99 ms | max ms | Errors |\n|---|---|---|---|---|---|---|---|"
                );
                for o in &l.operations {
                    let _ = writeln!(
                        s,
                        "| {} | {} | {:.0} | {:.2} | {:.2} | {:.2} | {:.2} | {} |",
                        o.name, o.count, o.per_second, o.p50_ms, o.p95_ms, o.p99_ms, o.max_ms, o.errors
                    );
                }
                let _ = writeln!(
                    s,
                    "\nBroker: RSS peak {}, CPU {}; inside the broker discovery p99 {} and heartbeat p99 {}; index rebuild p99 {}; leases expired {}; expired leases seen in results {}.\n",
                    l.rss_peak_mb.map_or("not measured".into(), |v| format!("{v:.0} MB")),
                    l.cpu_seconds.map_or("not measured".into(), |c| format!(
                        "{c:.1} s ({:.0} % of one core)",
                        100.0 * c / l.seconds.max(0.001)
                    )),
                    l.server_discover_p99_ms.map_or("—".into(), |v| format!("{v:.2} ms")),
                    l.server_heartbeat_p99_ms.map_or("—".into(), |v| format!("{v:.2} ms")),
                    l.rebuild_p99_ms.map_or("—".into(), |v| format!("{v:.1} ms")),
                    l.leases_expired,
                    l.stale_hits
                );
                let _ = writeln!(s, "| Target | Wanted | Measured | Met |\n|---|---|---|---|");
                for t in &l.targets {
                    let measured =
                        if t.measured.is_nan() { "not measured".to_string() } else { format!("{:.2}", t.measured) };
                    let _ = writeln!(
                        s,
                        "| {} | {} | {measured} | {} |",
                        t.name,
                        t.wanted,
                        if t.met { "yes" } else { "**no**" }
                    );
                }
                let _ = writeln!(s);
            }
        }
        s
    }
}
