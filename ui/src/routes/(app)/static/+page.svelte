<script lang="ts">
	import Notice from '#lib/components/Notice.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import type { PageProps } from './$types';

	let { data, form }: PageProps = $props();
	const EXAMPLE = JSON.stringify(
		{
			provider: {
				name: 'my-catalogue',
				summary: 'Resources I list by hand',
				website: 'https://example.com'
			},
			resources: [
				{
					key: 'weather',
					kind: 'mcp_server',
					name: 'weather',
					summary: 'Forecasts as MCP tools',
					interfaces: [{ protocol: 'mcp-streamable-http', url: 'https://tools.example.com/mcp' }],
					pricing: { free: true },
					access: { visibility: 'public' }
				}
			]
		},
		null,
		2
	);
</script>

<PageHeader
	title="Static entries"
	description="Resources listed by hand rather than announced: they never expire. The catalogue file (BROKER_CATALOG_FILE) holds more of them; entries saved here are kept in BROKER_STATE_DIR."
/>

{#if form?.message}<Notice tone="error">{form.message}</Notice>{/if}
{#if form?.done}<Notice tone="success">{form.done}</Notice>{/if}

<section class="card stack">
	<h2>Listed</h2>
	<table>
		<thead
			><tr><th>Provider</th><th>Principal</th><th class="num">Resources</th><th></th></tr></thead
		>
		<tbody>
			{#each data.entries as e (e.principal + e.provider.name)}
				<tr>
					<td
						>{e.provider.name}{#if e.verified}
							(verified){/if}</td
					>
					<td class="mono small">{e.principal || `static:${e.provider.name}`}</td>
					<td class="num">{e.resources.length}</td>
					<td>
						<details>
							<summary>Edit or remove</summary>
							<form method="POST" action="?/put" class="stack">
								<textarea name="json" rows="12" class="mono">{JSON.stringify(e, null, 2)}</textarea>
								<button class="btn" type="submit">Save</button>
							</form>
						</details>
					</td>
				</tr>
			{:else}
				<tr><td colspan="4" class="muted">No static entry.</td></tr>
			{/each}
		</tbody>
	</table>
</section>

<section class="card stack">
	<h2>Add or replace a provider's entries</h2>
	<p class="muted small">
		The same document as an announcement, without the lease: a provider and its resources. Saving a
		provider again replaces its entries; prices already in force cannot change.
	</p>
	<form method="POST" action="?/put" class="stack">
		<textarea name="json" rows="18" class="mono">{form?.json ?? EXAMPLE}</textarea>
		<button class="btn btn-primary" type="submit">Save</button>
	</form>
</section>
