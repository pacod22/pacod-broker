import { redirect } from '@sveltejs/kit';
import { BROKER_COOKIE } from '#lib/brokers.ts';
import type { RequestHandler } from './$types';

/** Docks the console to another of the listed brokers. */
export const POST: RequestHandler = async ({ request, cookies, locals, url }) => {
	const form = await request.formData();
	const name = String(form.get('broker') ?? '');
	if (locals.brokers.some((b) => b.name === name)) {
		cookies.set(BROKER_COOKIE, name, {
			path: '/',
			httpOnly: true,
			sameSite: 'lax',
			secure: url.protocol === 'https:',
			maxAge: 60 * 60 * 24 * 365
		});
	}
	redirect(303, '/');
};
