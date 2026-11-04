import { fail } from '@sveltejs/kit';
import { act, read } from '#lib/server/read.ts';
import { problemText } from '#lib/server/api.ts';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals }) => ({
	entries: await read(locals.api.GET('/api/v1/static'))
});

export const actions: Actions = {
	put: async ({ locals, request }) => {
		const text = String((await request.formData()).get('json') ?? '');
		let body: unknown;
		try {
			body = JSON.parse(text);
		} catch (e) {
			return fail(400, { message: `Not JSON: ${(e as Error).message}`, json: text });
		}
		const { error, response } = await locals.api.PUT('/api/v1/static', { body: body as never });
		if (!response.ok)
			return fail(response.status, { message: problemText(error, response.status), json: text });
		return { done: 'Saved: the entries are in the catalogue and kept in the state directory.' };
	},
	remove: async ({ locals, request }) => {
		const id = String((await request.formData()).get('provider_id') ?? '');
		return act(
			locals.api.DELETE('/api/v1/static/{provider_id}', { params: { path: { provider_id: id } } }),
			'Removed.'
		);
	}
};
