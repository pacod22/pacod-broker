import { describe, expect, it } from 'vitest';
import { renderMarkdown } from './markdown';

describe('renderMarkdown', () => {
	it('renders the subset', () => {
		const html = renderMarkdown(
			'## Title\nSome **bold** and *em* and `code`.\n\n- one\n- two\n\n1. first\n\n```bash\ncurl <x>\n```'
		);
		expect(html).toContain('<h4>Title</h4>');
		expect(html).toContain('<strong>bold</strong>');
		expect(html).toContain('<em>em</em>');
		expect(html).toContain('<code>code</code>');
		expect(html).toContain('<ul><li>one</li><li>two</li></ul>');
		expect(html).toContain('<ol><li>first</li></ol>');
		expect(html).toContain('<pre data-lang="bash"><code>curl &lt;x&gt;</code></pre>');
	});

	it('never lets HTML or unsafe links through', () => {
		const html = renderMarkdown(
			'<script>alert(1)</script> <img src=x onerror=alert(1)> [a](javascript:alert(1)) [b](https://ok.example/x?a=1&b=2)'
		);
		expect(html).not.toContain('<script');
		expect(html).not.toContain('<img');
		expect(html).not.toContain('href="javascript');
		expect(html).toContain('&lt;script&gt;');
		expect(html).toContain('href="https://ok.example/x?a=1&amp;b=2"');
	});

	it('does not break out of attributes', () => {
		const html = renderMarkdown('[x](https://a.example/"onmouseover=alert(1))');
		expect(html).not.toMatch(/href="[^"]*"onmouseover/);
	});
});
