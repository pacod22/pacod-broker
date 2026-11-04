import { identity } from '#lib/server/identity.ts';
import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = async ({ locals }) => ({
	principal: locals.principal,
	apiProblem: locals.apiProblem,
	brokers: locals.brokers.map((b) => b.name),
	broker: locals.broker.name,
	identity: identity
		? { account: identity.account, serviceAccounts: identity.serviceAccounts }
		: null
});
