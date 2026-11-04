/** A small server-sent-events reader for the watch stream. */

export type WatchEvent =
	| { type: 'hello'; generation: number }
	| { type: 'resync' }
	| { type: 'upserted'; resource_id: string; provider_id: string }
	| { type: 'removed'; resource_id: string; provider_id: string; reason: string }
	| { type: 'provider'; provider_id: string; what: string }
	| { type: 'generation'; generation: number; resources: number };

const KNOWN = new Set(['hello', 'resync', 'upserted', 'removed', 'provider', 'generation']);

export class SseParser {
	private buf = '';
	private event = '';
	private data: string[] = [];
	private readonly decoder = new TextDecoder();

	/** Feeds bytes; returns the (event, data) pairs they complete. Comments are skipped. */
	push(chunk: Uint8Array): [string, string][] {
		this.buf += this.decoder.decode(chunk, { stream: true });
		const out: [string, string][] = [];
		let at: number;
		while ((at = this.buf.indexOf('\n')) >= 0) {
			const line = this.buf.slice(0, at).replace(/\r$/, '');
			this.buf = this.buf.slice(at + 1);
			if (line === '') {
				if (this.event || this.data.length) out.push([this.event, this.data.join('\n')]);
				this.event = '';
				this.data = [];
			} else if (line.startsWith('event:')) {
				this.event = line.slice(6).trimStart();
			} else if (line.startsWith('data:')) {
				const v = line.slice(5);
				this.data.push(v.startsWith(' ') ? v.slice(1) : v);
			}
		}
		return out;
	}
}

/** The watch event, or `null` for kinds this client does not know. */
export function toEvent(event: string, data: string): WatchEvent | null {
	if (!KNOWN.has(event)) return null;
	let body: Record<string, unknown> = {};
	try {
		const parsed = data ? JSON.parse(data) : {};
		if (parsed && typeof parsed === 'object') body = parsed as Record<string, unknown>;
	} catch {
		// A malformed event carries only its type.
	}
	return { ...body, type: event } as WatchEvent;
}
