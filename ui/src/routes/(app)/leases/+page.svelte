<script lang="ts">
	import Notice from '#lib/components/Notice.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { ago } from '#lib/format.ts';
	import type { PageProps } from './$types';

	let { data, form }: PageProps = $props();
</script>

<PageHeader
	title="Leases"
	description="Every lease a provider holds. A lease not renewed by a heartbeat ends by itself, and its resources disappear."
/>

{#if form?.message}<Notice tone="error">{form.message}</Notice>{/if}
{#if form?.done}<Notice tone="success">{form.done}</Notice>{/if}

<div class="table-wrap">
	<table>
		<thead
			><tr
				><th>Lease</th><th>Provider</th><th class="num">Resources</th><th class="num">TTL s</th><th
					>Last heartbeat</th
				><th>Expires</th><th></th></tr
			></thead
		>
		<tbody>
			{#each data.leases as l (l.id)}
				<tr>
					<td class="mono small">{l.id}</td>
					<td><a href="/providers/{l.provider_id}">{l.principal}</a></td>
					<td class="num">{l.resources.length}</td>
					<td class="num">{l.ttl_s}</td>
					<td>{ago(l.last_heartbeat)}</td>
					<td>{ago(l.expires_at)}</td>
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
				<tr><td colspan="7" class="muted">No lease is open.</td></tr>
			{/each}
		</tbody>
	</table>
</div>
