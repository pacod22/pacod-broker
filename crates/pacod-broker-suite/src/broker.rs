//! The broker under test: the real binary as a child process, on free local ports, with the
//! scenario's settings. Restarts keep the ports and the state directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::process::{Child, Command};

pub struct BrokerProcess {
    pub bin: PathBuf,
    pub env: BTreeMap<String, String>,
    pub public: String,
    pub admin: String,
    pub metrics: String,
    pub log: PathBuf,
    pub state_dir: Option<PathBuf>,
    child: Option<Child>,
    started: Instant,
}

/// A port nobody listens on now.
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(0)
}

impl BrokerProcess {
    /// Starts the broker with `env` added to the suite's own settings, and waits until it is ready.
    pub async fn start(
        bin: &Path,
        mut env: BTreeMap<String, String>,
        work: &Path,
        state: bool,
    ) -> anyhow::Result<Self> {
        let (public, admin, metrics) = (free_port(), free_port(), free_port());
        std::fs::create_dir_all(work)?;
        let state_dir = state.then(|| work.join("state"));
        let base = [
            ("BROKER_LISTEN", format!("127.0.0.1:{public}")),
            ("BROKER_ADMIN_LISTEN", format!("127.0.0.1:{admin}")),
            ("BROKER_METRICS_LISTEN", format!("127.0.0.1:{metrics}")),
            ("BROKER_LOG_FORMAT", "text".into()),
            ("RUST_LOG", "info".into()),
            ("SERVICE_NAME", "pacod-broker".into()),
        ];
        for (k, v) in base {
            env.entry(k.into()).or_insert(v);
        }
        if let Some(d) = &state_dir {
            env.insert("BROKER_STATE_DIR".into(), d.display().to_string());
        }
        let mut p = Self {
            bin: bin.to_path_buf(),
            env,
            public: format!("http://127.0.0.1:{public}"),
            admin: format!("http://127.0.0.1:{admin}"),
            metrics: format!("http://127.0.0.1:{metrics}"),
            log: work.join("broker.log"),
            state_dir,
            child: None,
            started: Instant::now(),
        };
        p.spawn().await?;
        Ok(p)
    }

    async fn spawn(&mut self) -> anyhow::Result<()> {
        let log = std::fs::OpenOptions::new().create(true).append(true).open(&self.log)?;
        let mut cmd = Command::new(&self.bin);
        // Only the scenario's settings: the developer's own BROKER_* variables must not leak in.
        cmd.env_clear().env("PATH", std::env::var("PATH").unwrap_or_default());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd.arg("serve").stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log).kill_on_drop(true);
        self.child = Some(cmd.spawn().map_err(|e| anyhow::anyhow!("starting {}: {e}", self.bin.display()))?);
        self.started = Instant::now();
        let http = reqwest::Client::new();
        for _ in 0..200 {
            if let Some(c) = self.child.as_mut()
                && let Ok(Some(status)) = c.try_wait()
            {
                anyhow::bail!("the broker exited at start ({status}):\n{}", self.log_tail(20));
            }
            if http.get(format!("{}/healthz", self.public)).send().await.is_ok_and(|r| r.status().is_success()) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        anyhow::bail!("the broker did not start within 5 s:\n{}", self.log_tail(20))
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
    }

    fn signal(&self, sig: i32) {
        if let Some(pid) = self.pid() {
            // SAFETY: kill(2) on our own child's pid, with a valid signal number.
            unsafe {
                libc::kill(pid as i32, sig);
            }
        }
    }

    /// Stops the broker: SIGTERM (it writes what it keeps) or SIGKILL.
    pub async fn stop(&mut self, graceful: bool) {
        if let Some(mut c) = self.child.take() {
            if graceful {
                self.signal_child(&c, libc::SIGTERM);
                if tokio::time::timeout(Duration::from_secs(10), c.wait()).await.is_err() {
                    let _ = c.kill().await;
                }
            } else {
                let _ = c.kill().await;
            }
        }
    }

    fn signal_child(&self, c: &Child, sig: i32) {
        if let Some(pid) = c.id() {
            // SAFETY: as above.
            unsafe {
                libc::kill(pid as i32, sig);
            }
        }
    }

    pub async fn restart(&mut self, graceful: bool) -> anyhow::Result<()> {
        self.stop(graceful).await;
        self.spawn().await
    }

    /// SIGHUP: read the catalogue file again.
    pub fn reload(&self) {
        self.signal(libc::SIGHUP);
    }

    /// Resident memory, in MB.
    pub fn rss_mb(&self) -> Option<f64> {
        let status = std::fs::read_to_string(format!("/proc/{}/status", self.pid()?)).ok()?;
        let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
        let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kb / 1024.0)
    }

    /// CPU time used so far, in seconds (user + system).
    pub fn cpu_seconds(&self) -> Option<f64> {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", self.pid()?)).ok()?;
        // The command name is in parentheses and may hold spaces: count fields after it.
        let rest = &stat[stat.rfind(')')? + 2..];
        let f: Vec<&str> = rest.split_whitespace().collect();
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
        Some((f.get(11)?.parse::<f64>().ok()? + f.get(12)?.parse::<f64>().ok()?) / ticks)
    }

    pub fn log_tail(&self, lines: usize) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let all: Vec<&str> = text.lines().collect();
        all[all.len().saturating_sub(lines)..].join("\n")
    }

    /// The broker's /metrics, as text.
    pub async fn metrics_text(&self) -> String {
        match reqwest::get(format!("{}/metrics", self.metrics)).await {
            Ok(r) => r.text().await.unwrap_or_default(),
            Err(_) => String::new(),
        }
    }
}

impl Drop for BrokerProcess {
    fn drop(&mut self) {
        if let Some(c) = self.child.as_mut() {
            let _ = c.start_kill();
        }
    }
}

/// The sum of a metric's samples (all label sets), from Prometheus text.
pub fn metric_sum(text: &str, name: &str) -> Option<f64> {
    let mut found = false;
    let mut sum = 0.0;
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let Some((series, value)) = line.rsplit_once(' ') else { continue };
        let base = series.split('{').next().unwrap_or(series);
        if base == name {
            found = true;
            sum += value.parse::<f64>().unwrap_or(0.0);
        }
    }
    found.then_some(sum)
}

/// A quantile of a histogram (all label sets merged), from its cumulative buckets.
pub fn histogram_quantile(text: &str, name: &str, q: f64) -> Option<f64> {
    histogram_quantile_of(text, name, "", q)
}

/// A quantile of the series of a histogram whose labels contain `label` (`route="/v1/…"`).
pub fn histogram_quantile_of(text: &str, name: &str, label: &str, q: f64) -> Option<f64> {
    let mut buckets: BTreeMap<String, f64> = BTreeMap::new();
    for line in text.lines().filter(|l| l.starts_with(&format!("{name}_bucket")) && l.contains(label)) {
        let le = line.split("le=\"").nth(1)?.split('"').next()?.to_string();
        let v: f64 = line.rsplit_once(' ')?.1.parse().ok()?;
        *buckets.entry(le).or_default() += v;
    }
    let mut list: Vec<(f64, f64)> = buckets
        .into_iter()
        .map(|(le, c)| (if le == "+Inf" { f64::INFINITY } else { le.parse().unwrap_or(f64::INFINITY) }, c))
        .collect();
    list.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total = list.last()?.1;
    if total <= 0.0 {
        return None;
    }
    let want = q * total;
    let mut prev = (0.0, 0.0);
    for (le, c) in list {
        if c >= want {
            if le.is_infinite() {
                return Some(prev.0);
            }
            // Linear within the bucket, as Prometheus does.
            let span = c - prev.1;
            return Some(if span <= 0.0 { le } else { prev.0 + (le - prev.0) * (want - prev.1) / span });
        }
        prev = (le, c);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_are_read() {
        let text = "# HELP x\nbroker_x_total{a=\"1\"} 2\nbroker_x_total{a=\"2\"} 3\nh_bucket{le=\"0.1\"} 50\nh_bucket{le=\"1\"} 100\nh_bucket{le=\"+Inf\"} 100\n";
        assert_eq!(metric_sum(text, "broker_x_total"), Some(5.0));
        assert_eq!(metric_sum(text, "nothing"), None);
        let p = histogram_quantile(text, "h", 0.75).unwrap();
        assert!((p - 0.55).abs() < 1e-9, "{p}");
    }
}
