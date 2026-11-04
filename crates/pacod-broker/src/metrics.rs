//! Prometheus metrics, on their own listener (`BROKER_METRICS_LISTEN`, job `pacod-broker`).
//!
//! No label ever holds a resource, provider or client id: labels are routes, kinds, sources and
//! outcomes, so the number of series stays bounded however many resources there are.

use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};

/// Installs the recorder and returns the handle `/metrics` renders.
pub fn install() -> anyhow::Result<PrometheusHandle> {
    let buckets =
        [0.000_05, 0.000_1, 0.000_25, 0.000_5, 0.001, 0.002_5, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5];
    let handle = PrometheusBuilder::new()
        .set_buckets_for_metric(Matcher::Suffix("_seconds".into()), &buckets)?
        .install_recorder()?;
    describe();
    Ok(handle)
}

fn describe() {
    use metrics::{describe_counter, describe_gauge, describe_histogram};
    describe_counter!("broker_http_requests_total", "Requests, by route, method and status");
    describe_histogram!("broker_http_request_seconds", "Request duration, by route");
    describe_histogram!("broker_discover_seconds", "Time to run a discovery query in the index");
    describe_counter!("broker_discoveries_total", "Discovery queries run");
    describe_counter!("broker_announces_total", "Announcements accepted, by source");
    describe_counter!("broker_announces_refused_total", "Announcements refused");
    describe_counter!("broker_heartbeats_total", "Heartbeats accepted");
    describe_counter!("broker_leases_expired_total", "Leases that ended without a goodbye");
    describe_gauge!("broker_resources", "Resources in the index");
    describe_gauge!("broker_resources_by_kind", "Resources in the index, by kind");
    describe_gauge!("broker_providers", "Providers in the index");
    describe_gauge!("broker_index_generation", "The index generation");
    describe_histogram!("broker_index_rebuild_seconds", "Time to rebuild the index");
    describe_counter!("broker_reports_total", "Reports that counted, by kind");
    describe_histogram!("broker_reputation_compute_seconds", "Time to compute reputation scores");
    describe_counter!("broker_mcp_requests_total", "MCP requests, by method");
}
