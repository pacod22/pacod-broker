<script lang="ts">
	import JsonView from '#lib/components/JsonView.svelte';
	import Notice from '#lib/components/Notice.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import ReputationCard from '#lib/components/ReputationCard.svelte';
	import type { PageProps } from './$types';

	let { data, form }: PageProps = $props();
	const link = $derived(
		data.side === 'resource'
			? `/catalogue/${data.id}`
			: data.side === 'provider'
				? `/providers/${data.id}`
				: null
	);
</script>

<PageHeader
	title={data.id}
	description="A score and the evidence it rests on."
	crumbs={[{ label: 'Reputation', href: `/reputation?side=${data.side}` }]}
/>

{#if form?.message}<Notice tone="error">{form.message}</Notice>{/if}
{#if form?.done}<Notice tone="success">{form.done}</Notice>{/if}

{#if link}<p><a href={link}>Open the {data.side}</a></p>{/if}

<ReputationCard reputation={data.one.reputation} title="Score" />

<section class="card stack">
	<h2>Moderation</h2>
	<p class="muted small">
		Freezing holds the score while something is looked into; resetting removes the evidence, and is
		recorded.
	</p>
	<div class="row actions">
		{#if data.one.reputation?.frozen}
			<form method="POST" action="?/unfreeze">
				<button class="btn" type="submit">Unfreeze</button>
			</form>
		{:else}
			<form method="POST" action="?/freeze"><button class="btn" type="submit">Freeze</button></form>
		{/if}
		<form method="POST" action="?/reset">
			<button class="btn btn-danger" type="submit">Reset</button>
		</form>
	</div>
</section>

<JsonView value={data.one.evidence} summary="Evidence: the decayed aggregates" />

<style>
	.actions {
		gap: var(--space-2);
	}
</style>
