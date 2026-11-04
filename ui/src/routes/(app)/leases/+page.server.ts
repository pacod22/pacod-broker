import { act, read } from '#lib/server/read.ts';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals }) => ({
	leases: await read(locals.api.GET('/api/v1/leases'))
});

export const actions: Actions = {
	evict: async ({ locals, request }) => {
		const id = String((await request.formData()).get('lease') ?? '');
		return act(
			locals.api.DELETE('/api/v1/leases/{id}', { params: { path: { id } } }),
			'Lease ended.'
		);
	}
};
