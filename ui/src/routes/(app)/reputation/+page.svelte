<script lang="ts">
	import Badge from '#lib/components/Badge.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { ago, pct } from '#lib/format.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
	const SIDES = [
		{ id: 'provider', label: 'Providers' },
		{ id: 'resource', label: 'Resources' },
		{ id: 'client', label: 'Clients' }
	];
</script>

<PageHeader
	title="Reputation"
	description="Scores computed from evidence: what the broker observed and what callers reported with interaction tickets. Lowest first, to find problems."
/>

<nav class="row tabs" aria-label="Side">
	{#each SIDES as s (s.id)}
		<a href="?side={s.id}" class="btn" class:btn-primary={data.side === s.id}>{s.label}</a>
	{/each}
	<a
		href="?side={data.side}&sort={data.sort === 'score' ? 'confidence' : 'score'}"
		class="btn-quiet">Sort by {data.sort === 'score' ? 'confidence' : 'score'}</a
	>
</nav>

<div class="table-wrap">
	<table>
		<thead
			><tr
				><th>Subject</th><th class="num">Score</th><th class="num">Confidence</th><th>Label</th><th
					>Weakest component</th
				><th>Computed</th></tr
			></thead
		>
		<tbody>
			{#each data.rows as r (r.id)}
				{@const weakest = Object.entries(r.reputation.components).sort(
					(a, b) => a[1].score - b[1].score
				)[0]}
				<tr>
					<td
						><a href="/reputation/{data.side}/{encodeURIComponent(r.id)}">{r.name ?? r.id}</a
						>{#if r.name}<div class="muted small mono">{r.id}</div>{/if}</td
					>
					<td class="num">{pct(r.reputation.score)}</td>
					<td class="num">{pct(r.reputation.confidence)}</td>
					<td
						><Badge value={r.reputation.label} />{#if r.reputation.frozen}
							<Badge value="frozen" />{/if}</td
					>
					<td>{weakest ? `${weakest[0]} ${pct(weakest[1].score)}` : '—'}</td>
					<td>{ago(r.reputation.computed_at)}</td>
				</tr>
			{:else}
				<tr><td colspan="6" class="muted">No evidence yet on this side.</td></tr>
			{/each}
		</tbody>
	</table>
</div>

<style>
	.tabs {
		gap: var(--space-2);
	}
</style>
