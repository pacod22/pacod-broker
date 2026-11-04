import { identity } from '#lib/server/identity.ts';
import { manifestSource } from '#lib/authz.generated.ts';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async () => ({
	roles: Object.entries(manifestSource.roles ?? {}).map(([name, r]) => ({
		name,
		at: r.at,
		permissions: r.permissions ?? [],
		implicit: Boolean(r.implicit)
	})),
	identity: identity ? { serviceAccounts: identity.serviceAccounts, issuer: identity.issuer } : null
});
