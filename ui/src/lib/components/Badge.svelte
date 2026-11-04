<script lang="ts">
	/** A state in a word. The tone follows the word unless it is given. */
	interface Props {
		value: string;
		tone?: 'good' | 'bad' | 'warn' | 'neutral';
	}

	let { value, tone }: Props = $props();

	const TONES: Record<string, NonNullable<Props['tone']>> = {
		// Live status of resources.
		active: 'good',
		degraded: 'warn',
		draining: 'warn',
		maintenance: 'warn',
		// Reputation labels.
		excellent: 'good',
		good: 'good',
		fair: 'neutral',
		low: 'bad',
		new: 'neutral',
		// Sources and states.
		lease: 'neutral',
		static: 'good',
		verified: 'good',
		blocked: 'bad',
		pending: 'warn',
		frozen: 'warn',
		available: 'good',
		unavailable: 'bad'
	};

	const resolved = $derived(tone ?? TONES[value] ?? 'neutral');
</script>

<span class="badge {resolved}">{value.replaceAll('_', ' ')}</span>

<style>
	.badge {
		display: inline-block;
		padding: 0.0625rem 0.5625rem;
		border: 1px solid color-mix(in srgb, var(--tone) 35%, transparent);
		border-radius: 999px;
		background: var(--tone-soft);
		color: var(--tone);
		font-size: var(--text-xs);
		font-weight: 600;
		line-height: 1.5;
		white-space: nowrap;
	}

	.good {
		--tone: var(--success);
		--tone-soft: var(--success-soft);
	}

	.bad {
		--tone: var(--danger);
		--tone-soft: var(--danger-soft);
	}

	.warn {
		--tone: var(--warning);
		--tone-soft: var(--warning-soft);
	}

	.neutral {
		--tone: var(--text-muted);
		--tone-soft: var(--surface-2);
	}
</style>
