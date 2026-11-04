/** Instant queries against Prometheus for the Metrics page (job `pacod-broker`). */
import { PROMETHEUS_URL } from '$app/env/private';

export const configured = Boolean(PROMETHEUS_URL);

export async function instant(
	query: string
): Promise<{ labels: Record<string, string>; value: number }[] | null> {
	if (!PROMETHEUS_URL) return null;
	try {
		const r = await fetch(`${PROMETHEUS_URL}/api/v1/query?query=${encodeURIComponent(query)}`, {
			signal: AbortSignal.timeout(5000)
		});
		if (!r.ok) return null;
		const body = (await r.json()) as {
			data?: { result?: { metric: Record<string, string>; value: [number, string] }[] };
		};
		return (body.data?.result ?? []).map((x) => ({ labels: x.metric, value: Number(x.value[1]) }));
	} catch {
		return null;
	}
}
