<script lang="ts">
	/**
	 * When a resource is available and what it costs over a span: one band of availability
	 * windows, one band of price variants, day ticks. SVG drawn on the server; nothing to load.
	 */
	import { bars, dayTicks, priceSpans } from '#lib/timeline.ts';

	interface Props {
		from: string;
		to: string;
		windows: { from: string; to: string; capacity?: number | null }[];
		changes?: { at: string; source: string }[];
	}

	let { from, to, windows, changes = [] }: Props = $props();
	const W = 700;
	const avail = $derived(
		bars(
			windows.map((w) => ({ ...w, label: w.capacity == null ? '' : `capacity ${w.capacity}` })),
			from,
			to,
			W
		)
	);
	const prices = $derived(bars(priceSpans(changes, to), from, to, W));
	const ticks = $derived(dayTicks(from, to, W));
	const tone = (label: string) =>
		label === 'base'
			? 'base'
			: label.startsWith('window')
				? 'window'
				: label === 'unpriced'
					? 'none'
					: 'override';
</script>

<figure class="timeline">
	<svg
		viewBox="0 0 {W + 90} 92"
		role="img"
		aria-label="Availability and prices from {from} to {to}"
	>
		<text x="0" y="22" class="lbl">Available</text>
		<rect x="90" y="10" width={W} height="18" class="track" />
		{#each avail as b, i (i)}
			<rect x={90 + b.x} y="10" width={b.width} height="18" class="avail"
				><title>{b.label}</title></rect
			>
		{/each}
		<text x="0" y="52" class="lbl">Price</text>
		<rect x="90" y="40" width={W} height="18" class="track" />
		{#each prices as b, i (i)}
			<rect x={90 + b.x} y="40" width={b.width} height="18" class="price {tone(b.label)}"
				><title>{b.label}</title></rect
			>
		{/each}
		{#each ticks as t (t.label)}
			<line x1={90 + t.x} x2={90 + t.x} y1="6" y2="62" class="tick" />
			<text x={90 + t.x + 3} y="78" class="day">{t.label}</text>
		{/each}
	</svg>
	<figcaption class="muted small">
		Green: available. Price band: <span class="key base"></span> base,
		<span class="key window"></span> recurring window, <span class="key override"></span> one-off
		price,
		<span class="key none"></span> unpriced. Times in UTC.
	</figcaption>
</figure>

<style>
	.timeline svg {
		width: 100%;
		height: auto;
	}

	.lbl,
	.day {
		fill: var(--text-muted);
		font-family: var(--font-mono);
		font-size: 11px;
	}

	.track {
		fill: var(--surface-2);
	}

	.avail {
		fill: var(--success, #2f9e6b);
	}

	.price.base,
	.key.base {
		fill: var(--series-1);
		background: var(--series-1);
	}

	.price.window,
	.key.window {
		fill: var(--series-2);
		background: var(--series-2);
	}

	.price.override,
	.key.override {
		fill: var(--series-3);
		background: var(--series-3);
	}

	.price.none,
	.key.none {
		fill: var(--border-strong);
		background: var(--border-strong);
	}

	.tick {
		stroke: var(--border);
	}

	.key {
		display: inline-block;
		width: 0.75rem;
		height: 0.75rem;
		border-radius: 2px;
		vertical-align: middle;
	}
</style>
