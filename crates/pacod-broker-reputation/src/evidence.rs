//! The evidence reputation is computed from: one variant per kind of fact.

use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The side of a score: providers and their resources are rated as sellers, clients as buyers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Resource,
    Provider,
    Client,
}

/// How a call ended, as the caller reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    /// The caller's own mistake: does not count against the resource.
    ClientError,
    ServerError,
    Timeout,
    RateLimited,
    /// Refused although the resource said it had capacity.
    Refused,
}

/// What a provider reports about a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientReportKind {
    Abuse,
    PolicyViolation,
    NonPayment,
    NoShow,
    LateCancel,
    GoodStanding,
}

/// One piece of evidence. Every variant carries the instant it happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Evidence {
    /// A provider's lease ended: cleanly (deregistered, drained) or by expiry.
    LeaseEnded { provider: String, resources: Vec<String>, clean: bool, at: Timestamp },
    /// Over the last interval the resource was announced and usable for `up` of the `expected`
    /// seconds its schedule said it would be. Maintenance announced in time is not expected time.
    Uptime { resource: String, provider: String, up: f64, expected: f64, at: Timestamp },
    /// The live status flipped between usable and unusable.
    StatusFlap { resource: String, provider: String, at: Timestamp },
    /// The resource's prices or interfaces changed.
    Changed { resource: String, provider: String, at: Timestamp },
    /// Who the provider is, as pacod-id or an admin vouch for it.
    ProviderVerified { provider: String, verified: bool, at: Timestamp },
    /// A call, as the client that made it reports it.
    Usage {
        resource: String,
        provider: String,
        kind: String,
        client: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_org: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_org: Option<String>,
        outcome: Outcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttft_ms: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ms_per_token: Option<f64>,
        /// The declared values at the time, to judge the accuracy of claims.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        declared_ttft_ms: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        price_charged: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        price_listed: Option<f64>,
        at: Timestamp,
        /// Who reported the call when it was not the client: a gateway that carried it
        /// (`reports.witness`). The evidence stays the client's.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        witness: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        witness_org: Option<String>,
    },
    /// A rating, 1 to 5.
    Feedback {
        resource: String,
        provider: String,
        client: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_org: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_org: Option<String>,
        rating: u8,
        at: Timestamp,
    },
    /// A provider's report about a client.
    ClientReport { client: String, provider: String, report: ClientReportKind, id: String, at: Timestamp },
    /// An admin found a client report unfounded: its effect is taken back.
    ClientReportRejected { client: String, report: ClientReportKind, id: String, at: Timestamp },
    /// An incident on a provider's side an admin upheld (a breach, a broken promise).
    ProviderIncident { provider: String, at: Timestamp },
    /// How sure the broker is of who a client is: 0 anonymous, 0.6 a person signed in, 0.7 a
    /// service account, 1 a service account of a verified organisation.
    ClientIdentity { client: String, level: f64, at: Timestamp },
    /// An admin froze or unfroze a subject's score.
    Freeze { side: Side, subject: String, frozen: bool, at: Timestamp },
    /// An admin reset a subject's score to the prior.
    Reset { side: Side, subject: String, at: Timestamp },
}

impl Evidence {
    pub fn at(&self) -> Timestamp {
        match self {
            Self::LeaseEnded { at, .. }
            | Self::Uptime { at, .. }
            | Self::StatusFlap { at, .. }
            | Self::Changed { at, .. }
            | Self::ProviderVerified { at, .. }
            | Self::Usage { at, .. }
            | Self::Feedback { at, .. }
            | Self::ClientReport { at, .. }
            | Self::ClientReportRejected { at, .. }
            | Self::ProviderIncident { at, .. }
            | Self::ClientIdentity { at, .. }
            | Self::Freeze { at, .. }
            | Self::Reset { at, .. } => *at,
        }
    }
}
