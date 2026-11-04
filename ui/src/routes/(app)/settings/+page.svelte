<script lang="ts">
	import JsonView from '#lib/components/JsonView.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
	const c = $derived(data.config);
</script>

<PageHeader
	title="Settings"
	description="The broker's effective configuration, read only: it comes from environment variables (docs/runbook.md), and secrets are never shown."
/>

{#if c.warnings?.length}
	<section class="card stack">
		<h2>Warnings</h2>
		<ul>
			{#each c.warnings as w (w)}<li>{w}</li>{/each}
		</ul>
	</section>
{/if}

<section class="card stack">
	<h2>Reputation weights</h2>
	<div class="row weights">
		{#each ['provider_weights', 'client_weights'] as side (side)}
			<table>
				<thead
					><tr
						><th>{side === 'provider_weights' ? 'Provider and resource' : 'Client'}</th><th
							class="num">Weight</th
						></tr
					></thead
				>
				<tbody
					>{#each Object.entries(c.reputation[side]) as [k, v] (k)}<tr
							><td>{k}</td><td class="num">{v}</td></tr
						>{/each}</tbody
				>
			</table>
		{/each}
	</div>
	<p class="muted small">
		Half-life {c.reputation.half_life_days} days · prior weight {c.reputation.prior_weight} · recomputed
		every {c.reputation.every_seconds} s.
	</p>
</section>

<section class="card stack">
	<h2>Brokers this console docks to</h2>
	<ul>
		{#each data.brokers as b (b.name)}<li>
				<strong>{b.name}</strong> <span class="mono small">{b.origin}</span>
			</li>{/each}
	</ul>
</section>

<JsonView value={c} summary="Everything" />

<style>
	.weights {
		flex-wrap: wrap;
		gap: var(--space-4);
		align-items: flex-start;
	}
</style>
