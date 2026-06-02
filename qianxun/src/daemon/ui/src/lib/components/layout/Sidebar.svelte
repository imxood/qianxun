<script lang="ts">
	// Stage 7a Sidebar — 千寻 logo + 4 核心链接 + Settings/System 占位
	// 跟 docs/30_子项目规划/01b-daemon-web-console.md §2.3 一致

	import { page } from '$app/state';
	import {
		Brain,
		Sparkles,
		Server,
		Wrench,
		Settings as SettingsIcon,
		Activity
	} from '@lucide/svelte';

	type NavItem = {
		href: string;
		label: string;
		icon: typeof Brain;
		stage?: '7a' | '7b' | '7c';
	};

	const items: NavItem[] = [
		{ href: '/llm', label: 'LLM Providers', icon: Brain, stage: '7a' },
		{ href: '/skills', label: 'Skills', icon: Sparkles, stage: '7a' },
		{ href: '/mcp', label: 'MCP Servers', icon: Server, stage: '7a' },
		{ href: '/tools', label: 'Tools', icon: Wrench, stage: '7a' },
		{ href: '/settings', label: 'Settings', icon: SettingsIcon, stage: '7b' },
		{ href: '/system', label: 'System', icon: Activity, stage: '7b' }
	];

	function isActive(href: string): boolean {
		const p = page.url.pathname;
		return p === href || p.startsWith(href + '/');
	}
</script>

<aside
	class="bg-card text-card-foreground flex w-56 shrink-0 flex-col border-r"
	aria-label="主导航"
>
	<header class="flex h-14 items-center gap-2 border-b px-4">
		<div
			class="flex h-7 w-7 items-center justify-center rounded-md text-sm font-bold text-white"
			style="background: var(--qianxun-accent, #ff7a3d);"
		>
			千
		</div>
		<div class="flex flex-col">
			<span class="text-sm font-semibold">千寻 Console</span>
			<span class="text-muted-foreground text-[10px]">Daemon · Stage 7a</span>
		</div>
	</header>

	<nav class="flex flex-1 flex-col gap-0.5 p-2">
		{#each items as item (item.href)}
			{@const Icon = item.icon}
			{@const active = isActive(item.href)}
			<a
				href={item.href}
				class="group flex items-center gap-2.5 rounded-md px-2.5 py-2 text-sm transition-colors"
				class:bg-accent={active}
				class:text-accent-foreground={active}
				class:font-medium={active}
				class:text-muted-foreground={!active}
				class:hover:bg-accent={!active}
				class:hover:text-accent-foreground={!active}
				aria-current={active ? 'page' : undefined}
				data-testid={`nav-${item.href.replace('/', '')}`}
			>
				<Icon class="h-4 w-4" />
				<span class="flex-1">{item.label}</span>
				{#if item.stage === '7b'}
					<span
						class="rounded-full border border-dashed px-1.5 py-0 text-[9px] font-medium uppercase tracking-wide opacity-60"
					>
						7b
					</span>
				{/if}
			</a>
		{/each}
	</nav>

	<footer class="border-t p-3 text-[10px] text-muted-foreground">
		<div>Admin Console v0.1.0</div>
		<div class="mt-0.5">Local: 127.0.0.1:23900</div>
	</footer>
</aside>
