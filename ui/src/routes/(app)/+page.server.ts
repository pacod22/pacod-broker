import { read } from '#lib/server/read.ts';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals }) => {
	if (locals.apiProblem) return { status: null, pending: [] };
	const [status, pending] = await Promise.all([
		read(locals.api.GET('/api/v1/status')),
		read(locals.api.GET('/api/v1/pending'))
	]);
	return { status, pending };
};
