import { redirect } from '@sveltejs/kit';
import { sequence, type Handle } from '@sveltejs/kit/hooks';
import { BROKER_CONSOLE_BROKERS, BROKER_UI_ADMIN_TOKEN } from '$app/env/private';
import { BROKER_COOKIE, chosen, parseBrokers } from '#lib/brokers.ts';
import { allowFormTarget } from '#lib/csp.ts';
import { createApi, problemText } from '#lib/server/api.ts';
import { identity } from '#lib/server/identity.ts';

const brokers = parseBrokers(BROKER_CONSOLE_BROKERS);
const issuerOrigin = identity ? new URL(identity.issuer).origin : null;

const app: Handle = async ({ event, resolve }) => {
	const { locals, cookies, url } = event;
	locals.brokers = brokers;
	locals.broker = chosen(brokers, cookies.get(BROKER_COOKIE));
	locals.token = identity ? (locals.session?.accessToken ?? null) : (BROKER_UI_ADMIN_TOKEN ?? null);
	locals.api = createApi(locals.broker.origin, locals.token);
	locals.principal = null;
	locals.apiProblem = null;

	const signedIn = !identity || locals.token;
	if (signedIn && !url.pathname.startsWith('/auth/')) {
		// Who is calling, asked once per request.
		const { data, error, response } = await locals.api.GET('/api/v1/session');
		if (response.ok && data) {
			locals.principal = data as App.Locals['principal'];
		} else if (response.status === 401 && identity) {
			redirect(303, `/auth/login?next=${encodeURIComponent(url.pathname + url.search)}`);
		} else {
			locals.apiProblem = `${locals.broker.name} (${locals.broker.origin}): ${problemText(error, response.status)}`;
		}
	} else if (identity && !url.pathname.startsWith('/auth/')) {
		redirect(303, `/auth/login?next=${encodeURIComponent(url.pathname + url.search)}`);
	}

	const response = await resolve(event);
	if (issuerOrigin) {
		const policy = allowFormTarget(response.headers.get('content-security-policy'), issuerOrigin);
		if (policy) response.headers.set('content-security-policy', policy);
	}
	if (!response.headers.has('cache-control')) response.headers.set('cache-control', 'no-store');
	response.headers.set('x-content-type-options', 'nosniff');
	response.headers.set('x-frame-options', 'DENY');
	response.headers.set('referrer-policy', 'same-origin');
	return response;
};

/** With pacod-id, its sign-in routes and the session come first. */
export const handle: Handle = identity ? sequence(identity.handle as unknown as Handle, app) : app;
