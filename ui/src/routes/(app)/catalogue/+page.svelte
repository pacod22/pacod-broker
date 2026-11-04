<script lang="ts">
	import Badge from '#lib/components/Badge.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { money, pct } from '#lib/format.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
	const KINDS = ['model', 'mcp_server', 'agent', 'tool', 'compute', 'embedding', 'dataset'];
	const next = $derived.by(() => {
		if (!data.result.next_cursor) return null;
		const u = new URLSearchParams(data.query);
		u.set('cursor', data.result.next_cursor);
		return `?${u}`;
	});

	function firstRate(rates: Record<string, number | null | undefined> | undefined): string {
		const e = Object.entries(rates ?? {}).find(([, v]) => v != null);
		return e ? `${e[0]} ${money(e[1])}` : '—';
	}
</script>

<PageHeader
	title="Resources"
	description="Everything announced on the broker, as consumers would find it: available or not, priced or not."
/>

<form method="GET" class="card row filters">
	<label
		>Words <input name="q" value={data.query.q ?? ''} placeholder="llama, weather, H100…" /></label
	>
	<label
		>Kind
		<select name="kind" value={data.query.kind ?? ''}>
			<option value="">any</option>
			{#each KINDS as k (k)}<option value={k}>{k}</option>{/each}
		</select>
	</label>
	<label
		>Sort
		<select name="sort" value={data.query.sort ?? 'default'}>
			<option value="default">relevance / name</option>
			<option value="name">name</option>
			<option value="price:input">price: input (one kind)</option>
			<option value="price:per_gpu_hour">price: GPU hour (one kind)</option>
			<option value="ttft">time to first token (one kind)</option>
			<option value="reputation">reputation (one kind)</option>
		</select>
	</label>
	{#if data.query.provider}<input type="hidden" name="provider" value={data.query.provider} />{/if}
	<button class="btn btn-primary" type="submit">Search</button>
</form>

<p class="muted small">{data.result.total} resources · index generation {data.result.generation}</p>

<div class="table-wrap">
	<table>
		<thead>
			<tr
				><th>Resource</th><th>Kind</th><th>Provider</th><th>Status</th><th>Price now</th><th
					class="num">Reputation</th
				></tr
			>
		</thead>
		<tbody>
			{#each data.result.hits as h (h.id)}
				<tr>
					<td>
						<a href="/catalogue/{h.id}">{h.resource.title ?? h.resource.name}</a>
						<div class="muted small">{h.resource.summary}</div>
					</td>
					<td>{h.resource.kind}</td>
					<td
						><a href="/providers/{h.provider.id}">{h.provider.name}</a>{#if h.provider.verified}
							<Badge value="verified" />{/if}</td
					>
					<td><Badge value={h.available ? h.live.status : 'unavailable'} /></td>
					<td class="mono small"
						>{h.price ? (h.price.free ? 'free' : firstRate(h.price.rates)) : '—'}</td
					>
					<td class="num"
						>{h.reputation ? `${pct(h.reputation.score)} (${h.reputation.label})` : 'new'}</td
					>
				</tr>
			{:else}
				<tr><td colspan="6" class="muted">Nothing matches.</td></tr>
			{/each}
		</tbody>
	</table>
</div>

{#if next}<p><a class="btn" href={next}>Next page</a></p>{/if}

<style>
	.filters {
		flex-wrap: wrap;
		align-items: flex-end;
		gap: var(--space-3);
	}
</style>
