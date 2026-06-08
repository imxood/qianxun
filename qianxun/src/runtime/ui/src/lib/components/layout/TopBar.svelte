<script lang="ts">
	// Stage 9c TopBar — daemon 状态指示 + 主题切换 + 语言切换 + token 配置 + 汉堡按钮 (移动端)
	// 主题走 themeStore (mode-watcher); 语言走 i18n/locale store; 移动端汉堡 → 切换 sidebar drawer
	// 复用 connectionStore 的可达性 (Stage 9c 集中化) 而不是自己再 fetch

	import {
		Circle,
		KeyRound,
		LogOut,
		RefreshCw,
		Sun,
		Moon,
		Monitor,
		Languages,
		Menu
	} from '@lucide/svelte';
	import { authStore } from '$lib/stores/auth.svelte';
	import { themeStore } from '$lib/stores/theme.svelte';
	import { connectionStore } from '$lib/stores/connection.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import { setLocale, locale, t } from '$lib/i18n';
	import { onMount } from 'svelte';

	type Props = {
		onConfigureToken?: () => void;
	};

	let { onConfigureToken }: Props = $props();

	let version = $state<string>('');

	async function refresh() {
		const result = await connectionStore.checkReachable();
		if (result.ok) {
			try {
				const s = await fetch('/v1/system/status', {
					headers: authStore.token
						? { Authorization: `Bearer ${authStore.token}` }
						: {}
				});
				if (s.ok) {
					const j = (await s.json()) as { version?: string };
					version = j.version ?? '';
				}
			} catch {
				/* ignore version fetch failure */
			}
		}
	}

	function logout() {
		authStore.clear();
	}

	function toggleTheme() {
		themeStore.toggle();
	}

	function toggleLang() {
		const cur = $locale;
		setLocale(cur === 'zh-CN' ? 'en' : 'zh-CN');
	}

	onMount(() => {
		void refresh();
		const stop = connectionStore.startHealthLoop(10_000);
		return stop;
	});

	const stateLabel = $derived(
		connectionStore.daemonReachable
			? t('topbar.connected')
			: connectionStore.lastError
				? t('topbar.offline')
				: t('topbar.unauth')
	);
	const stateColor = $derived(
		connectionStore.daemonReachable ? '#22c55e' : connectionStore.lastError ? '#ef4444' : '#a3a3a3'
	);
	const tokenMask = $derived(
		authStore.token ? authStore.token.slice(0, 6) + '…' + authStore.token.slice(-4) : t('topbar.set_token')
	);

	const ThemeIcon = $derived(themeStore.mode === 'light' ? Sun : themeStore.mode === 'dark' ? Moon : Monitor);
	const themeTitle = $derived(
		themeStore.mode === 'light'
			? t('topbar.theme_light')
			: themeStore.mode === 'dark'
				? t('topbar.theme_dark')
				: t('topbar.theme_system')
	);
</script>

<header
	class="bg-background flex h-14 items-center justify-between border-b px-3 sm:px-4"
	aria-label="顶部状态栏"
>
	<div class="flex items-center gap-2 text-sm">
		<!-- 汉堡菜单按钮 (仅移动端) -->
		<button
			type="button"
			class="hover:bg-accent inline-flex h-8 w-8 items-center justify-center rounded-md lg:hidden"
			title="打开导航"
			aria-label="打开导航"
			aria-expanded={uiStore.sidebarOpen}
			onclick={() => uiStore.toggleSidebar()}
			data-testid="topbar-hamburger"
		>
			<Menu class="h-4 w-4" />
		</button>

		<Circle class="h-3 w-3" style="color: {stateColor};" fill={stateColor} />
		<span class="font-medium" data-testid="topbar-state-label">{stateLabel}</span>
		{#if version}
			<span class="text-muted-foreground hidden text-xs sm:inline">v{version}</span>
		{/if}
		<button
			type="button"
			class="text-muted-foreground hover:text-foreground ml-1"
			title="刷新状态"
			aria-label="刷新状态"
			onclick={refresh}
		>
			<RefreshCw class="h-3.5 w-3.5" />
		</button>
	</div>

	<div class="flex items-center gap-1.5 text-sm sm:gap-2">
		<!-- 主题切换 -->
		<button
			type="button"
			class="border-input hover:bg-accent rounded-md border px-2 py-1"
			title="theme: {themeTitle}"
			aria-label="切换主题"
			onclick={toggleTheme}
			data-testid="topbar-theme-toggle"
		>
			<ThemeIcon class="h-3.5 w-3.5" />
		</button>

		<!-- 语言切换 -->
		<button
			type="button"
			class="border-input hover:bg-accent hidden items-center gap-1 rounded-md border px-2 py-1 text-xs sm:inline-flex"
			title="language: {$locale}"
			aria-label="切换语言"
			onclick={toggleLang}
			data-testid="topbar-lang-toggle"
		>
			<Languages class="h-3.5 w-3.5" />
			{$locale === 'zh-CN' ? '中' : 'EN'}
		</button>

		<!-- 移动端语言切换: 简化 (只显示文字) -->
		<button
			type="button"
			class="border-input hover:bg-accent inline-flex items-center rounded-md border px-2 py-1 text-xs sm:hidden"
			title="language: {$locale}"
			aria-label="切换语言"
			onclick={toggleLang}
		>
			{$locale === 'zh-CN' ? '中' : 'EN'}
		</button>

		<div class="text-muted-foreground hidden items-center gap-1.5 sm:flex">
			<KeyRound class="h-3.5 w-3.5" />
			<code class="bg-muted rounded px-1.5 py-0.5 text-xs">{tokenMask}</code>
		</div>
		<button
			type="button"
			class="border-input hover:bg-accent rounded-md border px-2.5 py-1 text-xs"
			onclick={onConfigureToken}
			data-testid="topbar-configure-token"
		>
			{authStore.token ? t('topbar.change_token') : t('topbar.set_token')}
		</button>
		{#if authStore.token}
			<button
				type="button"
				class="text-muted-foreground hover:text-foreground"
				title={t('topbar.logout')}
				aria-label={t('topbar.logout')}
				onclick={logout}
			>
				<LogOut class="h-3.5 w-3.5" />
			</button>
		{/if}
	</div>
</header>
