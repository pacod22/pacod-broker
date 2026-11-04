/**
 * The brokers the console docks to (BROKER_CONSOLE_BROKERS). The console holds no state of its
 * own: it can attach to any running broker's admin API, and a broker never needs the console.
 * With several listed, the top bar offers a switcher; the choice is kept in a cookie.
 */

export interface Broker {
	name: string;
	/** The admin API's origin, without a path. */
	origin: string;
}

export const BROKER_COOKIE = 'broker_choice';

/** Reads `name=origin,name=origin`. Throws with a message naming what is wrong. */
export function parseBrokers(text: string): Broker[] {
	const out: Broker[] = [];
	for (const part of text
		.split(',')
		.map((p) => p.trim())
		.filter(Boolean)) {
		const at = part.indexOf('=');
		if (at < 1) throw new Error(`BROKER_CONSOLE_BROKERS: "${part}" is not name=origin`);
		const name = part.slice(0, at).trim();
		const origin = part
			.slice(at + 1)
			.trim()
			.replace(/\/+$/, '');
		if (!/^[a-z0-9][a-z0-9._-]*$/.test(name))
			throw new Error(`BROKER_CONSOLE_BROKERS: "${name}" is not a name (a-z, 0-9, . _ -)`);
		let url: URL;
		try {
			url = new URL(origin);
		} catch {
			throw new Error(`BROKER_CONSOLE_BROKERS: "${origin}" is not a URL`);
		}
		if (url.protocol !== 'http:' && url.protocol !== 'https:')
			throw new Error(`BROKER_CONSOLE_BROKERS: "${origin}" must start with http:// or https://`);
		if (out.some((b) => b.name === name))
			throw new Error(`BROKER_CONSOLE_BROKERS: "${name}" is listed twice`);
		out.push({ name, origin });
	}
	if (out.length === 0) throw new Error('BROKER_CONSOLE_BROKERS lists no broker');
	return out;
}

/** The broker a cookie names, or the first one. */
export function chosen(brokers: Broker[], cookie: string | undefined): Broker {
	return brokers.find((b) => b.name === cookie) ?? brokers[0];
}
