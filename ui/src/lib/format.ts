/** Formatting for the console's pages. */

export function dateTime(v: string | null | undefined): string {
	if (!v) return '—';
	const d = new Date(v);
	if (Number.isNaN(d.getTime())) return v;
	return d.toISOString().replace('T', ' ').slice(0, 16) + ' UTC';
}

export function ago(v: string | null | undefined, now = Date.now()): string {
	if (!v) return '—';
	const s = Math.round((now - new Date(v).getTime()) / 1000);
	if (Number.isNaN(s)) return v;
	const abs = Math.abs(s);
	const unit =
		abs < 90
			? `${abs} s`
			: abs < 5400
				? `${Math.round(abs / 60)} min`
				: abs < 129600
					? `${Math.round(abs / 3600)} h`
					: `${Math.round(abs / 86400)} d`;
	return s >= 0 ? `${unit} ago` : `in ${unit}`;
}

export function money(v: number | null | undefined, currency = ''): string {
	if (v === null || v === undefined) return '—';
	const digits = v !== 0 && Math.abs(v) < 0.01 ? 4 : 2;
	return `${v.toFixed(digits)}${currency ? ' ' + currency : ''}`;
}

export function pct(v: number | null | undefined): string {
	return v === null || v === undefined ? '—' : `${Math.round(v * 100)}%`;
}

/** The rate names as people read them. */
export const RATE_LABELS: Record<string, string> = {
	input: 'Input / 1M tokens',
	output: 'Output / 1M tokens',
	cache_read: 'Cache read / 1M',
	cache_write: 'Cache write / 1M',
	reasoning: 'Reasoning / 1M',
	per_request: 'Per request',
	per_call: 'Per call',
	per_image: 'Per image',
	per_audio_second: 'Per audio second',
	per_second: 'Per second',
	per_hour: 'Per hour',
	per_gpu_hour: 'Per GPU hour'
};
