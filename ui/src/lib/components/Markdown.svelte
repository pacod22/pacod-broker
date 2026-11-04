<script lang="ts">
	/** A provider's Markdown description, rendered safely (lib/markdown.ts escapes everything first). */
	import { renderMarkdown } from '#lib/markdown.ts';

	let { source }: { source: string } = $props();
	const html = $derived(renderMarkdown(source));
</script>

<div class="markdown">
	<!-- eslint-disable-next-line svelte/no-at-html-tags -- escaped by renderMarkdown -->
	{@html html}
</div>

<style>
	.markdown :global(h3),
	.markdown :global(h4),
	.markdown :global(h5) {
		margin: var(--space-4) 0 var(--space-2);
	}

	.markdown :global(p),
	.markdown :global(ul),
	.markdown :global(ol),
	.markdown :global(pre) {
		margin: 0 0 var(--space-3);
	}

	.markdown :global(pre) {
		overflow-x: auto;
		padding: var(--space-3);
		border-radius: var(--radius);
		background: var(--surface-2);
		font-family: var(--font-mono);
		font-size: var(--text-sm);
	}

	.markdown :global(pre code) {
		padding: 0;
		background: none;
	}
</style>
