/**
 * Sign-in through pacod-id, with BROKER_UI_AUTH=oidc, as pacod-gw's console does it
 * (pacod-gw/ui/src/lib/server/identity.ts): the console is a confidential client of pacod-id
 * (a BFF). `@pacod/id-sveltekit` serves /auth/login, /auth/callback and /auth/logout and keeps
 * pacod-id's tokens sealed in one cookie; the session's access token is the bearer the broker's
 * admin API takes. With BROKER_UI_AUTH=open there is no sign-in.
 */
import {
	BROKER_CONSOLE_URL,
	BROKER_ID_INTERNAL_URL,
	BROKER_ID_ISSUER,
	BROKER_UI_AUTH,
	BROKER_UI_CLIENT_ID,
	BROKER_UI_CLIENT_SECRET,
	BROKER_UI_SESSION_SECRET
} from '$app/env/private';
import { createIdHandle, sealedCookie } from '@pacod/id-sveltekit';

const COOKIE = 'broker_session';

function required(name: string, value: string | undefined): string {
	if (!value)
		throw new Error(`${name} is required with BROKER_UI_AUTH=oidc (signing in through pacod-id)`);
	return value;
}

function start(issuer: string) {
	const origin = required('BROKER_CONSOLE_URL', BROKER_CONSOLE_URL);
	const sessionSecret = required('BROKER_UI_SESSION_SECRET', BROKER_UI_SESSION_SECRET);
	if (sessionSecret.length < 32)
		throw new Error('BROKER_UI_SESSION_SECRET must have at least 32 characters');
	const handle = createIdHandle({
		issuer,
		internalUrl: BROKER_ID_INTERNAL_URL ?? issuer,
		clientId: BROKER_UI_CLIENT_ID,
		clientSecret: required('BROKER_UI_CLIENT_SECRET', BROKER_UI_CLIENT_SECRET),
		redirectUri: `${origin}/auth/callback`,
		postLogoutRedirectUri: `${origin}/`,
		origin,
		store: sealedCookie({ name: COOKIE, secret: sessionSecret }),
		secret: sessionSecret
	});
	return {
		issuer,
		handle,
		account: `${issuer}/account`,
		serviceAccounts: `${issuer}/admin/service-accounts`
	};
}

/** Signing in through pacod-id, or `null` in open mode. */
export const identity =
	BROKER_UI_AUTH === 'oidc' ? start(required('BROKER_ID_ISSUER', BROKER_ID_ISSUER)) : null;
