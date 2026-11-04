import { read } from '#lib/server/read.ts';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, url }) => {
	const p = url.searchParams;
	const query: Record<string, string> = {};
	for (const k of ['q', 'kind', 'provider', 'sort', 'cursor']) {
		const v = p.get(k);
		if (v) query[k] = v;
	}
	const result = await read(
		locals.api.GET('/api/v1/resources', { params: { query: { ...query, limit: 50 } } })
	);
	return { result, query };
};
