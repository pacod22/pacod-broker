//! What the broker keeps, when `BROKER_STATE_DIR` is set, and the catalogue file
//! (docs/runbook.md §4).
//!
//! | File | What |
//! |---|---|
//! | `static.json` | Static entries added through the admin API |
//! | `decisions.json` | Providers approved and blocked |
//! | `ticket.key` | The key interaction tickets are signed with |
//! | `reputation.log` | Reputation evidence, one JSON line each, appended |
//! | `reputation.snap` | The reputation aggregates, written daily and at shutdown |
//!
//! Files are replaced atomically (written beside, then renamed), so a crash leaves the old
//! version or the new one, never half of one. Leased entries are never written: providers
//! announce again after a restart.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pacod_broker_reputation::{Engine, Sink, Snapshot};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::registry::{Announcement, Registry, StaticProvider};

pub struct StateDir {
    dir: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Decisions {
    #[serde(default)]
    pub approved: Vec<String>,
    #[serde(default)]
    pub blocked: Vec<String>,
    #[serde(default)]
    pub verified: Vec<String>,
}

/// The catalogue file: static providers and their resources.
#[derive(Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Catalogue {
    #[serde(default)]
    pub providers: Vec<StaticProvider>,
}

impl StateDir {
    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(dir).map_err(|e| anyhow::anyhow!("BROKER_STATE_DIR {}: {e}", dir.display()))?;
        // Check now that the directory is writable, not at the first write hours later.
        let probe = dir.join(".probe");
        fs::write(&probe, b"")
            .map_err(|e| anyhow::anyhow!("BROKER_STATE_DIR {} is not writable: {e}", dir.display()))?;
        let _ = fs::remove_file(probe);
        Ok(Self { dir: dir.to_path_buf() })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Writes a file atomically.
    pub fn write(&self, name: &str, data: &[u8]) -> std::io::Result<()> {
        let tmp = self.path(&format!(".{name}.tmp"));
        {
            let mut f = File::create(&tmp)?;
            f.write_all(data)?;
            f.sync_all()?;
        }
        fs::rename(tmp, self.path(name))
    }

    fn read_json<T: for<'de> Deserialize<'de>>(&self, name: &str) -> anyhow::Result<Option<T>> {
        match fs::read(self.path(name)) {
            Ok(b) => {
                Ok(Some(serde_json::from_slice(&b).map_err(|e| anyhow::anyhow!("{}: {e}", self.path(name).display()))?))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(anyhow::anyhow!("{}: {e}", self.path(name).display())),
        }
    }

    pub fn ticket_key(&self) -> anyhow::Result<Vec<u8>> {
        let p = self.path("ticket.key");
        match fs::read(&p) {
            Ok(k) if k.len() >= 32 => Ok(k),
            Ok(_) => anyhow::bail!("{} is shorter than 32 bytes", p.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let k = crate::tickets::random_key();
                self.write("ticket.key", &k)?;
                Ok(k)
            }
            Err(e) => Err(anyhow::anyhow!("{}: {e}", p.display())),
        }
    }

    pub fn load_static(&self) -> anyhow::Result<Vec<StaticProvider>> {
        Ok(self.read_json("static.json")?.unwrap_or_default())
    }

    pub fn save_static(&self, entries: &[StaticProvider]) -> std::io::Result<()> {
        self.write("static.json", &serde_json::to_vec_pretty(entries).unwrap_or_default())
    }

    pub fn load_decisions(&self) -> anyhow::Result<Decisions> {
        Ok(self.read_json("decisions.json")?.unwrap_or_default())
    }

    pub fn save_decisions(&self, d: &Decisions) -> std::io::Result<()> {
        self.write("decisions.json", &serde_json::to_vec_pretty(d).unwrap_or_default())
    }

    pub fn load_reputation(&self) -> anyhow::Result<(Option<Snapshot>, Vec<Vec<u8>>)> {
        let snap: Option<Snapshot> = self.read_json("reputation.snap")?;
        let log = match File::open(self.path("reputation.log")) {
            Ok(f) => BufReader::new(f).split(b'\n').filter_map(Result::ok).filter(|l| !l.is_empty()).collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => anyhow::bail!("{}: {e}", self.path("reputation.log").display()),
        };
        Ok((snap, log))
    }

    pub fn log_sink(&self) -> anyhow::Result<Arc<FileSink>> {
        let path = self.path("reputation.log");
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        Ok(Arc::new(FileSink { path, file: Mutex::new(Some(f)) }))
    }

    /// Writes the reputation snapshot and starts the log afresh. Lines written while this runs
    /// carry sequence numbers past the snapshot's and are kept.
    pub fn compact(&self, engine: &Engine, sink: &FileSink) -> anyhow::Result<()> {
        let mut file = sink.file.lock();
        let snap = engine.snapshot();
        self.write("reputation.snap", &serde_json::to_vec(&snap)?)?;
        // Nothing is appended while the lock is held, so the log holds nothing newer than the
        // snapshot and can be started again.
        *file = None;
        let f = OpenOptions::new().create(true).write(true).truncate(true).open(&sink.path)?;
        *file = Some(OpenOptions::new().append(true).open(&sink.path).unwrap_or(f));
        Ok(())
    }
}

/// The reputation log as a file.
pub struct FileSink {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

impl Sink for FileSink {
    fn append(&self, line: &[u8]) {
        let mut f = self.file.lock();
        if let Some(file) = f.as_mut()
            && let Err(e) = file.write_all(line)
        {
            tracing::error!(error = %e, path = %self.path.display(), "cannot append to the reputation log");
        }
    }
}

/// Reads the catalogue file (YAML or JSON).
pub fn read_catalogue(path: &Path) -> anyhow::Result<Catalogue> {
    let text = fs::read_to_string(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let is_json = path.extension().is_some_and(|e| e == "json");
    let c: Catalogue = if is_json {
        serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?
    } else {
        serde_saphyr::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?
    };
    Ok(c)
}

/// Loads static providers into the registry; returns how many resources were loaded, and the
/// problems of the providers that were refused.
pub fn load_static(reg: &Registry, providers: Vec<StaticProvider>) -> (usize, Vec<String>) {
    let (mut n, mut problems) = (0, Vec::new());
    for p in providers {
        let principal = p.principal();
        let name = p.provider.name.clone();
        let a = Announcement { ttl_s: None, lease_id: None, provider: p.provider, resources: p.resources };
        match reg.put_static(&principal, p.verified, a) {
            Ok(done) => n += done.resources.len(),
            Err(e) => problems.push(format!("provider {name}: {e}")),
        }
    }
    (n, problems)
}
