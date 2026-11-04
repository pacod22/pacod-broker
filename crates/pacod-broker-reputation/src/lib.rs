//! Reputation for providers, resources and clients (docs/reputation.md).
//!
//! **Evidence in, scores out.** The broker records evidence as it happens: what it observes
//! itself (leases kept or dropped, uptime against the declared schedule, price changes) and
//! what callers report with an interaction ticket (usage outcomes, latency, prices charged,
//! ratings, reports about clients). Each piece updates a decayed aggregate in O(1). Every
//! `BROKER_REPUTATION_EVERY` the engine turns the aggregates into scores and swaps them in; the
//! discovery path only reads the swapped-in scores.
//!
//! **Robust by construction:**
//! - Bayesian smoothing toward a neutral prior of 0.5: little evidence moves a score little;
//! - exponential decay with a half-life: old evidence fades, a provider that improved recovers;
//! - a report counts with the weight of its author's own reputation as a client;
//! - a provider's own organisation cannot rate it; a rater counts once per subject per hour;
//! - performance is a percentile within the resource's kind, never compared across kinds.
//!
//! **Kept.** Every accepted piece of evidence is one JSON line in an append-only log (through a
//! [`Sink`]); a snapshot of the aggregates is written from time to time and the log restarted.
//! Starting again reads the snapshot and replays the log, so a restart loses nothing.

mod aggregate;
mod engine;
mod evidence;

pub use aggregate::Decayed;
pub use engine::{Engine, MemorySink, Scores, Settings, Sink, Snapshot};
pub use evidence::{ClientReportKind, Evidence, Outcome, Side};
