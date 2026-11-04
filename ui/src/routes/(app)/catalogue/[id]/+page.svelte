<script lang="ts">
	import Badge from '#lib/components/Badge.svelte';
	import JsonView from '#lib/components/JsonView.svelte';
	import Markdown from '#lib/components/Markdown.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import ReputationCard from '#lib/components/ReputationCard.svelte';
	import Timeline from '#lib/components/Timeline.svelte';
	import { ago, dateTime, money, pct, RATE_LABELS } from '#lib/format.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
	const r = $derived(data.resource);
	const t = $derived(data.timeline);
	const caps = $derived(
		Object.entries(r.capabilities ?? {})
			.filter(([k, v]) => v === true && k !== 'extra')
			.map(([k]) => k)
	);
	const periods = $derived((r.pricing?.periods ?? []) as any[]);
</script>

<PageHeader
	title={r.title ?? r.name}
	description={r.summary}
	crumbs={[{ label: 'Resources', href: '/catalogue' }]}
/>

<section class="row badges">
	<Badge value={r.kind} tone="neutral" />
	<Badge value={r.available_now ? r.live.status : 'unavailable'} />
	<Badge value={r.source} />
	{#each r.tags ?? [] as tag (tag)}<Badge value={tag} tone="neutral" />{/each}
</section>

<div class="grid">
	<div class="stack-lg">
		{#if r.description}
			<section class="card"><Markdown source={r.description} /></section>
		{/if}

		<section class="card stack">
			<h2>Next seven days</h2>
			<Timeline
				from={t.from}
				to={t.to}
				windows={t.availability.windows}
				changes={t.prices?.changes ?? []}
			/>
			<p class="small">
				Available {pct(t.availability.available_share)} of the time{#if t.availability.min_capacity},
					with at least {t.availability.min_capacity} free when available{/if}.
			</p>
			{#if t.prices}
				<table>
					<thead
						><tr
							><th>Rate ({t.prices.currency})</th><th class="num">Lowest</th><th class="num"
								>Mean</th
							><th class="num">Highest</th></tr
						></thead
					>
					<tbody>
						{#each Object.entries(t.prices.rates) as [name, v] (name)}
							<tr
								><td>{RATE_LABELS[name] ?? name}</td><td class="num">{money(v.min)}</td><td
									class="num">{money(v.avg)}</td
								><td class="num">{money(v.max)}</td></tr
							>
						{/each}
					</tbody>
				</table>
			{/if}
		</section>

		<section class="card stack">
			<h2>Interfaces</h2>
			<div class="table-wrap">
				<table>
					<thead><tr><th>Protocol</th><th>Address</th><th>Auth</th></tr></thead>
					<tbody>
						{#each r.interfaces ?? [] as f (f.url + f.protocol)}
							<tr
								><td>{f.protocol}</td><td class="mono small">{f.url}</td><td
									>{f.auth?.scheme ?? 'none'}{#if f.auth?.obtain_url}
										· <a href={f.auth.obtain_url} rel="noopener noreferrer">get credentials</a
										>{/if}</td
								></tr
							>
						{:else}
							<tr><td colspan="3" class="muted">No interface announced.</td></tr>
						{/each}
					</tbody>
				</table>
			</div>
		</section>

		{#if r.examples?.length}
			<section class="card stack">
				<h2>Examples</h2>
				{#each r.examples as ex (ex.title)}
					<h3>{ex.title}</h3>
					<pre><code>{ex.code}</code></pre>
				{/each}
			</section>
		{/if}

		{#if periods.length}
			<section class="card stack">
				<h2>Price periods</h2>
				{#each periods as p (p.valid_from)}
					<div class="stack">
						<p>
							<strong>From {dateTime(p.valid_from)}</strong>{#if p.valid_to}
								to {dateTime(p.valid_to)}{/if}
						</p>
						<p class="mono small">
							{Object.entries(p.rates)
								.map(([k, v]) => `${k} ${v}`)
								.join(' · ')}
						</p>
						{#each p.windows ?? [] as w (w.name)}<p class="small">
								Window <strong>{w.name}</strong>
								{w.from_time}–{w.to_time}
								{(w.weekdays ?? []).join(' ')}:
								<span class="mono"
									>{Object.entries(w.rates)
										.map(([k, v]) => `${k} ${v}`)
										.join(' · ')}</span
								>
							</p>{/each}
						{#each p.overrides ?? [] as o (o.from)}<p class="small">
								Override {dateTime(o.from)}–{dateTime(o.to)}{#if o.reason}
									({o.reason}){/if}
							</p>{/each}
						{#each p.tiers ?? [] as tier (tier.above)}<p class="small">
								Above {tier.above}
								{tier.unit}:
								<span class="mono"
									>{Object.entries(tier.rates)
										.map(([k, v]) => `${k} ${v}`)
										.join(' · ')}</span
								>
							</p>{/each}
					</div>
				{/each}
			</section>
		{/if}
	</div>

	<div class="stack-lg">
		<ReputationCard reputation={r.reputation} />
		<section class="card stack">
			<h2>Facts</h2>
			<dl class="facts">
				<dt>Id</dt>
				<dd class="mono small">{r.id}</dd>
				<dt>Provider</dt>
				<dd><a href="/providers/{r.provider.id}">{r.provider.name}</a></dd>
				<dt>Key</dt>
				<dd class="mono">{r.key}</dd>
				{#if r.model}
					<dt>Model</dt>
					<dd>
						{r.model.family ?? ''}
						{r.model.parameters_b ? `${r.model.parameters_b}B` : ''}
						{r.model.quantisation ?? ''}
					</dd>
					{#if r.model.context_window}<dt>Context</dt>
						<dd>{r.model.context_window.toLocaleString('en')}</dd>{/if}
				{/if}
				{#if caps.length}<dt>Capabilities</dt>
					<dd>{caps.join(', ')}</dd>{/if}
				{#if r.capabilities?.mcp_tools?.length}<dt>MCP tools</dt>
					<dd>{r.capabilities.mcp_tools.map((x: any) => x.name).join(', ')}</dd>{/if}
				{#if r.availability?.regions?.length}<dt>Regions</dt>
					<dd>{r.availability.regions.join(', ')}</dd>{/if}
				{#if r.policy?.residency?.length}<dt>Residency</dt>
					<dd>{r.policy.residency.join(', ')}</dd>{/if}
				{#if r.policy?.training_use !== undefined}<dt>Training use</dt>
					<dd>{r.policy.training_use ? 'yes' : 'no'}</dd>{/if}
				<dt>Visibility</dt>
				<dd>
					{r.access?.visibility}{#if r.access?.min_client_reputation}
						· clients from {pct(r.access.min_client_reputation)}{/if}
				</dd>
				<dt>In use</dt>
				<dd>
					{r.live.in_use ?? '—'}{#if r.capacity_now != null}
						· {r.capacity_now} free now{/if}
				</dd>
				<dt>Lease</dt>
				<dd>
					{#if r.lease}<span class="mono small">{r.lease.id}</span>, expires {ago(
							r.lease.expires_at
						)}{:else}static{/if}
				</dd>
				<dt>Updated</dt>
				<dd>{ago(r.updated_at)}</dd>
			</dl>
			{#if r.runs_on}
				<p class="small">
					Runs on {[...(r.runs_on.hardware ?? []), ...(r.runs_on.software ?? [])].join(', ')} — see the
					<a href="/providers/{r.provider.id}#inventory">provider's inventory</a>.
				</p>
			{/if}
		</section>
		<JsonView value={r} />
	</div>
</div>

<style>
	.badges {
		flex-wrap: wrap;
		gap: var(--space-2);
	}

	.grid > :global(*) {
		min-width: 0;
	}

	.grid {
		display: grid;
		grid-template-columns: minmax(0, 2fr) minmax(18rem, 1fr);
		gap: var(--space-4);
		align-items: start;
	}

	pre {
		overflow-x: auto;
		padding: var(--space-3);
		border-radius: var(--radius);
		background: var(--surface-2);
	}

	@media (max-width: 64rem) {
		.grid {
			grid-template-columns: 1fr;
		}
	}
</style>
