/**
 * The TypeScript client of pacod-broker.
 *
 * ```ts
 * import { BrokerClient, Provider } from '@pacod/broker-client';
 *
 * const broker = new BrokerClient({ url: 'http://127.0.0.1:8060', clientName: 'my-agent' });
 * const found = await broker.discover({ kind: 'model', capabilities: ['tools'], currency: 'EUR', sort: 'price:input' });
 * const hit = found.hits[0];
 * await broker.reportUsage({ ticket: hit.ticket!, outcome: 'ok', ttft_ms: 310 });
 * await broker.rate(hit.ticket!, 5);
 *
 * const provider = new Provider(new BrokerClient(), announcement);
 * await provider.start();
 * provider.setInUse(3);
 * // … on shutdown:
 * await provider.stop();
 * ```
 */
export * from './client.js';
export { Provider, type ProviderEvent } from './provider.js';
export { SseParser, toEvent, type WatchEvent } from './sse.js';
export type { components } from './schema.js';
