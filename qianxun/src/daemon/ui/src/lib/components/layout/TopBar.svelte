<script lang="ts">
	// Stage 7b TopBar — daemon 状态指示 + 主题切换 (light/dark/system) + 语言切换 (zh/en) + token 配置
	// 主题走 themeStore (mode-watcher); 语言走 i18n/locale store

	import { Circle, KeyRound, LogOut, RefreshCw, Sun, Moon, Monitor, Languages } from '@lucide/svelte';
	import { authStore } from '$lib/stores/auth.svelte';
	import { themeStore } from '$lib/stores/theme.svelte';
	import { setLocale, locale, t } from '$lib/i18n';
	import { onMount } from 'svelte';

	type Props = {
		onConfigureToken?: () => void;
	};

	let { onConfigureToken }: Props = $props();

	type DaemonState = 'connected' | 'unknown' | 'offline';
	let daemonState = $state<DaemonState>('unknown');
	let version = $state<string>('');

	async function refresh() {
		try {
			const r = await fetch('/v1/system/health', {
				headers: authStore.token
					? { Authorization: `Bearer ${authStore.token}` }
					: {}
			});
			if (r.ok) {
				daemonState = 'connected';
				const s = await fetch('/v1/system/status', {
					headers: authStore.token
						? { Authorization: `Bearer ${authStore.token}` }
						: {}
				});
				if (s.ok) {
					const j = (await s.json()) as { version?: string };
					version = j.version ?? '';
				}
			} else {
				daemonState = r.status === 401 ? 'unknown' : 'offline';
			}
		} catch {
			daemonState = 'offline';
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
		const t = setInterval(refresh, 10_000);
		return () => clearInterval(t);
	});

	const stateLabel = $derived(
		daemonState === 'connected'
			? t('topbar.connected')
			: daemonState === 'offline'
				? t('topbar.offline')
				: t('topbar.unauth')
	);
	const stateColor = $derived(
		daemonState === 'connected' ? '#22c55e' : daemonState === 'offline' ? '#ef4444' : '#a3a3a3'
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
	class="bg-background flex h-14 items-center justify-between border-b px-4"
	aria-label="顶部状态栏"
>
	<div class="flex items-center gap-2 text-sm">
		<Circle class="h-3 w-3" style="color: {stateColor};" fill={stateColor} />
		<span class="font-medium">{stateLabel}</span>
		{#if version}
			<span class="text-muted-foreground text-xs">v{version}</span>
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

	<div class="flex items-center gap-2 text-sm">
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
			class="border-input hover:bg-accent flex items-center gap-1 rounded-md border px-2 py-1 text-xs"
			title="language: {$locale}"
			aria-label="切换语言"
			onclick={toggleLang}
			data-testid="topbar-lang-toggle"
		>
			<Languages class="h-3.5 w-3.5" />
			{$locale === 'zh-CN' ? '中' : 'EN'}
		</button>

		<div class="text-muted-foreground flex items-center gap-1.5">
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
