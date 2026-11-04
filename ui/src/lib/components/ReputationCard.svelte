<script lang="ts">
	/** A reputation: the score, how sure the broker is, and each component with its weight. */
	import Badge from './Badge.svelte';
	import { pct } from '#lib/format.ts';

	interface Component {
		score: number;
		weight: number;
		samples: number;
	}

	interface Reputation {
		score: number;
		confidence: number;
		label: string;
		components: Record<string, Component>;
		computed_at: string;
		frozen?: boolean;
	}

	let {
		reputation,
		title = 'Reputation'
	}: { reputation: Reputation | null | undefined; title?: string } = $props();

	const NAMES: Record<string, string> = {
		availability: 'Availability',
		reliability: 'Reliability',
		performance: 'Performance (within its kind)',
		accuracy: 'Accuracy of claims',
		lease_discipline: 'Lease discipline',
		stability: 'Stability',
		quality: 'Quality ratings',
		trust: 'Trust',
		identity: 'Identity assurance',
		conduct: 'Conduct',
		settlement: 'Settlement',
		commitment: 'Commitment keeping',
		feedback_honesty: 'Feedback honesty',
		tenure: 'Tenure'
	};
</script>

<section class="card stack" aria-label={title}>
	<div class="row head">
		<h2>{title}</h2>
		{#if reputation}
			<Badge value={reputation.label} />
			{#if reputation.frozen}<Badge value="frozen" />{/if}
		{/if}
	</div>
	{#if !reputation}
		<p class="muted">No evidence yet: the broker has seen nothing about this subject.</p>
	{:else}
		<p class="big">
			<span class="score">{pct(reputation.score)}</span>
			<span class="muted small">confidence {pct(reputation.confidence)}</span>
		</p>
		<ul class="components">
			{#each Object.entries(reputation.components) as [name, c] (name)}
				<li>
					<span class="name">
						{NAMES[name] ?? name}
						<span class="muted mono meta" title="weight in the score · evidence counted"
							>weight {c.weight} · evidence {c.samples < 10
								? c.samples.toFixed(1)
								: Math.round(c.samples)}</span
						>
					</span>
					<span class="meter" aria-hidden="true"
						><span style="width: {Math.round(c.score * 100)}%"></span></span
					>
					<span class="value mono">{pct(c.score)}</span>
				</li>
			{/each}
		</ul>
	{/if}
</section>

<style>
	.head {
		align-items: center;
		gap: var(--space-2);
	}

	.big {
		display: flex;
		align-items: baseline;
		gap: var(--space-3);
	}

	.score {
		font-family: var(--font-display);
		font-size: var(--text-xl);
		font-weight: 800;
	}

	.components {
		display: grid;
		gap: var(--space-2);
		padding: 0;
		list-style: none;
	}

	.components li {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(3rem, 6rem) 2.75rem;
		align-items: center;
		gap: var(--space-2);
	}

	.meta {
		display: block;
		font-size: var(--text-xs);
	}

	.meter {
		height: 0.5rem;
		overflow: hidden;
		border-radius: 999px;
		background: var(--surface-2);
	}

	.meter span {
		display: block;
		height: 100%;
		background: var(--accent);
	}

	@media (max-width: 40rem) {
		.components li {
			grid-template-columns: 1fr 5rem 3rem;
		}

		.components li > :last-child {
			display: none;
		}
	}
</style>
