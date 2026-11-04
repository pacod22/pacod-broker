/**
 * A provider that keeps itself announced.
 *
 * `start()` announces, then heartbeats every `heartbeat_every_s` the broker asked for, with the
 * live state set through `setStatus`, `setInUse`, `setTtftMs` and `setResource`. When the broker
 * no longer knows the lease (it restarted, or the lease ran out while the network was down), the
 * provider announces again by itself; when the broker cannot be reached, it tries again with
 * back-off. `stop()` says goodbye, which counts in the provider's favour in reputation.
 */
import {
	BrokerError,
	PendingApproval,
	type Announced,
	type Announcement,
	type BrokerClient,
	type Heartbeat
} from './client.js';

type LiveStatus = NonNullable<Heartbeat['status']>;

export type ProviderEvent =
	| { kind: 'announced'; announced: Announced }
	| { kind: 'reannounced'; announced: Announced }
	| { kind: 'retrying'; reason: string }
	| { kind: 'pending' };

interface One {
	status?: LiveStatus;
	in_use?: number;
	ttft_ms?: number;
}

export class Provider {
	announced: Announced | null = null;
	private all: One = {};
	private perKey: Record<string, One> = {};
	private timer: ReturnType<typeof setTimeout> | null = null;
	private stopped = false;
	private failures = 0;
	private readonly owner?: string;

	constructor(
		readonly client: BrokerClient,
		readonly announcement: Announcement,
		readonly onEvent: (e: ProviderEvent) => void = () => {}
	) {
		// Open-mode brokers know the lease's owner by the provider's name.
		this.owner = client.isOpenModeClient ? announcement.provider.name : undefined;
	}

	setStatus(status: LiveStatus): void {
		this.all.status = status;
	}

	setInUse(n: number): void {
		this.all.in_use = n;
	}

	setTtftMs(ms: number): void {
		this.all.ttft_ms = ms;
	}

	/** The live state of one resource, by its key, over the provider-wide one. */
	setResource(key: string, state: One): void {
		this.perKey[key] = { ...this.perKey[key], ...state };
	}

	private heartbeatBody(): Partial<Heartbeat> {
		const body: Partial<Heartbeat> = { ...this.all };
		if (Object.keys(this.perKey).length) body.resources = structuredClone(this.perKey);
		return body;
	}

	async start(): Promise<Announced> {
		this.stopped = false;
		this.announced = await this.client.announce(this.announcement);
		this.onEvent({ kind: 'announced', announced: this.announced });
		this.schedule();
		return this.announced;
	}

	private schedule(): void {
		if (this.stopped || !this.announced) return;
		const every = Math.max(1, this.announced.heartbeat_every_s);
		// After a failure, try again sooner, but never more often than every second.
		const wait = this.failures === 0 ? every : Math.min(2 ** Math.min(this.failures, 5), every);
		this.timer = setTimeout(() => void this.beat(), wait * 1000);
		// A provider's heartbeat alone never keeps Node running.
		this.timer.unref?.();
	}

	private async beat(): Promise<void> {
		if (this.stopped || !this.announced) return;
		try {
			await this.client.heartbeat(this.announced.lease_id, this.heartbeatBody(), this.owner);
			this.failures = 0;
		} catch (e) {
			if (e instanceof BrokerError && e.notFound) {
				try {
					this.announced = await this.client.announce(this.announcement);
					this.failures = 0;
					this.onEvent({ kind: 'reannounced', announced: this.announced });
				} catch (e2) {
					if (e2 instanceof PendingApproval) {
						this.failures = 5;
						this.onEvent({ kind: 'pending' });
					} else {
						this.failures += 1;
						this.onEvent({ kind: 'retrying', reason: String(e2) });
					}
				}
			} else {
				this.failures += 1;
				this.onEvent({ kind: 'retrying', reason: String(e) });
			}
		}
		this.schedule();
	}

	/** Stops the heartbeats and ends the lease cleanly. */
	async stop(): Promise<void> {
		this.stopped = true;
		if (this.timer) clearTimeout(this.timer);
		if (!this.announced) return;
		try {
			await this.client.deregister(this.announced.lease_id, this.owner);
		} catch (e) {
			if (!(e instanceof BrokerError && e.notFound)) throw e;
		}
	}
}
