<script lang="ts">
	import PageHeader from '#lib/components/PageHeader.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<PageHeader
	title="Access"
	description="Who may announce, discover, report and administer. Roles live in pacod-id, as for every pacod product; the broker's are defined in authz/broker.yaml."
/>

<section class="card stack">
	<h2>Providers and machines</h2>
	{#if data.identity}
		<p>
			Each provider is a pacod-id <strong>service account</strong> holding the role
			<code>broker.provider</code>
			(at the platform or in its organisation). It announces with its <code>pidt_…</code> token as the
			bearer.
		</p>
		<p>
			<a class="btn" href={data.identity.serviceAccounts} rel="noopener noreferrer" target="_blank"
				>Manage service accounts in pacod-id</a
			>
		</p>
		<p class="muted small">
			People get roles in pacod-id's administration ({data.identity.issuer}/admin).
		</p>
	{:else}
		<p>
			The console and the broker run in <strong>open mode</strong>: anyone who reaches the broker
			may announce and discover. For anything beyond a closed network, run both with pacod-id
			(BROKER_AUTH=oidc, BROKER_UI_AUTH=oidc).
		</p>
	{/if}
</section>

<section class="card stack">
	<h2>Roles</h2>
	<table>
		<thead><tr><th>Role</th><th>Held at</th><th>Permissions</th></tr></thead>
		<tbody>
			{#each data.roles as r (r.name)}
				<tr
					><td class="mono"
						>broker.{r.name}{#if r.implicit}
							<span class="muted small">(every person)</span>{/if}</td
					><td>{r.at.join(', ')}</td><td class="small">{r.permissions.join(', ')}</td></tr
				>
			{/each}
		</tbody>
	</table>
</section>
