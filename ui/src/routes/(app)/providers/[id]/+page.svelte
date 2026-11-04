<script lang="ts">
	import Badge from '#lib/components/Badge.svelte';
	import JsonView from '#lib/components/JsonView.svelte';
	import Markdown from '#lib/components/Markdown.svelte';
	import Notice from '#lib/components/Notice.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import ReputationCard from '#lib/components/ReputationCard.svelte';
	import { ago, dateTime } from '#lib/format.ts';
	import type { PageProps } from './$types';

	let { data, form }: PageProps = $props();
	const p = $derived(data.provider);
	const prof = $derived(p.profile);
	const inv = $derived(prof.inventory);
</script>

<PageHeader
	title={prof.title ?? prof.name}
	description={prof.summary}
	crumbs={[{ label: 'Providers', href: '/providers' }]}
/>

{#if form?.message}<Notice tone="error">{form.message}</Notice>{/if}
{#if form?.done}<Notice tone="success">{form.done}</Notice>{/if}

<section class="row badges">
	<Badge value={p.source} />
	{#if p.verified}<Badge value="verified" />{/if}
	{#if p.blocked}<Badge value="blocked" />{/if}
</section>

<div class="grid">
	<div class="stack-lg">
		{#if prof.description}<section class="card">
				<Markdown source={prof.description} />
			</section>{/if}

		<section class="card stack" id="inventory">
			<h2>Inventory</h2>
			{#if !inv}
				<p class="muted">The provider lists no hardware or software.</p>
			{:else}
				<p class="muted small">Declared by the provider; visible to: {inv.visibility}.</p>
				{#if inv.hardware?.length}
					<h3>Hardware</h3>
					<table>
						<thead
							><tr
								><th>Id</th><th>Type</th><th>Vendor / model</th><th class="num">Count</th><th
									class="num">Memory GB</th
								><th>Interconnect</th><th>Region</th></tr
							></thead
						>
						<tbody>
							{#each inv.hardware as h (h.id)}
								<tr
									><td class="mono">{h.id}</td><td>{h.type}</td><td
										>{h.vendor ?? ''} {h.model ?? ''}</td
									><td class="num">{h.count}</td><td class="num">{h.memory_gb ?? '—'}</td><td
										>{h.interconnect ?? '—'}</td
									><td>{h.region ?? '—'}</td></tr
								>
							{/each}
						</tbody>
					</table>
				{/if}
				{#if inv.software?.length}
					<h3>Software</h3>
					<table>
						<thead><tr><th>Id</th><th>Kind</th><th>Name</th><th>Version</th></tr></thead>
						<tbody>
							{#each inv.software as s (s.id)}
								<tr
									><td class="mono">{s.id}</td><td>{s.kind}</td><td>{s.name}</td><td class="mono"
										>{s.version ?? '—'}</td
									></tr
								>
							{/each}
						</tbody>
					</table>
				{/if}
			{/if}
		</section>

		<section class="card stack">
			<h2>Resources</h2>
			<ul>
				{#each p.resources as r (r.id)}
					<li>
						<a href="/catalogue/{r.id}">{r.name}</a>
						<span class="muted small">{r.kind} · {r.summary}</span>
					</li>
				{:else}
					<li class="muted">None now.</li>
				{/each}
			</ul>
		</section>

		<section class="card stack">
			<h2>Leases</h2>
			<table>
				<thead
					><tr
						><th>Lease</th><th class="num">TTL s</th><th>Last heartbeat</th><th>Expires</th><th
						></th></tr
					></thead
				>
				<tbody>
					{#each p.leases as l (l.id)}
						<tr>
							<td class="mono small">{l.id}</td><td class="num">{l.ttl_s}</td><td
								>{ago(l.last_heartbeat)}</td
							><td>{ago(l.expires_at)}</td>
							<td
								><form method="POST" action="?/evict">
									<input type="hidden" name="lease" value={l.id} /><button
										class="btn-quiet danger"
										type="submit">End</button
									>
								</form></td
							>
						</tr>
					{:else}
						<tr
							><td colspan="5" class="muted"
								>No lease: {p.source === 'static'
									? 'static entries never expire'
									: 'nothing announced now'}.</td
							></tr
						>
					{/each}
				</tbody>
			</table>
		</section>
	</div>

	<div class="stack-lg">
		<ReputationCard reputation={p.reputation} />
		<section class="card stack">
			<h2>Facts</h2>
			<dl class="facts">
				<dt>Id</dt>
				<dd class="mono small">{p.id}</dd>
				<dt>Principal</dt>
				<dd class="mono small">{p.principal}</dd>
				{#if p.organisation_id}<dt>Organisation</dt>
					<dd class="mono small">{p.organisation_id}</dd>{/if}
				{#if prof.website}<dt>Website</dt>
					<dd><a href={prof.website} rel="noopener noreferrer">{prof.website}</a></dd>{/if}
				{#if prof.contact}<dt>Contact</dt>
					<dd>{prof.contact}</dd>{/if}
				{#if prof.locations?.length}<dt>Locations</dt>
					<dd>{prof.locations.join(', ')}</dd>{/if}
				{#each prof.certifications ?? [] as c (c.name)}<dt>Certification</dt>
					<dd>
						{c.name} ({c.assurance}){#if c.valid_to}, to {dateTime(c.valid_to)}{/if}
					</dd>{/each}
				<dt>Registered</dt>
				<dd>{ago(p.registered_at)}</dd>
			</dl>
		</section>
		<section class="card stack">
			<h2>Actions</h2>
			<div class="row actions">
				{#if p.blocked}
					<form method="POST" action="?/unblock">
						<button class="btn" type="submit">Unblock</button>
					</form>
				{:else}
					<form method="POST" action="?/block">
						<button class="btn btn-danger" type="submit">Block</button>
					</form>
				{/if}
				<form method="POST" action="?/approve">
					<button class="btn" type="submit">Approve</button>
				</form>
				{#if p.verified}
					<form method="POST" action="?/unverify">
						<button class="btn" type="submit">Withdraw verification</button>
					</form>
				{:else}
					<form method="POST" action="?/verify">
						<button class="btn" type="submit">Verify</button>
					</form>
				{/if}
			</div>
			<form method="POST" action="?/incident" class="stack">
				<label
					>Record an upheld incident <textarea
						name="note"
						rows="2"
						required
						placeholder="What happened, and how it was established"></textarea></label
				>
				<button class="btn" type="submit">Record incident</button>
			</form>
		</section>
		<JsonView value={p} />
	</div>
</div>

<style>
	.badges,
	.actions {
		flex-wrap: wrap;
		gap: var(--space-2);
	}

	.grid > :global(*) {
		min-width: 0;
	}

	.grid :global(table) {
		display: block;
		overflow-x: auto;
	}

	.grid {
		display: grid;
		grid-template-columns: minmax(0, 2fr) minmax(18rem, 1fr);
		gap: var(--space-4);
		align-items: start;
	}

	@media (max-width: 64rem) {
		.grid {
			grid-template-columns: 1fr;
		}
	}
</style>
