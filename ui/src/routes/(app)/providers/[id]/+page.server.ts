import { act, read } from '#lib/server/read.ts';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, params }) => {
	const provider = await read(
		locals.api.GET('/api/v1/providers/{id}', { params: { path: { id: params.id } } })
	);
	return { provider: provider as Record<string, any> };
};

const path = (id: string) => ({ params: { path: { id } } });

export const actions: Actions = {
	block: ({ locals, params }) =>
		act(
			locals.api.POST('/api/v1/providers/{id}/block', path(params.id)),
			'Blocked: its leases ended and its announcements are refused.'
		),
	unblock: ({ locals, params }) =>
		act(locals.api.POST('/api/v1/providers/{id}/unblock', path(params.id)), 'Unblocked.'),
	approve: ({ locals, params }) =>
		act(
			locals.api.POST('/api/v1/providers/{id}/approve', path(params.id)),
			'Approved: its next announcement is taken.'
		),
	verify: ({ locals, params }) =>
		act(
			locals.api.POST('/api/v1/providers/{id}/verify', {
				...path(params.id),
				body: { verified: true }
			}),
			'Verified.'
		),
	unverify: ({ locals, params }) =>
		act(
			locals.api.POST('/api/v1/providers/{id}/verify', {
				...path(params.id),
				body: { verified: false }
			}),
			'No longer verified.'
		),
	incident: async ({ locals, params, request }) => {
		const note = String((await request.formData()).get('note') ?? '').trim();
		return act(
			locals.api.POST('/api/v1/providers/{id}/incident', { ...path(params.id), body: { note } }),
			'Incident recorded: it costs the provider trust.'
		);
	},
	evict: async ({ locals, request }) => {
		const id = String((await request.formData()).get('lease') ?? '');
		return act(
			locals.api.DELETE('/api/v1/leases/{id}', { params: { path: { id } } }),
			'Lease ended.'
		);
	}
};
