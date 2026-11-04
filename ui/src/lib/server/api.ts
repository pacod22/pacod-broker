/**
 * The typed client of a broker's admin API, generated from docs/openapi.json (`npm run
 * api:types`). It exists on the server only: one per request, with that request's bearer.
 */
import createClient, { type Client } from 'openapi-fetch';
import type { paths } from '#lib/api/schema.d.ts';

export type ApiClient = Client<paths>;

const TIMEOUT_MS = 15_000;

/** A `fetch` that answers instead of throwing when the broker cannot be reached. */
async function fetchOrExplain(request: Request): Promise<Response> {
	try {
		return await fetch(request, {
			signal: AbortSignal.any([request.signal, AbortSignal.timeout(TIMEOUT_MS)])
		});
	} catch {
		return Response.json(
			{
				type: 'unreachable',
				title: 'Bad Gateway',
				status: 502,
				detail: 'the broker did not answer'
			},
			{ status: 502 }
		);
	}
}

export function createApi(origin: string, token: string | null): ApiClient {
	return createClient<paths>({
		baseUrl: origin,
		headers: token ? { authorization: `Bearer ${token}` } : {},
		fetch: fetchOrExplain
	});
}

export interface Problem {
	type?: string;
	title?: string;
	status?: number;
	detail?: string;
	errors?: string[];
}

/** The problem an answer carries, as one sentence. */
export function problemText(error: unknown, status: number): string {
	const p = (error ?? {}) as Problem;
	const parts = [p.detail ?? p.title ?? `the broker answered ${status}`];
	if (p.errors?.length) parts.push(p.errors.join('; '));
	return parts.join(': ');
}
