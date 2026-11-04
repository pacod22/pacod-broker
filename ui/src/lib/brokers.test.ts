import { describe, expect, it } from 'vitest';
import { chosen, parseBrokers } from './brokers';

describe('parseBrokers', () => {
	it('reads pairs and trims slashes', () => {
		const b = parseBrokers('default=http://a:8061/, eu=https://b.example');
		expect(b).toEqual([
			{ name: 'default', origin: 'http://a:8061' },
			{ name: 'eu', origin: 'https://b.example' }
		]);
		expect(chosen(b, 'eu').name).toBe('eu');
		expect(chosen(b, 'nope').name).toBe('default');
	});

	it('refuses what it cannot read', () => {
		expect(() => parseBrokers('')).toThrow(/no broker/);
		expect(() => parseBrokers('x')).toThrow(/name=origin/);
		expect(() => parseBrokers('A=http://a')).toThrow(/not a name/);
		expect(() => parseBrokers('a=ftp://a')).toThrow(/http/);
		expect(() => parseBrokers('a=http://a,a=http://b')).toThrow(/twice/);
	});
});
