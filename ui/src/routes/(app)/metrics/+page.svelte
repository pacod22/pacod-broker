<script lang="ts">
	import PageHeader from '#lib/components/PageHeader.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
	const fmt = (v: number | null | undefined, digits = 2) =>
		v == null || Number.isNaN(v) ? '—' : v.toFixed(digits);
	const TILES: [string, string, number][] = [
		['discoveries', 'Discoveries / s', 2],
		['discover_p99_ms', 'Discovery p99, ms', 3],
		['announces', 'Announcements / s', 3],
		['heartbeats', 'Heartbeats / s', 2],
		['expired', 'Leases expired, last hour', 0],
		['errors', '5xx / s', 3],
		['rebuild_p99_ms', 'Index rebuild p99, ms', 1]
	];
</script>

<PageHeader
	title="Metrics"
	description="From the shared Prometheus (job pacod-broker), with the queries of the Grafana dashboard."
/>

{#if !data.configured}
	<section class="card stack">
		<p>PROMETHEUS_URL is not set, so the console cannot read the broker's metrics.</p>
		<p class="muted small">
			The broker exposes them itself on BROKER_METRICS_LISTEN (<code>/metrics</code>);
			platform/prometheus/broker.yml is the scrape job for the shared Prometheus.
		</p>
	</section>
{:else}
	<section class="tiles">
		{#each TILES as [key, label, digits] (key)}
			<div class="card">
				<p class="eyebrow">{label}</p>
				<p class="figure">{fmt(data.values[key], digits)}</p>
			</div>
		{/each}
	</section>
	<section class="card stack">
		<h2>Reports per hour that counted</h2>
		<ul>
			{#each data.reports as r (r.kind)}<li>{r.kind}: {fmt(r.value * 3600, 0)}</li>{:else}<li
					class="muted"
				>
					None.
				</li>{/each}
		</ul>
	</section>
{/if}
{#if data.grafana}<p>
		<a href={data.grafana} rel="noopener noreferrer" target="_blank"
			>Open the dashboards in Grafana</a
		>
	</p>{/if}

<style>
	.tiles {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
		gap: var(--space-4);
	}

	.figure {
		font-family: var(--font-display);
		font-size: var(--text-xl);
		font-weight: 800;
	}
</style>
