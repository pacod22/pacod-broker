<script lang="ts">
	import Badge from '#lib/components/Badge.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { ago, pct } from '#lib/format.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<PageHeader
	title="Providers"
	description="Who announces resources: their leases, inventories and reputation."
/>

{#if data.pending.length}
	<section class="card stack">
		<h2>Waiting for approval</h2>
		<ul>
			{#each data.pending as p (p.provider_id)}
				<li>
					<a href="/providers/{p.provider_id}">{p.profile.name}</a> ({p.principal}) · {p.resources} resources
					· first seen {ago(p.first_seen)}
				</li>
			{/each}
		</ul>
	</section>
{/if}

<div class="table-wrap">
	<table>
		<thead
			><tr
				><th>Provider</th><th>Source</th><th class="num">Resources</th><th class="num">Leases</th
				><th>Inventory</th><th class="num">Reputation</th><th>Updated</th></tr
			></thead
		>
		<tbody>
			{#each data.providers as p (p.id)}
				<tr>
					<td>
						<a href="/providers/{p.id}">{p.title ?? p.name}</a>
						{#if p.verified}<Badge value="verified" />{/if}
						{#if p.blocked}<Badge value="blocked" />{/if}
						<div class="muted small">{p.summary}</div>
					</td>
					<td><Badge value={p.source} /></td>
					<td class="num">{p.resources}</td>
					<td class="num">{p.leases}</td>
					<td>{p.has_inventory ? 'yes' : '—'}</td>
					<td class="num"
						>{p.reputation ? `${pct(p.reputation.score)} (${p.reputation.label})` : 'new'}</td
					>
					<td>{ago(p.updated_at)}</td>
				</tr>
			{:else}
				<tr><td colspan="7" class="muted">No provider has announced yet.</td></tr>
			{/each}
		</tbody>
	</table>
</div>
