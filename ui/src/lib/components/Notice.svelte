<script lang="ts">
	let {
		tone = 'info',
		children
	}: { tone?: 'info' | 'error' | 'success'; children: import('svelte').Snippet } = $props();
</script>

<p class="notice {tone}" role={tone === 'error' ? 'alert' : 'status'}>{@render children()}</p>

<style>
	.notice {
		padding: var(--space-3) var(--space-4);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--surface);
	}

	.notice.error {
		border-color: var(--danger, #c0392b);
		color: var(--danger, #c0392b);
	}

	.notice.success {
		border-color: var(--success, #2f9e6b);
	}
</style>
