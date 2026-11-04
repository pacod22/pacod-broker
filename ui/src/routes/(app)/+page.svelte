<script lang="ts">
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { ago, dateTime } from '#lib/format.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
	const s = $derived(data.status);
</script>

<PageHeader
	title="Overview"
	description="What the broker holds now. Providers announce under leases; what is not renewed disappears by itself."
/>

{#if s}
	<section class="tiles">
		<div class="card">
			<p class="eyebrow">Resources</p>
			<p class="figure">{s.resources}</p>
		</div>
		<div class="card">
			<p class="eyebrow">Providers</p>
			<p class="figure">{s.providers}</p>
		</div>
		<div class="card">
			<p class="eyebrow">Leases</p>
			<p class="figure">{s.leases}</p>
		</div>
		<div class="card">
			<p class="eyebrow">Clients rated</p>
			<p class="figure">{s.clients_rated}</p>
		</div>
	</section>

	<section class="grid">
		<div class="card stack">
			<h2>By kind</h2>
			{#if Object.keys(s.by_kind).length === 0}
				<p class="muted">
					Nothing announced yet. A provider announces with <code>PUT /v1/announce</code>; see
					examples/provider.sh.
				</p>
			{:else}
				<table>
					<tbody>
						{#each Object.entries(s.by_kind) as [kind, n] (kind)}
							<tr><td><a href="/catalogue?kind={kind}">{kind}</a></td><td class="num">{n}</td></tr>
						{/each}
					</tbody>
				</table>
			{/if}
		</div>
		<div class="card stack">
			<h2>Live status</h2>
			<table>
				<tbody>
					{#each Object.entries(s.by_status) as [st, n] (st)}
						<tr><td>{st}</td><td class="num">{n}</td></tr>
					{/each}
				</tbody>
			</table>
		</div>
		<div class="card stack">
			<h2>The broker</h2>
			<dl class="facts">
				<dt>Id</dt>
				<dd class="mono">{s.broker_id}</dd>
				<dt>Version</dt>
				<dd>{s.version}</dd>
				<dt>Started</dt>
				<dd>{dateTime(s.started_at)} ({ago(s.started_at)})</dd>
				<dt>Index</dt>
				<dd>generation {s.generation}, built {ago(s.index_built_at)}</dd>
				<dt>Reputation</dt>
				<dd>computed {ago(s.reputation_computed_at)}</dd>
				<dt>Access</dt>
				<dd>
					{s.auth}{#if s.identity_ready === false}
						(waiting for pacod-id){/if}
				</dd>
				<dt>State kept</dt>
				<dd>{s.state_dir ? 'yes (BROKER_STATE_DIR)' : 'no: memory only'}</dd>
			</dl>
		</div>
	</section>

	{#if data.pending.length > 0}
		<section class="card stack">
			<h2>Waiting for approval</h2>
			<p class="muted">
				These providers announced for the first time; BROKER_APPROVAL=new_providers holds them until
				approved.
			</p>
			<ul>
				{#each data.pending as p (p.provider_id)}
					<li>
						<a href="/providers/{p.provider_id}">{p.profile.name}</a> · {p.resources} resources · {ago(
							p.first_seen
						)}
					</li>
				{/each}
			</ul>
		</section>
	{/if}
{/if}

<style>
	.tiles,
	.grid {
		display: grid;
		gap: var(--space-4);
	}

	.tiles {
		grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
	}

	.grid {
		grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
	}

	.figure {
		font-family: var(--font-display);
		font-size: var(--text-xl);
		font-weight: 800;
	}
</style>
