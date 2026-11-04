/**
 * Markdown to HTML for the descriptions providers write, safe by construction.
 *
 * Every character of the source is escaped first, so no HTML a provider writes can reach the
 * page; the converter then adds the few elements it knows: headings, paragraphs, lists, code
 * blocks and spans, emphasis, and links to http(s) addresses only. Images, raw HTML and other
 * schemes are shown as text. A small subset on purpose: descriptions are prose, and every
 * feature is a way in.
 */

const ESC: Record<string, string> = {
	'&': '&amp;',
	'<': '&lt;',
	'>': '&gt;',
	'"': '&quot;',
	"'": '&#39;'
};

export function escapeHtml(s: string): string {
	return s.replace(/[&<>"']/g, (c) => ESC[c]);
}

function inline(text: string): string {
	// Code spans first, so their content is not formatted further.
	const parts = text.split(/(`[^`]+`)/g);
	return parts
		.map((p) => {
			if (p.startsWith('`') && p.endsWith('`') && p.length > 1)
				return `<code>${escapeHtml(p.slice(1, -1))}</code>`;
			let s = escapeHtml(p);
			// [text](https://…): the address is escaped already; only http(s) becomes a link.
			s = s.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, (_m, label: string, href: string) => {
				return `<a href="${href}" rel="nofollow noopener noreferrer" target="_blank">${label}</a>`;
			});
			s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
			s = s.replace(/(^|[^*])\*([^*\s][^*]*)\*/g, '$1<em>$2</em>');
			return s;
		})
		.join('');
}

export function renderMarkdown(src: string): string {
	const lines = src.replace(/\r\n?/g, '\n').split('\n');
	const out: string[] = [];
	let para: string[] = [];
	let list: { kind: 'ul' | 'ol'; items: string[] } | null = null;
	const flushPara = () => {
		if (para.length) out.push(`<p>${inline(para.join(' '))}</p>`);
		para = [];
	};
	const flushList = () => {
		if (list)
			out.push(
				`<${list.kind}>${list.items.map((i) => `<li>${inline(i)}</li>`).join('')}</${list.kind}>`
			);
		list = null;
	};
	for (let i = 0; i < lines.length; i++) {
		const line = lines[i];
		const fence = line.match(/^```\s*([\w-]*)\s*$/);
		if (fence) {
			flushPara();
			flushList();
			const code: string[] = [];
			i++;
			while (i < lines.length && !/^```\s*$/.test(lines[i])) code.push(lines[i++]);
			const lang = fence[1] ? ` data-lang="${escapeHtml(fence[1])}"` : '';
			out.push(`<pre${lang}><code>${escapeHtml(code.join('\n'))}</code></pre>`);
			continue;
		}
		const h = line.match(/^(#{1,4})\s+(.*)$/);
		if (h) {
			flushPara();
			flushList();
			// Headings start at h3: the page's own headings are above them.
			const level = Math.min(6, h[1].length + 2);
			out.push(`<h${level}>${inline(h[2])}</h${level}>`);
			continue;
		}
		const ul = line.match(/^\s*[-*]\s+(.*)$/);
		const ol = line.match(/^\s*\d+[.)]\s+(.*)$/);
		if (ul || ol) {
			flushPara();
			const kind = ul ? 'ul' : 'ol';
			if (list && list.kind !== kind) flushList();
			if (!list) list = { kind, items: [] };
			list.items.push((ul ?? ol)![1]);
			continue;
		}
		if (line.trim() === '') {
			flushPara();
			flushList();
			continue;
		}
		flushList();
		para.push(line.trim());
	}
	flushPara();
	flushList();
	return out.join('\n');
}
