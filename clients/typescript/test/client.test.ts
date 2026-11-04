/**
 * The TypeScript client against a real broker: the binary of this repository (BROKER_BIN, or
 * target/debug/pacod-broker after `cargo build -p pacod-broker`) on a free local port.
 */
import { spawn, type ChildProcess } from 'node:child_process';
import { existsSync, writeFileSync, mkdtempSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import {
	BrokerClient,
	BrokerError,
	Provider,
	SseParser,
	toEvent,
	type Announcement,
	type ProviderEvent,
	type WatchEvent
} from '../src/index.js';
import { main } from '../src/cli.js';

const BIN = process.env.BROKER_BIN ?? resolve(__dirname, '../../../target/debug/pacod-broker');
let proc: ChildProcess;
let url = '';

async function freePort(): Promise<number> {
	return new Promise((ok) => {
		const s = createServer().listen(0, '127.0.0.1', () => {
			const port = (s.address() as { port: number }).port;
			s.close(() => ok(port));
		});
	});
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function eventually<T>(
	what: string,
	check: () => Promise<T | false | undefined>
): Promise<T> {
	for (let i = 0; i < 160; i++) {
		const v = await check();
		if (v) return v;
		await sleep(50);
	}
	throw new Error(`${what} did not happen`);
}

const announcement = (name: string): Announcement =>
	({
		ttl_s: 3,
		provider: { name, summary: 'A provider in the TypeScript test' },
		resources: [
			{
				key: 'embed',
				kind: 'embedding',
				name: `${name}-embed`,
				summary: 'An embedding model for the TypeScript test',
				interfaces: [{ protocol: 'openai-chat', url: 'http://ts.test/v1' }],
				capabilities: { embeddings: true },
				pricing: {
					currency: 'EUR',
					periods: [{ valid_from: '2026-01-01T00:00:00Z', rates: { input: 0.02 } }]
				},
				access: { visibility: 'public' }
			}
		]
	}) as Announcement;

const hasBroker = existsSync(BIN);

describe.skipIf(!hasBroker)('against a broker', () => {
	beforeAll(async () => {
		const port = await freePort();
		url = `http://127.0.0.1:${port}`;
		const env: Record<string, string> = {
			PATH: process.env.PATH ?? '',
			BROKER_LISTEN: `127.0.0.1:${port}`,
			BROKER_METRICS_LISTEN: 'off',
			BROKER_REBUILD_MS: '0',
			BROKER_LEASE_TTL_MIN: '3',
			RUST_LOG: 'warn'
		};
		proc = spawn(BIN, [], { env, stdio: 'ignore' });
		await eventually('the broker starting', () => new BrokerClient({ url }).ready());
	});

	afterAll(() => {
		proc?.kill('SIGTERM');
	});

	it('provider and consumer', async () => {
		const consumer = new BrokerClient({ url, clientName: 'ts-consumer' });
		const watched: WatchEvent[] = [];
		const abort = new AbortController();
		const following = (async () => {
			try {
				for await (const e of consumer.watch({ signal: abort.signal })) watched.push(e);
			} catch {
				// Aborted.
			}
		})();

		const events: ProviderEvent[] = [];
		const provider = new Provider(new BrokerClient({ url }), announcement('ts-provider'), (e) =>
			events.push(e)
		);
		const first = await provider.start();
		const rid = first.resources.embed;
		provider.setInUse(4);

		const hits = await eventually('discovery', async () => {
			const r = await consumer.discover({
				kind: 'embedding',
				currency: 'EUR',
				capabilities: ['embeddings']
			});
			return r.hits.length ? r.hits : undefined;
		});
		expect(hits[0].id).toBe(rid);
		const ticket = hits[0].ticket!;
		expect((await consumer.resources({ q: 'embedding', kind: 'embedding' })).total).toBe(1);
		await eventually('live state', async () => (await consumer.resource(rid)).live.in_use === 4);
		expect((await consumer.provider(first.provider_id)).profile.name).toBe('ts-provider');

		expect((await consumer.reportUsage({ ticket, outcome: 'ok', ttft_ms: 80 })).accepted).toBe(1);
		expect((await consumer.rate(ticket, 5, 'good')).accepted).toBe(1);
		await expect(consumer.rate(ticket, 7)).rejects.toThrow(RangeError);
		const other = new BrokerClient({ url, clientName: 'someone-else' });
		await expect(other.rate(ticket, 1)).rejects.toMatchObject({ status: 422 });

		const asProvider = new BrokerClient({ url });
		expect((await asProvider.verifyTicket(ticket)).client).toBe('anon:ts-consumer');
		expect((await asProvider.reportClient(ticket, 'good_standing')).counted).toBe(true);
		expect((await consumer.me()).client).toBe('anon:ts-consumer');

		// The broker forgets the lease; the provider announces again by itself.
		await asProvider.deregister(first.lease_id, 'ts-provider');
		await eventually('re-announcement', async () => events.some((e) => e.kind === 'reannounced'));
		expect(provider.announced!.resources.embed).toBe(rid);

		// The price feed, and an update under the same lease.
		const feed = await consumer.priceFeed({ provider: 'ts-provider' });
		expect(typeof feed.models).toBe('object');
		const changed = announcement('ts-provider');
		changed.resources.push({ ...changed.resources[0], key: 'embed-b', name: 'embed-b' });
		const lease = provider.announced!.lease_id;
		const updated = await provider.update(changed);
		expect(updated.lease_id).toBe(lease);
		expect(updated.resources.embed).toBe(rid);
		expect(updated.resources['embed-b']).toBeDefined();

		await provider.stop();
		await eventually(
			'removal',
			async () => (await consumer.discover({ q: 'embedding', kind: 'embedding' })).total === 0
		);
		abort.abort();
		await following;
		const kinds = new Set(watched.map((e) => e.type));
		expect(kinds.has('hello') && kinds.has('upserted') && kinds.has('removed')).toBe(true);
	});

	it('problems come back typed', async () => {
		const c = new BrokerClient({ url });
		const e = await c.discover({ sort: 'reputation' }).catch((x) => x);
		expect(e).toBeInstanceOf(BrokerError);
		expect([e.status, e.kind]).toEqual([422, 'invalid']);
		const bad = announcement('ts-bad');
		bad.resources![0].kind = 'nonsense';
		const e2 = await c.announce(bad).catch((x) => x);
		expect(e2.errors.some((x: string) => x.includes('resources[0].kind'))).toBe(true);
		expect((await c.resource('res_nope').catch((x) => x)).notFound).toBe(true);
	});

	it('the command line', async () => {
		const dir = mkdtempSync(join(tmpdir(), 'broker-ts-'));
		const file = join(dir, 'a.yaml');
		writeFileSync(file, JSON.stringify(announcement('ts-cli')));
		const logs: string[] = [];
		const log = console.log;
		console.log = (...a: unknown[]) => logs.push(a.join(' '));
		try {
			expect(await main(['--url', url, 'announce', file, '--once'])).toBe(0);
			await eventually(
				'discovery',
				async () => (await new BrokerClient({ url }).discover({ q: 'ts-cli' })).total > 0
			);
			expect(await main(['--url', url, '--client', 'cli', 'discover', '-q', 'ts-cli'])).toBe(0);
			expect(await main(['--url', url, 'discover', '--sort', 'reputation'])).toBe(1);
		} finally {
			console.log = log;
		}
		expect(logs.join('\n')).toContain('ts-cli-embed');
	});
});

describe('watch parser', () => {
	it('splits events across chunks and ignores comments', () => {
		const p = new SseParser();
		const enc = new TextEncoder();
		expect(p.push(enc.encode('event: hello\nda'))).toEqual([]);
		const out = p.push(
			enc.encode(
				'ta: {"generation": 2}\n\n: keep-alive\n\nevent: generation\ndata: {"generation":3,"resources":1}\n\n'
			)
		);
		expect(out.map(([e, d]) => toEvent(e, d))).toEqual([
			{ type: 'hello', generation: 2 },
			{ type: 'generation', generation: 3, resources: 1 }
		]);
		expect(toEvent('future', '{}')).toBeNull();
	});
});
