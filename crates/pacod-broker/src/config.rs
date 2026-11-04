//! The broker's configuration: environment variables only (docs/runbook.md §2).
//!
//! The broker needs no configuration to start: with no variable set it serves discovery and
//! registration in `open` mode on port 8060, holds everything in memory and keeps nothing. Every
//! variable has a default; a variable starting with `BROKER_` that the broker does not know is
//! an error, so a misspelt name is noticed instead of being silently ignored. Every problem is
//! collected before the broker refuses to start.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::env::Vars;

/// A secret value: never printed.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<secret>")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// No sign-in: every caller may do everything. For development and closed networks.
    Open,
    /// pacod-id: service accounts for providers and machines, users for people.
    Oidc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    Off,
    /// A provider announcing for the first time waits until an admin approves it.
    NewProviders,
}

#[derive(Debug, Clone)]
pub struct IdSettings {
    pub issuers: Vec<String>,
    pub internal_url: String,
    pub client_id: String,
    pub client_secret: Secret,
    pub audience: String,
}

#[derive(Debug, Clone)]
pub struct ReputationSettings {
    pub every: Duration,
    pub half_life_days: f64,
    pub prior_weight: f64,
    pub provider_weights: Vec<(String, f64)>,
    pub client_weights: Vec<(String, f64)>,
    pub maintenance_notice: Duration,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// This broker's id: the `origin` of every record announced on it.
    pub id: String,
    pub listen: String,
    pub admin_listen: Option<String>,
    pub metrics_listen: Option<String>,
    pub auth: AuthMode,
    pub id_settings: Option<IdSettings>,
    /// A static bearer token for the admin API in `open` mode.
    pub admin_token: Option<Secret>,
    pub public_discovery: bool,
    pub approval: Approval,
    pub state_dir: Option<PathBuf>,
    pub catalog_file: Option<PathBuf>,
    pub lease_ttl_default: Duration,
    pub lease_ttl_min: Duration,
    pub lease_ttl_max: Duration,
    pub max_resources: usize,
    pub max_resources_per_provider: usize,
    pub max_leases_per_provider: usize,
    pub max_body_bytes: usize,
    pub max_description_bytes: usize,
    pub rebuild: Duration,
    pub ticket_key: Option<Secret>,
    pub ticket_ttl: Duration,
    pub reputation: ReputationSettings,
    pub warnings: Vec<String>,
}

/// Variables read without the `BROKER_` prefix check.
const PLAIN: &[&str] = &[
    "BROKER_ID",
    "BROKER_LISTEN",
    "BROKER_ADMIN_LISTEN",
    "BROKER_METRICS_LISTEN",
    "BROKER_AUTH",
    "BROKER_ADMIN_TOKEN",
    "BROKER_ADMIN_TOKEN_FILE",
    "BROKER_PUBLIC_DISCOVERY",
    "BROKER_APPROVAL",
    "BROKER_STATE_DIR",
    "BROKER_CATALOG_FILE",
    "BROKER_LEASE_TTL_DEFAULT",
    "BROKER_LEASE_TTL_MIN",
    "BROKER_LEASE_TTL_MAX",
    "BROKER_MAX_RESOURCES",
    "BROKER_MAX_RESOURCES_PER_PROVIDER",
    "BROKER_MAX_LEASES_PER_PROVIDER",
    "BROKER_MAX_BODY_BYTES",
    "BROKER_MAX_DESCRIPTION_BYTES",
    "BROKER_REBUILD_MS",
    "BROKER_TICKET_KEY",
    "BROKER_TICKET_KEY_FILE",
    "BROKER_TICKET_TTL",
    "BROKER_REPUTATION_EVERY",
    "BROKER_REPUTATION_HALF_LIFE_DAYS",
    "BROKER_REPUTATION_PRIOR_WEIGHT",
    "BROKER_REPUTATION_WEIGHTS_PROVIDER",
    "BROKER_REPUTATION_WEIGHTS_CLIENT",
    "BROKER_MAINTENANCE_NOTICE",
    "BROKER_ID_ISSUER",
    "BROKER_ID_INTERNAL_URL",
    "BROKER_ID_CLIENT_ID",
    "BROKER_ID_CLIENT_SECRET",
    "BROKER_ID_CLIENT_SECRET_FILE",
    "BROKER_ID_AUDIENCE",
    "BROKER_LOG_FORMAT",
    "BROKER_ENV_FILE",
];

/// Every variable the broker reads, for the check that `.env.example` names them all.
pub fn known_variables() -> &'static [&'static str] {
    PLAIN
}

/// The reputation components and their default weights (docs/reputation.md §4).
pub const PROVIDER_WEIGHTS: &[(&str, f64)] = &[
    ("availability", 0.20),
    ("reliability", 0.20),
    ("performance", 0.15),
    ("accuracy", 0.15),
    ("lease_discipline", 0.05),
    ("stability", 0.05),
    ("quality", 0.15),
    ("trust", 0.05),
];

pub const CLIENT_WEIGHTS: &[(&str, f64)] = &[
    ("identity", 0.25),
    ("conduct", 0.30),
    ("settlement", 0.20),
    ("commitment", 0.10),
    ("feedback_honesty", 0.10),
    ("tenure", 0.05),
];

#[derive(Debug)]
pub struct Errors(pub Vec<String>);

impl fmt::Display for Errors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "the configuration has {} problem(s):", self.0.len())?;
        for e in &self.0 {
            writeln!(f, "  - {e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Errors {}

struct Reader<'a> {
    vars: &'a Vars,
    errors: Vec<String>,
    warnings: Vec<String>,
}

impl Reader<'_> {
    fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(str::trim).filter(|v| !v.is_empty())
    }

    fn string(&self, name: &str, default: &str) -> String {
        self.get(name).unwrap_or(default).to_string()
    }

    fn optional(&self, name: &str) -> Option<String> {
        self.get(name).map(str::to_string)
    }

    fn number(&mut self, name: &str, default: u64, min: u64, max: u64) -> u64 {
        match self.get(name) {
            None => default,
            Some(v) => match v.replace('_', "").parse::<u64>() {
                Ok(n) if (min..=max).contains(&n) => n,
                _ => {
                    self.errors.push(format!("{name}={v:?}: a whole number from {min} to {max}"));
                    default
                }
            },
        }
    }

    fn float(&mut self, name: &str, default: f64, min: f64, max: f64) -> f64 {
        match self.get(name) {
            None => default,
            Some(v) => match v.parse::<f64>() {
                Ok(n) if n.is_finite() && (min..=max).contains(&n) => n,
                _ => {
                    self.errors.push(format!("{name}={v:?}: a number from {min} to {max}"));
                    default
                }
            },
        }
    }

    fn boolean(&mut self, name: &str, default: bool) -> bool {
        match self.get(name).map(str::to_ascii_lowercase).as_deref() {
            None => default,
            Some("true" | "1" | "yes" | "on") => true,
            Some("false" | "0" | "no" | "off") => false,
            Some(v) => {
                self.errors.push(format!("{name}={v:?}: true or false"));
                default
            }
        }
    }

    /// A secret from `NAME` or from the file `NAME_FILE` names.
    fn secret(&mut self, name: &str) -> Option<Secret> {
        let file_var = format!("{name}_FILE");
        match (self.get(name), self.get(&file_var)) {
            (Some(_), Some(_)) => {
                self.errors.push(format!("{name} and {file_var} are both set; set one"));
                None
            }
            (Some(v), None) => Some(Secret(v.to_string())),
            (None, Some(path)) => match std::fs::read_to_string(path) {
                Ok(v) if !v.trim().is_empty() => Some(Secret(v.trim().to_string())),
                Ok(_) => {
                    self.errors.push(format!("{file_var}: {path} is empty"));
                    None
                }
                Err(e) => {
                    self.errors.push(format!("{file_var}: reading {path}: {e}"));
                    None
                }
            },
            (None, None) => None,
        }
    }

    fn listen(&mut self, name: &str, default: Option<&str>) -> Option<String> {
        let v = match self.vars.get(name) {
            // Set but empty turns an optional listener off.
            Some(v) if v.trim().is_empty() || v.trim() == "off" => return None,
            Some(v) => v.trim().to_string(),
            None => default?.to_string(),
        };
        // An address, or a host name with a port (an alias on a Docker network), resolved when
        // the listener binds.
        let ok = v.parse::<SocketAddr>().is_ok()
            || v.rsplit_once(':').is_some_and(|(h, p)| !h.is_empty() && p.parse::<u16>().is_ok());
        if !ok {
            self.errors.push(format!("{name}={v:?}: host:port"));
        }
        Some(v)
    }

    fn weights(&mut self, name: &str, defaults: &[(&str, f64)]) -> Vec<(String, f64)> {
        let mut out: Vec<(String, f64)> = defaults.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let Some(v) = self.get(name).map(str::to_string) else { return out };
        for part in v.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let Some((k, w)) = part.split_once('=') else {
                self.errors.push(format!("{name}: {part:?} is not component=weight"));
                continue;
            };
            let (k, w) = (k.trim(), w.trim());
            match (out.iter_mut().find(|(n, _)| n == k), w.parse::<f64>()) {
                (Some(slot), Ok(x)) if x.is_finite() && x >= 0.0 => slot.1 = x,
                (None, _) => self.errors.push(format!(
                    "{name}: unknown component {k:?}; components are {}",
                    defaults.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                )),
                _ => self.errors.push(format!("{name}: {w:?} is not a weight of zero or more")),
            }
        }
        if out.iter().map(|(_, w)| w).sum::<f64>() <= 0.0 {
            self.errors.push(format!("{name}: the weights add up to zero"));
        }
        out
    }
}

pub fn read(vars: &Vars) -> Result<Config, Errors> {
    let mut r = Reader { vars, errors: Vec::new(), warnings: Vec::new() };
    for (name, _) in vars.iter() {
        if name.starts_with("BROKER_") && !PLAIN.contains(&name) {
            r.errors.push(format!("{name}: not a variable the broker knows (docs/runbook.md §2)"));
        }
    }

    let id = r.string("BROKER_ID", "broker");
    if !pacod_broker_model::ids::is_slug(&id, 64) {
        r.errors.push(format!("BROKER_ID={id:?}: a slug"));
    }
    let listen = r.listen("BROKER_LISTEN", Some("0.0.0.0:8060")).unwrap_or_else(|| {
        r.errors.push("BROKER_LISTEN: the public listener cannot be turned off".into());
        String::new()
    });
    let admin_listen = r.listen("BROKER_ADMIN_LISTEN", None);
    let metrics_listen = r.listen("BROKER_METRICS_LISTEN", Some("0.0.0.0:9160"));

    let auth = match r.get("BROKER_AUTH").map(str::to_ascii_lowercase).as_deref() {
        None | Some("open") => AuthMode::Open,
        Some("oidc") => AuthMode::Oidc,
        Some(v) => {
            r.errors.push(format!("BROKER_AUTH={v:?}: open or oidc"));
            AuthMode::Open
        }
    };
    let id_settings = if auth == AuthMode::Oidc {
        let issuers: Vec<String> = r
            .get("BROKER_ID_ISSUER")
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        if issuers.is_empty() {
            r.errors.push("BROKER_ID_ISSUER: needed with BROKER_AUTH=oidc".into());
        }
        let internal_url = r.optional("BROKER_ID_INTERNAL_URL").unwrap_or_else(|| {
            r.errors
                .push("BROKER_ID_INTERNAL_URL: needed with BROKER_AUTH=oidc (http://pacod-id-internal:8071)".into());
            String::new()
        });
        let client_secret = r.secret("BROKER_ID_CLIENT_SECRET").unwrap_or_else(|| {
            r.errors.push("BROKER_ID_CLIENT_SECRET: needed with BROKER_AUTH=oidc".into());
            Secret::default()
        });
        if !client_secret.0.is_empty() && client_secret.0.len() < 32 {
            r.errors.push("BROKER_ID_CLIENT_SECRET: at least 32 characters".into());
        }
        Some(IdSettings {
            issuers,
            internal_url,
            client_id: r.string("BROKER_ID_CLIENT_ID", "pacod-broker-service"),
            client_secret,
            audience: r.string("BROKER_ID_AUDIENCE", "pacod-broker"),
        })
    } else {
        for v in
            ["BROKER_ID_ISSUER", "BROKER_ID_INTERNAL_URL", "BROKER_ID_CLIENT_SECRET", "BROKER_ID_CLIENT_SECRET_FILE"]
        {
            if r.get(v).is_some() {
                r.warnings.push(format!("{v} is set but BROKER_AUTH is open; it is not used"));
            }
        }
        None
    };
    let admin_token = r.secret("BROKER_ADMIN_TOKEN");
    if admin_token.as_ref().is_some_and(|t| t.0.len() < 24) {
        r.errors.push("BROKER_ADMIN_TOKEN: at least 24 characters".into());
    }
    if auth == AuthMode::Open {
        let public = !listen.starts_with("127.") && !listen.starts_with("localhost:") && !listen.starts_with("[::1]");
        if public {
            r.warnings.push(format!(
                "BROKER_AUTH=open on {listen}: anyone who reaches the broker can announce and discover; use oidc beyond a closed network"
            ));
        }
    }
    let public_discovery = r.boolean("BROKER_PUBLIC_DISCOVERY", false);
    let approval = match r.get("BROKER_APPROVAL").map(str::to_ascii_lowercase).as_deref() {
        None | Some("off") => Approval::Off,
        Some("new_providers") => Approval::NewProviders,
        Some(v) => {
            r.errors.push(format!("BROKER_APPROVAL={v:?}: off or new_providers"));
            Approval::Off
        }
    };
    let state_dir = r.optional("BROKER_STATE_DIR").map(PathBuf::from);
    if state_dir.is_none() {
        r.warnings.push("BROKER_STATE_DIR is not set: static entries added through the admin API and reputation live in memory only".into());
    }
    let catalog_file = r.optional("BROKER_CATALOG_FILE").map(PathBuf::from);
    if let Some(f) = &catalog_file
        && !f.is_file()
    {
        r.errors.push(format!("BROKER_CATALOG_FILE: {} is not a file", f.display()));
    }

    let secs = |n: u64| Duration::from_secs(n);
    let lease_ttl_min = secs(r.number("BROKER_LEASE_TTL_MIN", 5, 1, 3600));
    let lease_ttl_max = secs(r.number("BROKER_LEASE_TTL_MAX", 3600, 5, 86_400));
    let lease_ttl_default = secs(r.number("BROKER_LEASE_TTL_DEFAULT", 30, 1, 86_400));
    if lease_ttl_min > lease_ttl_max || lease_ttl_default < lease_ttl_min || lease_ttl_default > lease_ttl_max {
        r.errors.push("BROKER_LEASE_TTL_MIN ≤ BROKER_LEASE_TTL_DEFAULT ≤ BROKER_LEASE_TTL_MAX".into());
    }
    let max_resources = r.number("BROKER_MAX_RESOURCES", 100_000, 1, 10_000_000) as usize;
    let max_resources_per_provider = r.number("BROKER_MAX_RESOURCES_PER_PROVIDER", 5_000, 1, 1_000_000) as usize;
    let max_leases_per_provider = r.number("BROKER_MAX_LEASES_PER_PROVIDER", 16, 1, 10_000) as usize;
    let max_body_bytes = r.number("BROKER_MAX_BODY_BYTES", 8 << 20, 4096, 256 << 20) as usize;
    let max_description_bytes = r.number("BROKER_MAX_DESCRIPTION_BYTES", 16 * 1024, 256, 1 << 20) as usize;
    let rebuild = Duration::from_millis(r.number("BROKER_REBUILD_MS", 20, 0, 10_000));
    let ticket_key = r.secret("BROKER_TICKET_KEY");
    if ticket_key.as_ref().is_some_and(|k| k.0.len() < 32) {
        r.errors.push("BROKER_TICKET_KEY: at least 32 characters".into());
    }
    let ticket_ttl = secs(r.number("BROKER_TICKET_TTL", 86_400, 60, 30 * 86_400));
    let reputation = ReputationSettings {
        every: secs(r.number("BROKER_REPUTATION_EVERY", 60, 1, 86_400)),
        half_life_days: r.float("BROKER_REPUTATION_HALF_LIFE_DAYS", 30.0, 0.1, 3650.0),
        prior_weight: r.float("BROKER_REPUTATION_PRIOR_WEIGHT", 10.0, 0.0, 10_000.0),
        provider_weights: r.weights("BROKER_REPUTATION_WEIGHTS_PROVIDER", PROVIDER_WEIGHTS),
        client_weights: r.weights("BROKER_REPUTATION_WEIGHTS_CLIENT", CLIENT_WEIGHTS),
        maintenance_notice: secs(r.number("BROKER_MAINTENANCE_NOTICE", 86_400, 0, 90 * 86_400)),
    };
    if let Some(f) = r.get("BROKER_LOG_FORMAT")
        && !matches!(f.to_ascii_lowercase().as_str(), "json" | "text")
    {
        r.errors.push(format!("BROKER_LOG_FORMAT={f:?}: json or text"));
    }

    if !r.errors.is_empty() {
        return Err(Errors(r.errors));
    }
    Ok(Config {
        id,
        listen,
        admin_listen,
        metrics_listen,
        auth,
        id_settings,
        admin_token,
        public_discovery,
        approval,
        state_dir,
        catalog_file,
        lease_ttl_default,
        lease_ttl_min,
        lease_ttl_max,
        max_resources,
        max_resources_per_provider,
        max_leases_per_provider,
        max_body_bytes,
        max_description_bytes,
        rebuild,
        ticket_key,
        ticket_ttl,
        reputation,
        warnings: r.warnings,
    })
}

impl Config {
    /// A summary without secrets, for `pacod-broker check` and the admin API.
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "listen": self.listen,
            "admin_listen": self.admin_listen,
            "metrics_listen": self.metrics_listen,
            "auth": match self.auth { AuthMode::Open => "open", AuthMode::Oidc => "oidc" },
            "id_issuers": self.id_settings.as_ref().map(|s| s.issuers.clone()),
            "id_internal_url": self.id_settings.as_ref().map(|s| s.internal_url.clone()),
            "id_audience": self.id_settings.as_ref().map(|s| s.audience.clone()),
            "admin_token": self.admin_token.is_some(),
            "public_discovery": self.public_discovery,
            "approval": match self.approval { Approval::Off => "off", Approval::NewProviders => "new_providers" },
            "state_dir": self.state_dir.as_ref().map(|p| p.display().to_string()),
            "catalog_file": self.catalog_file.as_ref().map(|p| p.display().to_string()),
            "lease_ttl_seconds": {"default": self.lease_ttl_default.as_secs(), "min": self.lease_ttl_min.as_secs(), "max": self.lease_ttl_max.as_secs()},
            "limits": {
                "resources": self.max_resources,
                "resources_per_provider": self.max_resources_per_provider,
                "leases_per_provider": self.max_leases_per_provider,
                "body_bytes": self.max_body_bytes,
                "description_bytes": self.max_description_bytes,
            },
            "rebuild_ms": self.rebuild.as_millis() as u64,
            "ticket_ttl_seconds": self.ticket_ttl.as_secs(),
            "reputation": {
                "every_seconds": self.reputation.every.as_secs(),
                "half_life_days": self.reputation.half_life_days,
                "prior_weight": self.reputation.prior_weight,
                "provider_weights": self.reputation.provider_weights.iter().map(|(k, v)| (k.clone(), serde_json::Value::from(*v))).collect::<serde_json::Map<_, _>>(),
                "client_weights": self.reputation.client_weights.iter().map(|(k, v)| (k.clone(), serde_json::Value::from(*v))).collect::<serde_json::Map<_, _>>(),
                "maintenance_notice_seconds": self.reputation.maintenance_notice.as_secs(),
            },
            "warnings": self.warnings,
        })
    }

    /// A configuration for tests: open mode, nothing kept.
    pub fn for_tests() -> Self {
        read(&Vars::from_pairs([
            ("BROKER_LISTEN", "127.0.0.1:0"),
            ("BROKER_METRICS_LISTEN", "off"),
            ("BROKER_REBUILD_MS", "0"),
        ]))
        .expect("test configuration")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_variables_mean_open_mode_on_8060() {
        let c = read(&Vars::default()).unwrap();
        assert_eq!((c.listen.as_str(), c.auth, c.admin_listen.is_none()), ("0.0.0.0:8060", AuthMode::Open, true));
        assert!(c.warnings.iter().any(|w| w.contains("BROKER_AUTH=open")));
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let v = Vars::from_pairs([
            ("BROKER_LISTN", "x"),
            ("BROKER_AUTH", "oidc"),
            ("BROKER_LEASE_TTL_MIN", "0"),
            ("BROKER_REPUTATION_WEIGHTS_PROVIDER", "availability=0.5,speed=1"),
        ]);
        let e = read(&v).unwrap_err().to_string();
        for want in [
            "BROKER_LISTN: not a variable",
            "BROKER_ID_ISSUER",
            "BROKER_ID_CLIENT_SECRET",
            "BROKER_LEASE_TTL_MIN",
            "unknown component \"speed\"",
        ] {
            assert!(e.contains(want), "{want} missing in {e}");
        }
    }

    #[test]
    fn weights_can_be_overridden() {
        let v = Vars::from_pairs([("BROKER_REPUTATION_WEIGHTS_CLIENT", "conduct=0.5, tenure=0")]);
        let c = read(&v).unwrap();
        assert!(c.reputation.client_weights.contains(&("conduct".into(), 0.5)));
        assert!(c.reputation.client_weights.contains(&("tenure".into(), 0.0)));
    }

    #[test]
    fn the_example_file_names_every_variable() {
        let example = include_str!("../../../deploy/.env.example");
        for v in PLAIN {
            assert!(example.contains(&format!("{v}=")), "{v} is missing in deploy/.env.example");
        }
        for line in example.lines() {
            let line = line.trim_start_matches('#').trim();
            if let Some((name, _)) = line.split_once('=')
                && name.starts_with("BROKER_")
                && !name.contains(' ')
            {
                assert!(PLAIN.contains(&name), "deploy/.env.example names {name}, which the broker does not read");
            }
        }
    }
}
