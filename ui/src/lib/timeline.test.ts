import { describe, expect, it } from 'vitest';
import { bars, dayTicks, priceSpans } from './timeline';

describe('timeline', () => {
	const from = '2026-11-02T00:00:00Z';
	const to = '2026-11-04T00:00:00Z';

	it('places spans and clips them to the band', () => {
		const b = bars(
			[
				{ from: '2026-11-01T12:00:00Z', to: '2026-11-02T12:00:00Z' },
				{ from: '2026-11-03T00:00:00Z', to: '2026-11-05T00:00:00Z' }
			],
			from,
			to,
			200
		);
		expect(b).toEqual([
			{ x: 0, width: 50, label: '' },
			{ x: 100, width: 100, label: '' }
		]);
	});

	it('ticks every midnight', () => {
		expect(dayTicks('2026-11-02T06:00:00Z', to, 100).map((t) => t.label)).toEqual(['11-03']);
	});

	it('turns changes into spans', () => {
		expect(
			priceSpans(
				[
					{ at: from, source: 'base' },
					{ at: '2026-11-02T22:00:00Z', source: 'window:night' }
				],
				to
			)
		).toEqual([
			{ from, to: '2026-11-02T22:00:00Z', label: 'base' },
			{ from: '2026-11-02T22:00:00Z', to, label: 'window:night' }
		]);
	});
});
