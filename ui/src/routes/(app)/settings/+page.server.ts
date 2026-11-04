import { read } from '#lib/server/read.ts';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals }) => ({
	config: (await read(locals.api.GET('/api/v1/config'))) as Record<string, any>,
	brokers: locals.brokers
});
