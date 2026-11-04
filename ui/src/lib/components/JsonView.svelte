<script lang="ts">
	/** Raw JSON, folded, for those who want every field. */
	let { value, summary = 'Raw JSON' }: { value: unknown; summary?: string } = $props();
</script>

<details class="json">
	<summary>{summary}</summary>
	<pre><code>{JSON.stringify(value, null, 2)}</code></pre>
</details>

<style>
	pre {
		overflow-x: auto;
		max-height: 32rem;
		padding: var(--space-3);
		border-radius: var(--radius);
		background: var(--surface-2);
		font-size: var(--text-sm);
	}
</style>
