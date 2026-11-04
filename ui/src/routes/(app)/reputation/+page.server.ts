import { read } from '#lib/server/read.ts';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, url }) => {
	const side = ['resource', 'provider', 'client'].includes(url.searchParams.get('side') ?? '')
		? url.searchParams.get('side')!
		: 'provider';
	const sort = url.searchParams.get('sort') === 'confidence' ? 'confidence' : 'score';
	const rows = await read(
		locals.api.GET('/api/v1/reputation', { params: { query: { side, sort, limit: 200 } } })
	);
	return { rows, side, sort };
};
