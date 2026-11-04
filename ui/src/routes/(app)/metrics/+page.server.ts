import { GRAFANA_URL } from '$app/env/private';
import { configured, instant } from '#lib/server/prometheus.ts';
import type { PageServerLoad } from './$types';

/** The same queries as the Grafana dashboard (deploy/grafana/dashboards/broker-overview.json). */
const QUERIES = {
	discoveries: 'sum(rate(broker_discoveries_total{job="pacod-broker"}[5m]))',
	discover_p99_ms:
		'1000 * histogram_quantile(0.99, sum by (le) (rate(broker_discover_seconds_bucket{job="pacod-broker"}[5m])))',
	announces: 'sum(rate(broker_announces_total{job="pacod-broker"}[5m]))',
	heartbeats: 'sum(rate(broker_heartbeats_total{job="pacod-broker"}[5m]))',
	expired: 'sum(increase(broker_leases_expired_total{job="pacod-broker"}[1h]))',
	errors: 'sum(rate(broker_http_requests_total{job="pacod-broker",status=~"5.."}[5m]))',
	rebuild_p99_ms:
		'1000 * histogram_quantile(0.99, sum by (le) (rate(broker_index_rebuild_seconds_bucket{job="pacod-broker"}[15m])))',
	reports: 'sum by (kind) (rate(broker_reports_total{job="pacod-broker"}[1h]))'
};

export const load: PageServerLoad = async () => {
	if (!configured) return { configured, values: {}, reports: [], grafana: GRAFANA_URL ?? null };
	const entries = await Promise.all(
		Object.entries(QUERIES).map(async ([k, q]) => [k, await instant(q)] as const)
	);
	const values: Record<string, number | null> = {};
	let reports: { kind: string; value: number }[] = [];
	for (const [k, v] of entries) {
		if (k === 'reports')
			reports = (v ?? []).map((x) => ({ kind: x.labels.kind ?? '', value: x.value }));
		else values[k] = v && v.length ? v[0].value : null;
	}
	return { configured, values, reports, grafana: GRAFANA_URL ?? null };
};
