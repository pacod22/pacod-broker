import { act, read } from '#lib/server/read.ts';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals }) => {
	const reports = (await read(locals.api.GET('/api/v1/reports'))) as {
		client_reports: { id: string; client: string; kind: string; at: string }[];
		notes: {
			at: string;
			kind: string;
			subject: string;
			author: string;
			text: string;
			report_id?: string;
		}[];
	};
	return { reports };
};

export const actions: Actions = {
	reject: async ({ locals, request }) => {
		const id = String((await request.formData()).get('id') ?? '');
		return act(
			locals.api.POST('/api/v1/reports/{id}/reject', { params: { path: { id } } }),
			'The report was taken back: it no longer counts against the client.'
		);
	}
};
