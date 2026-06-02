<script lang="ts">
	// Stage 7b Sidebar — 千寻 logo + 管理区 (LLM/Skills/MCP/Tools) + 运维区 (Memory/Sessions/Config/System)
	// 移除 "7b" 阶段标签 — Stage 7b 已落地, 全部是生产可用路由

	import { page } from '$app/state';
	import { Brain, Sparkles, Server, Wrench, Database, MessagesSquare, FileCog, Activity } from '@lucide/svelte';
	import { t } from '$lib/i18n';

	type NavItem = {
		href: string;
		label: string;
		i18n: string;
		icon: typeof Brain;
	};

	const mgmtItems: NavItem[] = [
		{ href: '/llm', label: 'LLM Providers', i18n: 'nav.llm', icon: Brain },
		{ href: '/skills', label: 'Skills', i18n: 'nav.skills', icon: Sparkles },
		{ href: '/mcp', label: 'MCP Servers', i18n: 'nav.mcp', icon: Server },
		{ href: '/tools', label: 'Tools', i18n: 'nav.tools', icon: Wrench }
	];

	const opsItems: NavItem[] = [
		{ href: '/memory', label: 'Memory', i18n: 'nav.memory', icon: Database },
		{ href: '/sessions', label: 'Chat Sessions', i18n: 'nav.sessions', icon: MessagesSquare },
		{ href: '/config', label: 'Config', i18n: 'nav.config', icon: FileCog },
		{ href: '/system', label: 'System', i18n: 'nav.system', icon: Activity }
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
			<span class="text-sm font-semibold">{t('app.title')}</span>
			<span class="text-muted-foreground text-[10px]">{t('app.subtitle')}</span>
		</div>
	</header>

	<nav class="flex flex-1 flex-col gap-1 overflow-y-auto p-2">
		<!-- 管理区 -->
		<div class="text-muted-foreground px-2 pt-1 pb-1 text-[10px] font-medium uppercase tracking-wider">
			{t('nav.management')}
		</div>
		{#each mgmtItems as item (item.href)}
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
				<span class="flex-1">{t(item.i18n)}</span>
			</a>
		{/each}

		<!-- 分隔 -->
		<div class="border-border my-1 border-t"></div>

		<!-- 运维区 -->
		<div class="text-muted-foreground px-2 pt-1 pb-1 text-[10px] font-medium uppercase tracking-wider">
			{t('nav.operations')}
		</div>
		{#each opsItems as item (item.href)}
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
				<span class="flex-1">{t(item.i18n)}</span>
			</a>
		{/each}
	</nav>

	<footer class="border-t p-3 text-[10px] text-muted-foreground">
		<div>Admin Console v0.2.0</div>
		<div class="mt-0.5">Local: 127.0.0.1:23900</div>
	</footer>
</aside>
