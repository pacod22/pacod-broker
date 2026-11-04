//! Who is calling, and what they may do (docs/concept.md §5).
//!
//! In `open` mode every caller may do everything, and a client may name itself with the
//! `X-Broker-Client` header so that usage reports and ratings can be tried out without pacod-id.
//! In `oidc` mode callers present a pacod-id token: a person's access token or a service
//! account's `pidt_…` token. Their permissions come from the broker's manifest
//! (authz/broker.yaml) and the role bindings pacod-id's change feed reports.

use http::HeaderMap;
use pacod_broker_index::Viewer;

use crate::config::Config;
use crate::registry::Announcer;

/// What a caller may do. The names are the permissions of authz/broker.yaml.
pub mod perm {
    pub const DISCOVER: &str = "resources.discover";
    pub const ANNOUNCE: &str = "resources.announce";
    pub const REPORT_USAGE: &str = "reports.usage";
    pub const REPORT_FEEDBACK: &str = "reports.feedback";
    pub const REPORT_CLIENT: &str = "reports.client";
    pub const ADMIN_READ: &str = "broker.read";
    pub const ADMIN_MANAGE: &str = "broker.manage";
    pub const MODERATE: &str = "reputation.moderate";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalKind {
    User,
    Service,
}

#[derive(Debug, Clone)]
pub struct Principal {
    pub sub: String,
    pub kind: PrincipalKind,
    pub name: Option<String>,
    pub organisation_id: Option<String>,
    /// The organisation is verified in pacod-id (or by a broker admin).
    pub verified: bool,
    /// The permissions the caller holds at platform level, or in its organisation.
    pub permissions: Vec<String>,
}

impl Principal {
    pub fn has(&self, p: &str) -> bool {
        self.permissions.iter().any(|x| x == "*" || x == p)
    }
}

#[derive(Debug, Clone)]
pub enum Caller {
    /// `open` mode, optionally with a name the client gave itself.
    Open {
        client: Option<String>,
    },
    /// No token, in `oidc` mode.
    Anonymous,
    Principal(Principal),
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("the token is not valid")]
    Invalid,
    /// pacod-id cannot be asked now; try again later.
    #[error("identity is unavailable")]
    Unavailable,
}

impl Caller {
    pub fn may(&self, permission: &str, cfg: &Config) -> bool {
        match self {
            Self::Open { .. } => true,
            Self::Anonymous => permission == perm::DISCOVER && cfg.public_discovery,
            Self::Principal(p) => p.has(permission),
        }
    }

    /// The principal reputation and tickets are kept for; `None` for anonymous callers.
    pub fn client_id(&self) -> Option<String> {
        match self {
            Self::Open { client } => client.as_ref().map(|c| format!("anon:{c}")),
            Self::Anonymous => None,
            Self::Principal(p) => Some(p.sub.clone()),
        }
    }

    pub fn organisation(&self) -> Option<&str> {
        match self {
            Self::Principal(p) => p.organisation_id.as_deref(),
            _ => None,
        }
    }

    /// How sure the broker is of who the caller is (reputation component `identity`).
    pub fn identity_level(&self) -> f64 {
        match self {
            Self::Open { .. } | Self::Anonymous => 0.0,
            Self::Principal(p) => match (p.kind, p.verified) {
                (PrincipalKind::Service, true) | (PrincipalKind::User, true) => 1.0,
                (PrincipalKind::Service, false) => 0.7,
                (PrincipalKind::User, false) => 0.6,
            },
        }
    }

    /// Who the caller announces as.
    pub fn announcer(&self, provider_name: &str) -> Option<Announcer> {
        match self {
            Self::Open { .. } => {
                Some(Announcer { principal: format!("anon:{provider_name}"), organisation_id: None, verified: false })
            }
            Self::Anonymous => None,
            Self::Principal(p) => Some(Announcer {
                principal: p.sub.clone(),
                organisation_id: p.organisation_id.clone(),
                verified: p.verified,
            }),
        }
    }

    /// The principal that owns leases the caller opened. In open mode a lease belongs to the
    /// provider name it was announced under, which the caller must repeat.
    pub fn lease_owner(&self, provider_name: Option<&str>) -> Option<String> {
        match self {
            Self::Open { .. } => provider_name.map(|n| format!("anon:{n}")),
            Self::Anonymous => None,
            Self::Principal(p) => Some(p.sub.clone()),
        }
    }

    /// What the caller may see in discovery.
    pub fn viewer(&self, cfg: &Config, reputation: Option<(f64, f64)>) -> Viewer {
        match self {
            Self::Open { .. } => Viewer { all: true, authenticated: true, reputation, ..Viewer::default() },
            Self::Anonymous => Viewer { public_discovery: cfg.public_discovery, ..Viewer::default() },
            Self::Principal(p) => Viewer {
                all: p.has(perm::ADMIN_READ),
                reviewer: p.has(perm::ADMIN_READ),
                authenticated: true,
                organisation: p.organisation_id.clone(),
                reputation,
                public_discovery: cfg.public_discovery,
            },
        }
    }
}

/// The bearer token of a request, if any.
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
}

/// The name an open-mode client gives itself.
pub fn open_client(headers: &HeaderMap) -> Option<String> {
    let v = headers.get("x-broker-client")?.to_str().ok()?.trim();
    pacod_broker_model::ids::is_slug(v, 96).then(|| v.to_string())
}
