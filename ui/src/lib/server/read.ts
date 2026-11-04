/** Reads an answer of the broker, or ends the load with its problem. */
import { error, fail } from '@sveltejs/kit';
import { problemText } from './api.ts';

export async function read<T>(
	call: Promise<{ data?: T; error?: unknown; response: Response }>
): Promise<T> {
	const { data, error: e, response } = await call;
	if (!response.ok || data === undefined)
		error(response.status === 401 ? 401 : response.status || 502, problemText(e, response.status));
	return data;
}

/** Runs an action; a refusal becomes a form message rather than an error page. */
export async function act(call: Promise<{ error?: unknown; response: Response }>, done: string) {
	const { error: e, response } = await call;
	if (!response.ok) return fail(response.status, { message: problemText(e, response.status) });
	return { done };
}
