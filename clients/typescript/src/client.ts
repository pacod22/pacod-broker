/**
 * The client of pacod-broker's public API. Types come from docs/openapi.json, which the broker
 * generates from its own code (`npm run types`), so they follow the broker.
 */
import type { components } from './schema.js';
import { SseParser, toEvent, type WatchEvent } from './sse.js';

type S = components['schemas'];
export type Announcement = S['Announcement'];
export type Announced = S['Announced'];
export type Heartbeat = S['Heartbeat'];
export type Query = S['Query'];
export type DiscoverResponse = S['DiscoverResponse'];
export type Hit = S['HitView'];
export type ResourceView = S['ResourceView'];
export type ProviderView = S['ProviderView'];
export type PriceFeed = S['PriceFeed'];
export type UsageReport = S['UsageReport'];
export type Outcome = S['Outcome'];
export type Accepted = S['Accepted'];
export type ClientReportKind = S['ClientReportKind'];
export type Reputation = S['Reputation'];
export type ProblemBody = S['Problem'];
export type { WatchEvent };

export interface TicketInfo {
	client: string;
	resource_id: string;
	issued_at?: string | null;
	client_reputation?: Reputation | null;
}

export interface Me {
	client: string;
	as_client?: Reputation | null;
	provider_id: string;
	as_provider?: Reputation | null;
}

/** The broker refused, with its `application/problem+json`. */
export class BrokerError extends Error {
	readonly status: number;
	readonly problem: Partial<ProblemBody>;

	constructor(status: number, problem: Partial<ProblemBody>) {
		const errors = problem.errors ?? [];
		super(
			`${status} ${problem.detail ?? problem.title ?? 'error'}` +
				errors.map((e) => `\n  - ${e}`).join('')
		);
		this.name = 'BrokerError';
		this.status = status;
		this.problem = problem;
	}

	get kind(): string {
		return this.problem.type ?? '';
	}

	get errors(): string[] {
		return this.problem.errors ?? [];
	}

	get notFound(): boolean {
		return this.status === 404;
	}
}

/** The announcement waits for an admin's approval. */
export class PendingApproval extends BrokerError {
	constructor(problem: Partial<ProblemBody>) {
		super(202, problem);
		this.name = 'PendingApproval';
	}
}

export interface ClientOptions {
	/** The broker's public API; `BROKER_URL` or http://127.0.0.1:8060 by default. */
	url?: string;
	/** A pacod-id access token or service-account token (`pidt_…`); `BROKER_TOKEN`. */
	token?: string;
	/** Open mode: the name this client gives itself (`X-Broker-Client`); `BROKER_CLIENT`. */
	clientName?: string;
	timeoutMs?: number;
	fetch?: typeof fetch;
}

const env = (name: string): string | undefined =>
	typeof process !== 'undefined' ? process.env[name] || undefined : undefined;

const enc = encodeURIComponent;

function leasePath(leaseId: string, providerName?: string, suffix = ''): string {
	const p = `/v1/leases/${enc(leaseId)}${suffix}`;
	return providerName ? `${p}?provider=${enc(providerName)}` : p;
}

export class BrokerClient {
	readonly url: string;
	readonly token?: string;
	readonly clientName?: string;
	private readonly timeoutMs: number;
	private readonly fetchImpl: typeof fetch;

	constructor(options: ClientOptions = {}) {
		this.url = (options.url ?? env('BROKER_URL') ?? 'http://127.0.0.1:8060').replace(/\/+$/, '');
		this.token = options.token ?? env('BROKER_TOKEN');
		this.clientName = this.token ? undefined : (options.clientName ?? env('BROKER_CLIENT'));
		this.timeoutMs = options.timeoutMs ?? 30_000;
		this.fetchImpl = options.fetch ?? fetch;
	}

	/** Whether the broker knows lease owners by provider name (open mode, no token). */
	get isOpenModeClient(): boolean {
		return !this.token;
	}

	private headers(json: boolean): Record<string, string> {
		const h: Record<string, string> = { 'user-agent': 'pacod-broker-client-ts/0.1.0' };
		if (json) h['content-type'] = 'application/json';
		if (this.token) h.authorization = `Bearer ${this.token}`;
		else if (this.clientName) h['x-broker-client'] = this.clientName;
		return h;
	}

	private async call<T>(method: string, path: string, body?: unknown): Promise<T> {
		const res = await this.fetchImpl(`${this.url}${path}`, {
			method,
			headers: this.headers(body !== undefined),
			body: body === undefined ? undefined : JSON.stringify(body),
			signal: AbortSignal.timeout(this.timeoutMs)
		});
		const text = await res.text();
		const parsed = text ? safeJson(text) : undefined;
		if (res.status === 202) throw new PendingApproval((parsed ?? {}) as Partial<ProblemBody>);
		if (!res.ok) {
			const p =
				parsed && typeof parsed === 'object'
					? (parsed as Partial<ProblemBody>)
					: { detail: text.slice(0, 300) };
			throw new BrokerError(res.status, p);
		}
		return parsed as T;
	}

	// --- Consumers ---------------------------------------------------------------------------

	/** Runs a discovery query (docs/api.md §3.1). */
	discover(query: Partial<Query> = {}): Promise<DiscoverResponse> {
		return this.call('POST', '/v1/discover', query);
	}

	/** Every page of a query, following the cursor. */
	async discoverAll(query: Partial<Query> = {}, max = 1000): Promise<Hit[]> {
		const q = { ...query };
		const hits: Hit[] = [];
		for (;;) {
			const page = await this.discover(q);
			hits.push(...page.hits);
			if (!page.next_cursor || hits.length >= max) return hits.slice(0, max);
			q.cursor = page.next_cursor;
		}
	}

	/** `GET /v1/resources`: kind, q, tag, protocol, capability, region, currency, sort, … */
	resources(
		params: Record<string, string | number | boolean | undefined> = {}
	): Promise<DiscoverResponse> {
		const qs = new URLSearchParams(
			Object.entries(params)
				.filter(([, v]) => v !== undefined)
				.map(([k, v]) => [k, String(v)])
		);
		return this.call('GET', `/v1/resources?${qs}`);
	}

	resource(id: string): Promise<ResourceView> {
		return this.call('GET', `/v1/resources/${enc(id)}`);
	}

	provider(id: string): Promise<ProviderView> {
		return this.call('GET', `/v1/providers/${enc(id)}`);
	}

	/** Prices in force in pacod-gw's price feed format (`provider`, `kind`, `currency`, `key`). */
	priceFeed(params: { provider?: string; kind?: string; currency?: string; key?: 'name' | 'key' | 'id' } = {}): Promise<PriceFeed> {
		const qs = new URLSearchParams(Object.entries(params).filter(([, v]) => v !== undefined) as [string, string][]);
		return this.call('GET', `/v1/feeds/prices?${qs}`);
	}

	/**
	 * Changes as they happen. Iterate with `for await`; break, or abort `signal`, to stop.
	 * Watch again with the last generation seen to resume.
	 */
	async *watch(options: { after?: number; signal?: AbortSignal } = {}): AsyncGenerator<WatchEvent> {
		const qs = options.after !== undefined ? `?after=${options.after}` : '';
		const res = await this.fetchImpl(`${this.url}/v1/watch${qs}`, {
			headers: { ...this.headers(false), accept: 'text/event-stream' },
			signal: options.signal
		});
		if (!res.ok || !res.body) {
			const text = await res.text();
			throw new BrokerError(
				res.status,
				(safeJson(text) as Partial<ProblemBody>) ?? { detail: text }
			);
		}
		const parser = new SseParser();
		const reader = res.body.getReader();
		try {
			for (;;) {
				const { done, value } = await reader.read();
				if (done) return;
				for (const [event, data] of parser.push(value)) {
					const e = toEvent(event, data);
					if (e) yield e;
				}
			}
		} finally {
			reader.releaseLock();
			await res.body.cancel().catch(() => {});
		}
	}

	// --- Reports -----------------------------------------------------------------------------

	/** Reports how calls went, each with the ticket of the result called. */
	reportUsage(...reports: UsageReport[]): Promise<Accepted> {
		return this.call('POST', '/v1/reports/usage', { reports });
	}

	/** Rates a resource, 1 to 5, with the ticket of the result rated. */
	rate(ticket: string, rating: number, comment?: string): Promise<Accepted> {
		if (!Number.isInteger(rating) || rating < 1 || rating > 5)
			return Promise.reject(new RangeError('a rating is 1 to 5'));
		return this.call('POST', '/v1/reports/feedback', { ticket, rating, comment });
	}

	me(): Promise<Me> {
		return this.call('GET', '/v1/reputation/me');
	}

	// --- Providers ---------------------------------------------------------------------------

	/** Announces once; `Provider` keeps the lease alive. */
	announce(a: Announcement): Promise<Announced> {
		return this.call('PUT', '/v1/announce', a);
	}

	async heartbeat(
		leaseId: string,
		state: Partial<Heartbeat> = {},
		providerName?: string
	): Promise<void> {
		await this.call('POST', leasePath(leaseId, providerName, '/heartbeat'), state);
	}

	async deregister(leaseId: string, providerName?: string): Promise<void> {
		await this.call('DELETE', leasePath(leaseId, providerName));
	}

	verifyTicket(ticket: string): Promise<TicketInfo> {
		return this.call('POST', '/v1/tickets/verify', { ticket });
	}

	reportClient(
		ticket: string,
		kind: ClientReportKind,
		details?: string
	): Promise<{ id: string; counted: boolean }> {
		return this.call('POST', '/v1/reports/client', { ticket, kind, details });
	}

	schema(): Promise<Record<string, unknown>> {
		return this.call('GET', '/v1/schema');
	}

	async ready(): Promise<boolean> {
		try {
			return (await this.fetchImpl(`${this.url}/readyz`, { signal: AbortSignal.timeout(5000) })).ok;
		} catch {
			return false;
		}
	}
}

function safeJson(text: string): unknown {
	try {
		return JSON.parse(text);
	} catch {
		return undefined;
	}
}
