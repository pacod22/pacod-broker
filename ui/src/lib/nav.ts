/** The sidebar: one list, filtered by what the signed-in person may do. */

export interface NavItem {
	href: string;
	label: string;
	/** The broker permission the page needs. */
	needs: string;
}

export const NAV: { label: string; items: NavItem[] }[] = [
	{
		label: 'Catalogue',
		items: [
			{ href: '/', label: 'Overview', needs: 'broker.read' },
			{ href: '/catalogue', label: 'Resources', needs: 'broker.read' },
			{ href: '/providers', label: 'Providers', needs: 'broker.read' }
		]
	},
	{
		label: 'Trust',
		items: [
			{ href: '/reputation', label: 'Reputation', needs: 'broker.read' },
			{ href: '/moderation', label: 'Moderation', needs: 'reputation.moderate' }
		]
	},
	{
		label: 'Operation',
		items: [
			{ href: '/static', label: 'Static entries', needs: 'broker.manage' },
			{ href: '/leases', label: 'Leases', needs: 'broker.read' },
			{ href: '/metrics', label: 'Metrics', needs: 'broker.read' },
			{ href: '/access', label: 'Access', needs: 'broker.read' },
			{ href: '/settings', label: 'Settings', needs: 'broker.read' }
		]
	}
];

export function navFor(permissions: string[]) {
	const has = (p: string) => permissions.includes('*') || permissions.includes(p);
	return NAV.map((g) => ({ ...g, items: g.items.filter((i) => has(i.needs)) })).filter(
		(g) => g.items.length > 0
	);
}

export function isCurrent(href: string, path: string): boolean {
	return href === '/' ? path === '/' : path === href || path.startsWith(href + '/');
}
