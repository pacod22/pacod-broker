<script lang="ts">
	import Badge from '#lib/components/Badge.svelte';
	import Notice from '#lib/components/Notice.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { ago } from '#lib/format.ts';
	import type { PageProps } from './$types';

	let { data, form }: PageProps = $props();
	const notes = $derived(
		new Map(data.reports.notes.filter((n) => n.report_id).map((n) => [n.report_id, n.text]))
	);
</script>

<PageHeader
	title="Moderation"
	description="Reports providers made about clients, and what people wrote with their ratings. A report found unfounded can be taken back."
/>

{#if form?.message}<Notice tone="error">{form.message}</Notice>{/if}
{#if form?.done}<Notice tone="success">{form.done}</Notice>{/if}

<section class="card stack">
	<h2>Reports about clients</h2>
	<table>
		<thead><tr><th>When</th><th>Client</th><th>Report</th><th>Details</th><th></th></tr></thead>
		<tbody>
			{#each data.reports.client_reports as r (r.id)}
				<tr>
					<td>{ago(r.at)}</td>
					<td
						><a href="/reputation/client/{encodeURIComponent(r.client)}" class="mono small"
							>{r.client}</a
						></td
					>
					<td><Badge value={r.kind} tone={r.kind === 'good_standing' ? 'good' : 'bad'} /></td>
					<td class="small">{notes.get(r.id) ?? '—'}</td>
					<td
						><form method="POST" action="?/reject">
							<input type="hidden" name="id" value={r.id} /><button
								class="btn-quiet danger"
								type="submit">Take back</button
							>
						</form></td
					>
				</tr>
			{:else}
				<tr><td colspan="5" class="muted">No report counts now.</td></tr>
			{/each}
		</tbody>
	</table>
</section>

<section class="card stack">
	<h2>Notes</h2>
	<p class="muted small">
		The last thousand comments, in memory only; never shown to anyone but admins.
	</p>
	<ul class="notes">
		{#each data.reports.notes as n, i (i)}
			<li>
				<span class="muted small">{ago(n.at)} · {n.kind} · {n.author} on {n.subject}</span>
				<p>{n.text}</p>
			</li>
		{:else}
			<li class="muted">Nothing written yet.</li>
		{/each}
	</ul>
</section>

<style>
	.notes {
		display: grid;
		gap: var(--space-3);
		padding: 0;
		list-style: none;
	}
</style>
