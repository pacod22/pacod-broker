import { read } from '#lib/server/read.ts';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals }) => {
	const [providers, pending] = await Promise.all([
		read(locals.api.GET('/api/v1/providers')),
		read(locals.api.GET('/api/v1/pending'))
	]);
	return { providers, pending };
};
