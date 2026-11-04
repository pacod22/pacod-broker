//! The scenario format (docs/testing.md §2): who provides, who consumes, how the broker is set
//! up, and the steps with what each must show.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub broker: BrokerSetup,
    #[serde(default)]
    pub identity: Option<IdentitySetup>,
    #[serde(default)]
    pub providers: Vec<ProviderSetup>,
    #[serde(default)]
    pub consumers: Vec<ConsumerSetup>,
    pub steps: Vec<Step>,
    /// Where the file was read from; announcement and catalogue paths are relative to it.
    #[serde(skip)]
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerSetup {
    /// `open` (default) or `oidc`.
    #[serde(default)]
    pub auth: Option<String>,
    /// Any BROKER_* setting.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Give the broker a state directory (kept across `restart_broker`).
    #[serde(default)]
    pub state_dir: bool,
    /// A catalogue file, relative to the scenario.
    #[serde(default)]
    pub catalog: Option<String>,
    /// Require this admin token in open mode.
    #[serde(default)]
    pub admin_token: Option<String>,
}

impl BrokerSetup {
    pub fn is_oidc(&self) -> bool {
        self.auth.as_deref() == Some("oidc")
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentitySetup {
    #[serde(default)]
    pub organisations: Vec<String>,
    #[serde(default)]
    pub principals: Vec<PrincipalSetup>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrincipalSetup {
    pub name: String,
    /// `user` or `service`.
    pub kind: String,
    #[serde(default)]
    pub organisation: Option<String>,
    /// `broker.<role>@platform` or `broker.<role>@organisation` (the principal's own).
    #[serde(default)]
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSetup {
    pub name: String,
    /// An announcement file, relative to the scenario's directory.
    pub from: String,
    /// The pacod-id principal it announces as (oidc).
    #[serde(default, rename = "as")]
    pub principal: Option<String>,
    /// Values to set in the announcement, by path (`resources[0].pricing.periods[0].rates.input`).
    #[serde(default)]
    pub set: BTreeMap<String, Value>,
    /// Several providers from one template: `name-1` … `name-n`.
    #[serde(default)]
    pub count: Option<usize>,
    /// With `count`: values spread evenly from the first to the second, by path.
    #[serde(default)]
    pub vary: BTreeMap<String, (f64, f64)>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerSetup {
    pub name: String,
    /// The pacod-id principal (oidc); in open mode the name is the client's own name.
    #[serde(default, rename = "as")]
    pub principal: Option<String>,
    /// No identity at all.
    #[serde(default)]
    pub anonymous: bool,
    /// A service token of its own, which the broker has not seen yet (oidc): checking it needs
    /// pacod-id, unlike a token the broker already knows.
    #[serde(default)]
    pub fresh_token: bool,
}

/// One step: a single key naming what to do.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    /// Announce and keep the lease alive (heartbeats).
    Start(Names),
    /// Announce once, no heartbeats; `status` expects a refusal.
    Announce {
        provider: String,
        #[serde(default)]
        status: Option<u16>,
    },
    /// End the lease cleanly.
    Stop(Names),
    /// Stop heartbeats without a goodbye.
    Crash(Names),
    /// Set live state the next heartbeat carries.
    Live {
        provider: String,
        #[serde(default)]
        resource: Option<String>,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        in_use: Option<u32>,
        #[serde(default)]
        ttft_ms: Option<f64>,
    },
    Discover {
        #[serde(rename = "as")]
        consumer: String,
        #[serde(default)]
        query: Value,
        #[serde(default)]
        expect: Expect,
        #[serde(default)]
        status: Option<u16>,
        #[serde(default)]
        within: Option<String>,
    },
    Resource {
        #[serde(rename = "as")]
        consumer: String,
        /// `provider/key`.
        id: String,
        #[serde(default)]
        expect: Expect,
        #[serde(default)]
        status: Option<u16>,
        #[serde(default)]
        within: Option<String>,
    },
    ProviderView {
        #[serde(rename = "as")]
        consumer: String,
        provider: String,
        #[serde(default)]
        expect: Expect,
        #[serde(default)]
        status: Option<u16>,
        #[serde(default)]
        within: Option<String>,
    },
    /// Start collecting the watch stream as a consumer.
    Watch {
        #[serde(rename = "as")]
        consumer: String,
    },
    /// The events collected must include these types (in any order).
    ExpectEvents {
        #[serde(rename = "as")]
        consumer: String,
        contains: Vec<String>,
        #[serde(default)]
        within: Option<String>,
    },
    /// Report calls, with a ticket the consumer gets for the resource.
    Report {
        #[serde(rename = "as")]
        consumer: String,
        on: String,
        outcome: String,
        #[serde(default)]
        ttft_ms: Option<f64>,
        #[serde(default)]
        ms_per_token: Option<f64>,
        #[serde(default)]
        price_charged: Option<f64>,
        #[serde(default)]
        rate: Option<String>,
        #[serde(default = "one")]
        times: usize,
        /// Use another consumer's ticket.
        #[serde(default)]
        ticket_of: Option<String>,
        /// No ticket at all.
        #[serde(default)]
        no_ticket: bool,
        /// How many of the reports must count.
        #[serde(default)]
        accepted: Option<usize>,
        #[serde(default)]
        status: Option<u16>,
    },
    Rate {
        #[serde(rename = "as")]
        consumer: String,
        on: String,
        rating: u8,
        #[serde(default)]
        ticket_of: Option<String>,
        #[serde(default)]
        accepted: Option<usize>,
        #[serde(default)]
        status: Option<u16>,
    },
    /// A provider reports about a consumer that holds a ticket for one of its resources.
    ReportClient {
        provider: String,
        about: String,
        on: String,
        kind: String,
        #[serde(default)]
        status: Option<u16>,
    },
    /// Reputation, read through the admin API, until the expectations hold.
    Reputation {
        expect: BTreeMap<String, BTreeMap<String, Value>>,
        #[serde(default)]
        within: Option<String>,
    },
    Admin {
        action: String,
        #[serde(default)]
        target: Option<String>,
        #[serde(default)]
        side: Option<String>,
        #[serde(default)]
        status: Option<u16>,
        /// For `put_static`: an announcement file.
        #[serde(default)]
        file: Option<String>,
    },
    /// Admin API calls with a given bearer, for access checks.
    AdminAccess {
        #[serde(rename = "as")]
        who: Option<String>,
        #[serde(default)]
        token: Option<String>,
        status: u16,
    },
    RestartBroker {
        #[serde(default = "yes")]
        graceful: bool,
    },
    /// Replace the catalogue file and send SIGHUP.
    ReloadCatalog {
        file: String,
    },
    IdDown,
    IdUp,
    Wait(String),
    /// Wait until a resource is (not) found by a consumer.
    WaitFor {
        #[serde(rename = "as")]
        consumer: String,
        #[serde(default)]
        gone: Option<String>,
        #[serde(default)]
        present: Option<String>,
        #[serde(default)]
        within: Option<String>,
    },
    /// Counters and gauges of /metrics.
    Metrics {
        expect: BTreeMap<String, Value>,
        #[serde(default)]
        within: Option<String>,
    },
}

fn one() -> usize {
    1
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Names {
    One(String),
    Many(Vec<String>),
}

impl Names {
    pub fn list(&self) -> Vec<String> {
        match self {
            Self::One(s) => vec![s.clone()],
            Self::Many(v) => v.clone(),
        }
    }
}

/// What an answer must show.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// The number of results: a number or a comparison (`">=3"`).
    #[serde(default)]
    pub total: Option<Value>,
    /// Exactly these resources, as `provider/key`, in any order.
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    /// The first results, in this order.
    #[serde(default)]
    pub order: Option<Vec<String>>,
    #[serde(default)]
    pub contains: Vec<String>,
    #[serde(default)]
    pub excludes: Vec<String>,
    /// Values at paths in the answer (`hits[0].price.source`): equal, or a comparison.
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
    /// The results are exactly what the scenario's own filter of the announced data gives.
    #[serde(default)]
    pub oracle: bool,
}

impl Scenario {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let mut s: Scenario = serde_saphyr::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        s.dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        Ok(s)
    }
}

/// `"2s"`, `"500ms"`, `"1m"` as a duration.
pub fn duration(s: &str) -> anyhow::Result<std::time::Duration> {
    let s = s.trim();
    let (num, unit) = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).map_or((s, "s"), |i| (&s[..i], &s[i..]));
    let v: f64 = num.parse().map_err(|_| anyhow::anyhow!("{s:?} is not a duration"))?;
    let secs = match unit {
        "ms" => v / 1000.0,
        "s" | "" => v,
        "m" => v * 60.0,
        _ => anyhow::bail!("{s:?}: ms, s or m"),
    };
    Ok(std::time::Duration::from_secs_f64(secs))
}
