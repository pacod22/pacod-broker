import { defineEnvVars } from '@sveltejs/kit/env';

/** An optional http(s) URL setting, without a trailing slash. */
function optionalUrl(name: string, example: string) {
	return (value: string | undefined) => {
		const text = (value ?? '').trim();
		if (text === '') return undefined;
		let url: URL;
		try {
			url = new URL(text);
		} catch {
			throw new Error(`${name} must be a URL such as ${example}, got "${text}"`);
		}
		if (url.protocol !== 'http:' && url.protocol !== 'https:') {
			throw new Error(`${name} must start with http:// or https://`);
		}
		return text.replace(/\/+$/, '');
	};
}

/** An optional secret: trimmed, empty when unset. */
const secret = (value: string | undefined) => (value ?? '').trim() || undefined;

export const variables = defineEnvVars({
	BROKER_CONSOLE_BROKERS: {
		description:
			'The brokers the console docks to: name=admin-origin pairs separated by commas, such as default=http://pacod-broker-admin:8061,eu=http://broker-eu-admin:8061. The first is chosen by default. Server-only: the browser never talks to a broker.',
		schema: (value) => (value ?? '').trim() || 'default=http://127.0.0.1:18061'
	},
	BROKER_UI_ADMIN_TOKEN: {
		description:
			"With BROKER_UI_AUTH=open: the brokers' BROKER_ADMIN_TOKEN, sent as the bearer token. Server-only.",
		schema: secret
	},
	BROKER_UI_AUTH: {
		description:
			'Who signs people in: open (no sign-in; for a broker in open mode on a closed network) or oidc (pacod-id). Server-only.',
		schema: (value) => {
			const mode = (value ?? '').trim() || 'open';
			if (mode !== 'open' && mode !== 'oidc')
				throw new Error(`BROKER_UI_AUTH must be open or oidc, got "${mode}"`);
			return mode;
		}
	},
	BROKER_ID_ISSUER: {
		description: "pacod-id's public issuer, such as https://id.example.com. Required with oidc.",
		schema: optionalUrl('BROKER_ID_ISSUER', 'https://id.example.com')
	},
	BROKER_ID_INTERNAL_URL: {
		description:
			"pacod-id's internal listener as the console's server reaches it, such as http://pacod-id-internal:8071. Default: the issuer.",
		schema: optionalUrl('BROKER_ID_INTERNAL_URL', 'http://pacod-id-internal:8071')
	},
	BROKER_UI_CLIENT_ID: {
		description: "The console's client at pacod-id. Default pacod-broker-ui.",
		schema: (value) => (value ?? '').trim() || 'pacod-broker-ui'
	},
	BROKER_UI_CLIENT_SECRET: {
		description: "The console's client secret at pacod-id.",
		schema: secret
	},
	BROKER_UI_SESSION_SECRET: {
		description:
			"At least 32 characters: seals the session cookie that holds pacod-id's tokens. Changing it signs everyone out.",
		schema: secret
	},
	BROKER_CONSOLE_URL: {
		description:
			"The console's address as people reach it, such as https://broker.example.com: where pacod-id sends them back.",
		schema: optionalUrl('BROKER_CONSOLE_URL', 'https://broker.example.com')
	},
	PROMETHEUS_URL: {
		description:
			'Origin of the Prometheus server that scrapes the brokers (job pacod-broker). Optional: without it the Metrics page says so. Server-only.',
		schema: optionalUrl('PROMETHEUS_URL', 'http://pacod-prometheus:9090')
	},
	GRAFANA_URL: {
		description:
			'Address of Grafana as the browser reaches it. Optional: linked from the Metrics page.',
		schema: optionalUrl('GRAFANA_URL', 'https://grafana.example.com')
	}
});
