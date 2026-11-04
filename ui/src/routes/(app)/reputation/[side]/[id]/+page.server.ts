import { act, read } from '#lib/server/read.ts';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, params }) => {
	const one = await read(
		locals.api.GET('/api/v1/reputation/{side}/{id}', {
			params: { path: { side: params.side, id: params.id } }
		})
	);
	return { one: one as Record<string, any>, side: params.side, id: params.id };
};

const action = (name: string, done: string) =>
	(({ locals, params }) =>
		act(
			locals.api.POST('/api/v1/reputation/{side}/{id}/{action}', {
				params: { path: { side: params.side, id: params.id, action: name } }
			}),
			done
		)) as Actions[string];

export const actions: Actions = {
	freeze: action('freeze', 'Frozen: the score stays as it is until unfrozen.'),
	unfreeze: action('unfreeze', 'Unfrozen.'),
	reset: action('reset', 'Reset: the evidence is gone and the score is back at the prior.')
};
