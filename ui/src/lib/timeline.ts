/**
 * Geometry of the timelines on a resource's page: when it is available over the next days,
 * and when its price changes. Pure, so it is tested without a browser.
 */

export interface Span {
	from: string;
	to: string;
}

export interface Bar {
	x: number;
	width: number;
	label: string;
}

/** Positions of spans in a band `width` wide covering [from, to). */
export function bars(
	spans: (Span & { label?: string })[],
	from: string,
	to: string,
	width: number
): Bar[] {
	const a = new Date(from).getTime();
	const b = new Date(to).getTime();
	const total = Math.max(1, b - a);
	return spans
		.map((s) => {
			const x0 = Math.max(a, new Date(s.from).getTime());
			const x1 = Math.min(b, new Date(s.to).getTime());
			return {
				x: ((x0 - a) / total) * width,
				width: (Math.max(0, x1 - x0) / total) * width,
				label: s.label ?? ''
			};
		})
		.filter((r) => r.width > 0);
}

/** Day ticks: one per midnight UTC in [from, to). */
export function dayTicks(from: string, to: string, width: number): { x: number; label: string }[] {
	const a = new Date(from).getTime();
	const b = new Date(to).getTime();
	const out: { x: number; label: string }[] = [];
	const d = new Date(a);
	d.setUTCHours(0, 0, 0, 0);
	if (d.getTime() < a) d.setUTCDate(d.getUTCDate() + 1);
	while (d.getTime() < b) {
		out.push({ x: ((d.getTime() - a) / (b - a)) * width, label: d.toISOString().slice(5, 10) });
		d.setUTCDate(d.getUTCDate() + 1);
	}
	return out;
}

/** The spans between price changes, each with the source in force (`base`, `window:night`…). */
export function priceSpans(
	changes: { at: string; source: string }[],
	to: string
): (Span & { label: string })[] {
	return changes.map((c, i) => ({ from: c.at, to: changes[i + 1]?.at ?? to, label: c.source }));
}
