<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import Moon from '@lucide/svelte/icons/moon';
	import Sun from '@lucide/svelte/icons/sun';
	import LogOut from '@lucide/svelte/icons/log-out';
	import { isCurrent, navFor } from '#lib/nav.ts';
	import type { LayoutProps } from './$types';

	let { data, children }: LayoutProps = $props();
	const nav = $derived(navFor(data.principal?.permissions ?? []));
	let theme: 'light' | 'dark' = $state('light');

	onMount(() => {
		const chosen = document.documentElement.dataset.theme;
		theme =
			chosen === 'dark' || chosen === 'light'
				? chosen
				: matchMedia('(prefers-color-scheme: dark)').matches
					? 'dark'
					: 'light';
	});

	function toggleTheme() {
		theme = theme === 'dark' ? 'light' : 'dark';
		document.documentElement.dataset.theme = theme;
		try {
			localStorage.setItem('broker-theme', theme);
		} catch {
			// Storage can be blocked; the choice then lasts until the page is reloaded.
		}
	}
</script>

<a class="skip" href="#main">Skip to the content</a>

<div class="shell">
	<aside class="sidebar" aria-label="Navigation">
		<a href="/" class="brand" aria-label="pacod-broker console: overview">
			<span class="mark" aria-hidden="true">BR</span>
			<span class="brand-text">
				<span class="brand-name">pacod-broker</span>
				<span class="brand-sub">Discovery &amp; reputation</span>
			</span>
		</a>
		<nav aria-label="Main">
			{#each nav as group (group.label)}
				<section>
					<h2>{group.label}</h2>
					<ul>
						{#each group.items as item (item.href)}
							<li>
								<a
									href={item.href}
									aria-current={isCurrent(item.href, page.url.pathname) ? 'page' : undefined}
									>{item.label}</a
								>
							</li>
						{/each}
					</ul>
				</section>
			{/each}
		</nav>
	</aside>

	<div class="column">
		<header class="topbar">
			{#if data.brokers.length > 1}
				<form method="POST" action="/switch" class="row switcher">
					<label for="broker" class="small muted">Broker</label>
					<select id="broker" name="broker" value={data.broker}>
						{#each data.brokers as b (b)}<option value={b}>{b}</option>{/each}
					</select>
					<button class="btn" type="submit">Dock</button>
				</form>
			{:else}
				<p class="small muted">
					Docked to <strong>{data.broker}</strong>{#if data.principal}
						· {data.principal.broker_id} v{data.principal.version}{/if}
				</p>
			{/if}
			<div class="spacer"></div>
			<button
				type="button"
				class="icon-button"
				aria-label={theme === 'dark' ? 'Switch to the light theme' : 'Switch to the dark theme'}
				onclick={toggleTheme}
			>
				{#if theme === 'dark'}<Sun size={18} />{:else}<Moon size={18} />{/if}
			</button>
			{#if data.principal?.mode === 'oidc'}
				<span class="small">{data.principal.name ?? data.principal.sub}</span>
				{#if data.identity}<a class="small" href={data.identity.account}>Account</a>{/if}
				<form method="POST" action="/auth/logout">
					<button type="submit" class="icon-button" aria-label="Sign out"
						><LogOut size={18} /></button
					>
				</form>
			{:else if data.principal}
				<span class="small muted" title="The broker runs without sign-in">open mode</span>
			{/if}
		</header>

		<main id="main" tabindex="-1">
			{#if data.apiProblem}
				<section class="card stack">
					<h1>The broker does not answer</h1>
					<p>{data.apiProblem}</p>
					<p class="muted">
						The console holds no state of its own; it shows the broker it is docked to. Check that
						the broker's admin listener (BROKER_ADMIN_LISTEN) is reachable from the console.
					</p>
				</section>
			{:else}
				{@render children()}
			{/if}
		</main>
	</div>
</div>

<style>
	.skip {
		position: absolute;
		left: var(--space-2);
		top: -4rem;
		z-index: 50;
		padding: var(--space-2) var(--space-3);
		border-radius: var(--radius);
		background: var(--accent);
		color: var(--on-accent);
	}

	.skip:focus {
		top: var(--space-2);
	}

	.shell {
		display: grid;
		grid-template-columns: var(--sidebar-width) minmax(0, 1fr);
		min-height: 100dvh;
	}

	.sidebar {
		position: sticky;
		top: 0;
		align-self: start;
		height: 100dvh;
		overflow-y: auto;
		border-right: 1px solid var(--sidebar-border);
		background: var(--sidebar);
		color: var(--sidebar-text);
	}

	.brand {
		display: flex;
		align-items: center;
		gap: var(--space-3);
		height: var(--topbar-height);
		padding: 0 var(--space-4);
		border-bottom: 1px solid var(--sidebar-border);
		color: inherit;
	}

	.brand:hover {
		color: inherit;
		text-decoration: none;
	}

	.mark {
		display: grid;
		place-items: center;
		width: 2rem;
		height: 2rem;
		border-radius: var(--radius);
		background: var(--accent);
		color: #ffffff;
		font-family: var(--font-display);
		font-size: 0.75rem;
		font-weight: 800;
	}

	.brand-text {
		display: flex;
		flex-direction: column;
		line-height: 1.15;
	}

	.brand-name {
		font-family: var(--font-display);
		font-weight: 800;
	}

	.brand-sub {
		color: var(--sidebar-muted);
		font-size: var(--text-xs);
	}

	nav section {
		padding: var(--space-3) var(--space-3) 0;
	}

	nav h2 {
		padding: 0 var(--space-2) var(--space-1);
		color: var(--sidebar-muted);
		font-family: var(--font-mono);
		font-size: 0.6875rem;
		font-weight: 500;
		letter-spacing: 0.12em;
		text-transform: uppercase;
	}

	nav ul {
		padding: 0;
		list-style: none;
	}

	nav a {
		display: block;
		padding: var(--space-2);
		border-radius: var(--radius-sm);
		color: var(--sidebar-text);
	}

	nav a:hover {
		background: var(--sidebar-hover);
		text-decoration: none;
	}

	nav a[aria-current='page'] {
		background: var(--sidebar-active);
		color: var(--sidebar-active-text);
		font-weight: 600;
	}

	.column {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}

	.topbar {
		display: flex;
		align-items: center;
		gap: var(--space-3);
		height: var(--topbar-height);
		padding: 0 var(--space-5, 1.5rem);
		border-bottom: 1px solid var(--border);
		background: var(--surface);
	}

	.spacer {
		flex: 1;
	}

	.switcher select {
		width: auto;
	}

	.icon-button {
		display: inline-grid;
		place-items: center;
		width: 2.25rem;
		height: 2.25rem;
		border: 0;
		border-radius: var(--radius);
		background: transparent;
		color: var(--text-muted);
		cursor: pointer;
	}

	.icon-button:hover {
		background: var(--surface-2);
		color: var(--text);
	}

	main {
		display: flex;
		flex-direction: column;
		gap: var(--space-5, 1.5rem);
		width: 100%;
		max-width: var(--content-width);
		padding: var(--space-5, 1.5rem);
	}

	main :global(dd),
	main :global(td) {
		overflow-wrap: anywhere;
	}

	/* On a phone the navigation is one strip that scrolls sideways, above the page. */
	@media (max-width: 52rem) {
		.shell {
			grid-template-columns: minmax(0, 1fr);
		}

		.sidebar {
			position: static;
			height: auto;
			overflow: visible;
			border-right: 0;
			border-bottom: 1px solid var(--sidebar-border);
		}

		.brand {
			height: 3rem;
		}

		nav {
			display: flex;
			overflow-x: auto;
			padding: var(--space-2) var(--space-3);
			gap: var(--space-1);
		}

		nav section {
			display: contents;
		}

		nav h2 {
			display: none;
		}

		nav ul {
			display: flex;
			gap: var(--space-1);
		}

		nav a {
			white-space: nowrap;
		}

		.topbar {
			flex-wrap: wrap;
			height: auto;
			padding: var(--space-2) var(--space-3);
		}

		main {
			padding: var(--space-3);
		}
	}
</style>
