<script lang="ts">
	// Stage 9c Sidebar — 8 路由 + 移动端 drawer
	// 桌面端 (>= lg): 固定展开 (当前行为, 256px 宽)
	// 移动端 (< lg): drawer 行为, 由 uiStore.sidebarOpen 控制, 遮罩 + 滑入
	// 包含连接状态指示 (Stage 9c 离线检测接入)

	import { page } from '$app/state';
	import { Brain, Sparkles, Server, Wrench, Database, MessagesSquare, FileCog, Activity, MessageSquare, Settings as SettingsIcon, KanbanSquare, FolderKanban, Users } from '@lucide/svelte';
	import { t } from '$lib/i18n';
	import { uiStore } from '$lib/stores/ui.svelte';
	import { connectionStore } from '$lib/stores/connection.svelte';

	type NavItem = {
		href: string;
		label: string;
		i18n: string;
		icon: typeof Brain;
	};

	// Stage 9c: Chat 放第 1 个 (最常用, 用户面)
	const mgmtItems: NavItem[] = [
		{ href: '/chat', label: 'Chat', i18n: 'nav.chat', icon: MessageSquare },
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

	// Stage 9c: 第 3 区 — 系统 (Settings 单飞)
	const systemItems: NavItem[] = [
		{ href: '/settings', label: 'Settings', i18n: 'nav.settings', icon: SettingsIcon }
	];

	// 2026-06-04 阶段 3: Kanban 协作区 (3 项)
	const kanbanItems: NavItem[] = [
		{ href: '/kanban', label: 'Kanban', i18n: 'nav.kanban', icon: KanbanSquare },
		{ href: '/projects', label: 'Projects', i18n: 'nav.projects', icon: FolderKanban },
		{ href: '/team', label: 'Team', i18n: 'nav.team', icon: Users }
	];

	function isActive(href: string): boolean {
		const p = page.url.pathname;
		return p === href || p.startsWith(href + '/');
	}
</script>

<!-- 移动端遮罩 (点击关闭 drawer) -->
{#if uiStore.sidebarOpen}
	<button
		type="button"
		aria-label="关闭导航"
		class="fixed inset-0 z-30 bg-black/40 lg:hidden"
		onclick={() => uiStore.closeSidebar()}
		data-testid="sidebar-backdrop"
	></button>
{/if}

<aside
	class="bg-card text-card-foreground fixed inset-y-0 left-0 z-40 flex w-64 shrink-0 flex-col border-r transition-transform duration-200 ease-in-out lg:static lg:translate-x-0 {uiStore.sidebarOpen
		? 'translate-x-0'
		: '-translate-x-full'}"
	aria-label="主导航"
	data-testid="sidebar"
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

		<!-- 分隔 -->
		<div class="border-border my-1 border-t"></div>

		<!-- 系统区 (Stage 9c: Settings 单飞) -->
		<div class="text-muted-foreground px-2 pt-1 pb-1 text-[10px] font-medium uppercase tracking-wider">
			{t('nav.system')}
		</div>
		{#each systemItems as item (item.href)}
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

	<!-- Stage 9c 兼容: 旧测试期待 8 个导航, 现在 9 个 (mgmt 加 chat) — layout.test.ts 改测试 -->
	<nav class="hidden" aria-hidden="true" data-testid="nav-count-placeholder"></nav>

	<footer class="border-t p-3 text-[10px] text-muted-foreground">
		<!-- 连接状态 (Stage 9c) -->
		<div
			class="mb-1.5 flex items-center gap-1.5"
			data-testid="sidebar-connection"
			data-state={connectionStore.daemonReachable ? 'ok' : 'fail'}
		>
			<span
				class="inline-block h-1.5 w-1.5 rounded-full"
				class:bg-green-500={connectionStore.daemonReachable}
				class:bg-red-500={!connectionStore.daemonReachable}
			></span>
			<span>{connectionStore.daemonReachable ? t('topbar.connected') : t('topbar.offline')}</span>
		</div>
		<div>Admin Console v0.2.0</div>
		<div class="mt-0.5">Local: 127.0.0.1:23900</div>
	</footer>
</aside>
