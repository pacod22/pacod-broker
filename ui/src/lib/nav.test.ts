import { describe, expect, it } from 'vitest';
import { isCurrent, navFor } from './nav';

describe('nav', () => {
	it('filters by permission', () => {
		const all = navFor(['broker.read', 'broker.manage', 'reputation.moderate']).flatMap(
			(g) => g.items
		);
		expect(all.some((i) => i.href === '/static')).toBe(true);
		const viewer = navFor(['broker.read']).flatMap((g) => g.items);
		expect(viewer.some((i) => i.href === '/static' || i.href === '/moderation')).toBe(false);
		expect(navFor([])).toEqual([]);
	});

	it('marks the current page', () => {
		expect(isCurrent('/', '/')).toBe(true);
		expect(isCurrent('/', '/catalogue')).toBe(false);
		expect(isCurrent('/catalogue', '/catalogue/res_1')).toBe(true);
	});
});
