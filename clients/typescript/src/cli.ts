#!/usr/bin/env node
/**
 * pacod-broker: the broker from the command line (the same commands as broker-cli).
 *
 *   pacod-broker discover --kind model --capability tools --currency EUR --sort price:input
 *   pacod-broker announce provider.yaml        # keeps the lease alive until Ctrl-C
 *   pacod-broker report-usage --ticket tkt_… --outcome ok --ttft-ms 310
 *   pacod-broker rate --ticket tkt_… --rating 5
 *   pacod-broker watch
 *
 * The broker comes from --url or BROKER_URL; a pacod-id token from --token or BROKER_TOKEN; in
 * open mode a client name from --client or BROKER_CLIENT.
 */
import { readFileSync } from 'node:fs';
import { parseArgs } from 'node:util';
import { parse as parseYaml } from 'yaml';
import {
	BrokerClient,
	BrokerError,
	type Announcement,
	type DiscoverResponse,
	type Hit,
	type Query
} from './client.js';
import { Provider } from './provider.js';

const USAGE = `pacod-broker <command> [options]

Commands:
  discover        find resources (--kind --q --tag --capability --protocol --region --currency
                  --max-price rate=v --at --during FROM..TO --gpu --software name[@req]
                  --min-reputation --sort --limit --query file.json)
  resource ID     one resource
  provider ID     one provider
  announce FILE   announce from YAML or JSON and keep the lease alive (--ttl, --once)
  report-usage    --ticket --outcome [--ttft-ms --ms-per-token --price-charged --rate]
  rate            --ticket --rating 1-5 [--comment]
  report-client   --ticket --kind [--details]   (providers)
  verify-ticket TICKET                          (providers)
  watch           [--after N]
  me | schema

Global: --url (BROKER_URL) --token (BROKER_TOKEN) --client (BROKER_CLIENT) --json`;

const out = (v: unknown) => console.log(JSON.stringify(v, null, 2));
const cut = (s: string, n: number) => (s.length <= n ? s : s.slice(0, n - 1) + '…');

function price(h: Hit): string {
	if (h.price) {
		if (h.price.free) return 'free';
		const first = Object.entries(h.price.rates ?? {}).find(([, v]) => v != null);
		if (first) return `${first[0]} ${first[1]} ${h.price.currency}`;
	}
	const s = h.price_summary;
	const r = s && Object.entries(s.rates)[0];
	return r ? `${r[0]} avg ${r[1].avg.toFixed(4)} ${s!.currency}` : '—';
}

function table(r: DiscoverResponse): void {
	console.log(`${r.total} found (generation ${r.generation})`);
	const row = (...c: string[]) =>
		console.log(
			`${c[0].padEnd(34)} ${c[1].padEnd(12)} ${c[2].padEnd(14)} ${c[3].padEnd(26)} ${c[4].padEnd(11)} ${c[5].padStart(6)}  ${c[6]}`
		);
	row('RESOURCE', 'KIND', 'PROVIDER', 'PRICE', 'STATUS', 'REP', 'ID');
	for (const h of r.hits) {
		row(
			cut(h.resource.name, 34),
			cut(h.resource.kind, 12),
			cut(h.provider.name, 14),
			cut(price(h), 26),
			h.available ? h.live.status : 'unavailable',
			h.reputation ? `${Math.round(h.reputation.score * 100)}%` : 'new',
			h.id
		);
	}
	if (r.next_cursor) console.log(`more: --query with "cursor": "${r.next_cursor}"`);
}

export async function main(argv: string[]): Promise<number> {
	const { values: o, positionals } = parseArgs({
		args: argv,
		allowPositionals: true,
		options: {
			url: { type: 'string' },
			token: { type: 'string' },
			client: { type: 'string' },
			json: { type: 'boolean' },
			help: { type: 'boolean', short: 'h' },
			query: { type: 'string' },
			kind: { type: 'string' },
			q: { type: 'string', short: 'q' },
			tag: { type: 'string', multiple: true },
			capability: { type: 'string', multiple: true },
			protocol: { type: 'string', multiple: true },
			region: { type: 'string', multiple: true },
			software: { type: 'string', multiple: true },
			'max-price': { type: 'string', multiple: true },
			currency: { type: 'string' },
			at: { type: 'string' },
			during: { type: 'string' },
			gpu: { type: 'string' },
			'min-reputation': { type: 'string' },
			sort: { type: 'string' },
			limit: { type: 'string' },
			ttl: { type: 'string' },
			once: { type: 'boolean' },
			ticket: { type: 'string' },
			outcome: { type: 'string' },
			'ttft-ms': { type: 'string' },
			'ms-per-token': { type: 'string' },
			'price-charged': { type: 'string' },
			rate: { type: 'string' },
			rating: { type: 'string' },
			comment: { type: 'string' },
			details: { type: 'string' },
			after: { type: 'string' }
		}
	});
	const [command, arg] = positionals;
	if (o.help || !command) {
		console.log(USAGE);
		return command ? 0 : 2;
	}
	const broker = new BrokerClient({ url: o.url, token: o.token, clientName: o.client });
	const num = (v?: string) => (v === undefined ? undefined : Number(v));
	const need = (name: string, v?: string): string => {
		if (!v) throw new Error(`--${name} is required`);
		return v;
	};
	try {
		switch (command) {
			case 'discover': {
				const q: Partial<Query> = o.query ? JSON.parse(readFileSync(o.query, 'utf8')) : {};
				if (o.kind) q.kind = o.kind;
				if (o.q) q.q = o.q;
				if (o.currency) q.currency = o.currency;
				if (o.at) q.at = o.at;
				if (o.sort) q.sort = o.sort;
				if (o.limit) q.limit = Number(o.limit);
				if (o['min-reputation']) q.min_reputation = Number(o['min-reputation']);
				if (o.tag) q.tags = [...(q.tags ?? []), ...o.tag];
				if (o.capability) q.capabilities = [...(q.capabilities ?? []), ...o.capability];
				if (o.protocol) q.protocols = [...(q.protocols ?? []), ...o.protocol];
				if (o.region) q.regions = [...(q.regions ?? []), ...o.region];
				for (const m of o['max-price'] ?? []) {
					const [rate, v] = m.split('=');
					q.max_price = { ...(q.max_price ?? {}), [rate]: Number(v) };
				}
				if (o.during) {
					const [from, to] = o.during.split('..');
					q.during = { from, to };
				}
				if (o.gpu) q.hardware = { ...(q.hardware ?? {}), gpu_model: o.gpu };
				for (const s of o.software ?? []) {
					const [name, version] = s.split('@');
					q.software = [...(q.software ?? []), version ? { name, version } : { name }];
				}
				const r = await broker.discover(q);
				if (o.json) out(r);
				else table(r);
				break;
			}
			case 'resource':
				out(await broker.resource(need('ID', arg)));
				break;
			case 'provider':
				out(await broker.provider(need('ID', arg)));
				break;
			case 'announce': {
				const file = need('FILE', arg);
				const a = parseYaml(readFileSync(file, 'utf8')) as Announcement;
				if (o.ttl) a.ttl_s = Number(o.ttl);
				if (o.once) {
					out(await broker.announce(a));
					break;
				}
				const p = new Provider(broker, a, (e) => {
					if (e.kind === 'announced') {
						console.error(
							`announced ${Object.keys(e.announced.resources).length} resources under ${e.announced.lease_id}; heartbeat every ${e.announced.heartbeat_every_s} s (Ctrl-C ends the lease)`
						);
						for (const w of e.announced.warnings ?? []) console.error(`warning: ${w}`);
					} else if (e.kind === 'reannounced')
						console.error(
							`the broker had forgotten the lease; announced again under ${e.announced.lease_id}`
						);
					else if (e.kind === 'retrying') console.error(`retrying: ${e.reason}`);
					else console.error("waiting for an admin's approval");
				});
				await p.start();
				// The heartbeat timer does not keep Node alive; this does, until a signal.
				const keepAlive = setInterval(() => {}, 1 << 30);
				await new Promise<void>((resolve) => {
					process.once('SIGINT', resolve);
					process.once('SIGTERM', resolve);
				});
				clearInterval(keepAlive);
				await p.stop();
				console.error('lease ended');
				break;
			}
			case 'report-usage':
				out(
					await broker.reportUsage({
						ticket: need('ticket', o.ticket),
						outcome: need('outcome', o.outcome) as never,
						ttft_ms: num(o['ttft-ms']),
						ms_per_token: num(o['ms-per-token']),
						price_charged: num(o['price-charged']),
						rate: o.rate
					})
				);
				break;
			case 'rate':
				out(
					await broker.rate(need('ticket', o.ticket), Number(need('rating', o.rating)), o.comment)
				);
				break;
			case 'report-client':
				out(
					await broker.reportClient(
						need('ticket', o.ticket),
						need('kind', o.kind as string) as never,
						o.details
					)
				);
				break;
			case 'verify-ticket':
				out(await broker.verifyTicket(need('TICKET', arg)));
				break;
			case 'watch':
				for await (const e of broker.watch({ after: num(o.after) })) console.log(JSON.stringify(e));
				break;
			case 'me':
				out(await broker.me());
				break;
			case 'schema':
				out(await broker.schema());
				break;
			default:
				console.error(`unknown command ${command}\n\n${USAGE}`);
				return 2;
		}
	} catch (e) {
		console.error(e instanceof BrokerError ? `the broker refused: ${e.message}` : String(e));
		return 1;
	}
	return 0;
}

// Run when executed, not when imported (the tests import main).
if (import.meta.url === `file://${process.argv[1]}` || process.argv[1]?.endsWith('/pacod-broker')) {
	main(process.argv.slice(2)).then((code) => process.exit(code));
}
