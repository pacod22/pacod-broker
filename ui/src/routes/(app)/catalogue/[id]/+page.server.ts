import { read } from '#lib/server/read.ts';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, params }) => {
	const path = { params: { path: { id: params.id } } };
	const [resource, timeline] = await Promise.all([
		read(locals.api.GET('/api/v1/resources/{id}', path)),
		read(locals.api.GET('/api/v1/resources/{id}/timeline', path))
	]);
	return { resource: resource as Record<string, any>, timeline };
};
