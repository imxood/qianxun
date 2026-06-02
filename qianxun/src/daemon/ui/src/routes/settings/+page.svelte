<script lang="ts">
	// Stage 9c — Settings 面板
	//
	// 4 个 section:
	//   1. 主题 — Light / Dark / System (3 选 1, 调 themeStore.setMode)
	//   2. 语言 — 简体中文 / English (2 选 1, 调 setLocale)
	//   3. Token — 当前 token 状态 + 重新生成 + 复制 + 撤销
	//   4. 关于 — 千寻 logo + 版本 + 链接 + daemon version
	//
	// 跟 TopBar 的主题/语言切换按钮**共享**底层 store, 互相同步.

	import { onMount } from 'svelte';
	import {
		Sun,
		Moon,
		Monitor,
		Languages,
		KeyRound,
		Copy,
		RefreshCw,
		LogOut,
		BookOpen,
		Code2,
		MessageSquareWarning,
		CheckCircle2,
		AlertTriangle,
		ExternalLink
	} from '@lucide/svelte';
	import { goto } from '$app/navigation';

	import Card from '$lib/components/ui/card/Card.svelte';
	import CardHeader from '$lib/components/ui/card/CardHeader.svelte';
	import CardTitle from '$lib/components/ui/card/CardTitle.svelte';
	import CardDescription from '$lib/components/ui/card/CardDescription.svelte';
	import CardContent from '$lib/components/ui/card/CardContent.svelte';
	import Button from '$lib/components/ui/button/Button.svelte';
	import Loading from '$lib/components/common/Loading.svelte';
	import ErrorBanner from '$lib/components/common/ErrorBanner.svelte';
	import PageHeader from '$lib/components/common/PageHeader.svelte';

	import { authStore } from '$lib/stores/auth.svelte';
	import { themeStore, type ThemeMode } from '$lib/stores/theme.svelte';
	import { setLocale, locale, t, ALL_LOCALES, type Locale } from '$lib/i18n';
	import { rotateAdminToken } from '$lib/api/settings';
	import { getStatus } from '$lib/api/system';
	import type { SystemStatus, TokenRotateResponse } from '$lib/types/api';

	// ─── Section 1: Theme ──────────────────────────────────────
	const themeOptions: { value: ThemeMode; labelKey: string; descKey: string; Icon: typeof Sun }[] = [
		{ value: 'light', labelKey: 'settings.theme.light', descKey: 'settings.theme.light_desc', Icon: Sun },
		{ value: 'dark', labelKey: 'settings.theme.dark', descKey: 'settings.theme.dark_desc', Icon: Moon },
		{ value: 'system', labelKey: 'settings.theme.system', descKey: 'settings.theme.system_desc', Icon: Monitor }
	];

	function selectTheme(value: ThemeMode): void {
		themeStore.setMode(value);
	}

	// ─── Section 2: Language ───────────────────────────────────
	// ALL_LOCALES 已有 { value, label }, 这里把 descKey 也补上.
	type LangOption = (typeof ALL_LOCALES)[number] & { descKey: string };
	const languageOptions: LangOption[] = [
		{ value: 'zh-CN', label: '简体中文', descKey: 'settings.language.zh_cn_desc' },
		{ value: 'en', label: 'English', descKey: 'settings.language.en_desc' }
	];

	function selectLocale(value: Locale): void {
		setLocale(value);
	}

	// ─── Section 3: Token ──────────────────────────────────────
	let rotating = $state(false);
	let rotateError = $state<string | null>(null);
	let lastRotated = $state<TokenRotateResponse | null>(null);
	let copied = $state(false);

	const tokenMask = $derived(
		authStore.token
			? authStore.token.slice(0, 6) + '…' + authStore.token.slice(-4)
			: null
	);

	async function handleRotate(): Promise<void> {
		rotating = true;
		rotateError = null;
		try {
			const resp = await rotateAdminToken();
			lastRotated = resp;
			authStore.setToken(resp.token);
		} catch (e) {
			rotateError = e instanceof Error ? e.message : t('settings.token.rotate_failed');
		} finally {
			rotating = false;
		}
	}

	async function handleCopy(): Promise<void> {
		if (!authStore.token) return;
		try {
			if (typeof navigator !== 'undefined' && navigator.clipboard) {
				await navigator.clipboard.writeText(authStore.token);
			} else if (typeof document !== 'undefined') {
				// fallback: 用临时 textarea
				const ta = document.createElement('textarea');
				ta.value = authStore.token;
				ta.style.position = 'fixed';
				ta.style.opacity = '0';
				document.body.appendChild(ta);
				ta.select();
				document.execCommand('copy');
				document.body.removeChild(ta);
			}
			copied = true;
			setTimeout(() => (copied = false), 1500);
		} catch (e) {
			rotateError = e instanceof Error ? e.message : 'copy failed';
		}
	}

	function handleRevoke(): void {
		const ok = window.confirm(t('settings.token.revoke_confirm'));
		if (!ok) return;
		authStore.clear();
		void goto('/');
	}

	// ─── Section 4: About ──────────────────────────────────────
	let daemonStatus = $state<SystemStatus | null>(null);
	let daemonError = $state<string | null>(null);

	async function loadDaemonStatus(): Promise<void> {
		try {
			daemonStatus = await getStatus();
			daemonError = null;
		} catch (e) {
			// /v1/system/status 是公开端点, 一般不会失败; 失败时给个 fallback
			daemonError = e instanceof Error ? e.message : 'unknown';
		}
	}

	// 跟 package.json 的 version — Vite 在 build 时会替换, dev 时拿不到
	// 走 import.meta.env.PACKAGE_VERSION 兜底; 拿不到时给 'dev'.
	const FRONTEND_VERSION =
		(typeof import.meta !== 'undefined' &&
			(import.meta as { env?: Record<string, string> }).env?.['PACKAGE_VERSION']) ||
		'dev';

	onMount(() => {
		void loadDaemonStatus();
	});

	// 响应 i18n 变化 — 当 locale store 变, 当前显示的 tokenMask 跟主题的
	// selected highlight 都需要重新评估. Svelte 5 已是 $derived 自动追踪.
	const currentLocale = $derived($locale);
	const currentTheme = $derived(themeStore.mode);
</script>

<PageHeader title={t('settings.title')} description={t('settings.desc')} />

<div class="flex flex-col gap-4" data-testid="settings-page">
	<!-- Section 1: Theme -->
	<Card data-testid="settings-theme-section">
		<CardHeader>
			<div class="flex items-center gap-2">
				<Sun class="text-muted-foreground h-4 w-4" />
				<CardTitle>{t('settings.theme.title')}</CardTitle>
			</div>
			<CardDescription>{t('settings.theme.desc')}</CardDescription>
		</CardHeader>
		<CardContent>
			<div class="grid grid-cols-1 gap-3 sm:grid-cols-3" data-testid="settings-theme-grid">
				{#each themeOptions as opt (opt.value)}
					{@const Icon = opt.Icon}
					{@const selected = currentTheme === opt.value}
					<button
						type="button"
						class="hover:bg-accent/40 flex flex-col items-start gap-1 rounded-md border p-3 text-left transition-colors {selected
							? 'border-primary bg-accent/30 ring-1 ring-primary'
							: 'border-border'}"
						aria-pressed={selected}
						onclick={() => selectTheme(opt.value)}
						data-testid={`settings-theme-${opt.value}`}
					>
						<div class="flex w-full items-center justify-between">
							<Icon class="h-4 w-4" />
							{#if selected}
								<CheckCircle2 class="text-primary h-4 w-4" />
							{/if}
						</div>
						<span class="text-sm font-medium">{t(opt.labelKey)}</span>
						<span class="text-muted-foreground text-xs">{t(opt.descKey)}</span>
					</button>
				{/each}
			</div>
		</CardContent>
	</Card>

	<!-- Section 2: Language -->
	<Card data-testid="settings-language-section">
		<CardHeader>
			<div class="flex items-center gap-2">
				<Languages class="text-muted-foreground h-4 w-4" />
				<CardTitle>{t('settings.language.title')}</CardTitle>
			</div>
			<CardDescription>{t('settings.language.desc')}</CardDescription>
		</CardHeader>
		<CardContent>
			<div class="grid grid-cols-1 gap-3 sm:grid-cols-2" data-testid="settings-language-grid">
				{#each languageOptions as opt (opt.value)}
					{@const selected = currentLocale === opt.value}
					<button
						type="button"
						class="hover:bg-accent/40 flex flex-col items-start gap-1 rounded-md border p-3 text-left transition-colors {selected
							? 'border-primary bg-accent/30 ring-1 ring-primary'
							: 'border-border'}"
						aria-pressed={selected}
						onclick={() => selectLocale(opt.value)}
						data-testid={`settings-language-${opt.value}`}
					>
						<div class="flex w-full items-center justify-between">
							<span class="text-sm font-medium">{opt.label}</span>
							{#if selected}
								<CheckCircle2 class="text-primary h-4 w-4" />
							{/if}
						</div>
						<span class="text-muted-foreground text-xs">{t(opt.descKey)}</span>
					</button>
				{/each}
			</div>
		</CardContent>
	</Card>

	<!-- Section 3: Token -->
	<Card data-testid="settings-token-section">
		<CardHeader>
			<div class="flex items-center gap-2">
				<KeyRound class="text-muted-foreground h-4 w-4" />
				<CardTitle>{t('settings.token.title')}</CardTitle>
			</div>
			<CardDescription>{t('settings.token.desc')}</CardDescription>
		</CardHeader>
		<CardContent>
			<div class="bg-muted/30 flex flex-col gap-3 rounded-md border p-3">
				<div class="flex flex-col gap-1">
					<span class="text-muted-foreground text-xs">{t('settings.token.current')}</span>
					{#if tokenMask}
						<code
							class="bg-background w-fit rounded border px-2 py-1 font-mono text-sm"
							data-testid="settings-token-mask"
						>
							{tokenMask}
						</code>
					{:else}
						<span
							class="text-muted-foreground text-xs italic"
							data-testid="settings-token-none"
						>
							{t('settings.token.none')}
						</span>
					{/if}
				</div>

				{#if lastRotated}
					<div
						class="text-muted-foreground flex items-center gap-1.5 text-xs"
						data-testid="settings-token-rotate-success"
					>
						<CheckCircle2 class="h-3.5 w-3.5 text-green-600 dark:text-green-400" />
						{t('settings.token.rotate_success')}
						<span class="font-mono">({Math.floor(lastRotated.expires_in / 3600)}h)</span>
					</div>
				{/if}

				{#if rotateError}
					<ErrorBanner message={rotateError} />
				{/if}

				<div class="flex flex-wrap items-center gap-2">
					<Button
						variant="outline"
						size="sm"
						onclick={handleRotate}
						disabled={rotating}
						data-testid="settings-token-rotate"
					>
						<RefreshCw class="h-3.5 w-3.5 {rotating ? 'animate-spin' : ''}" />
						{rotating ? t('settings.token.rotating') : t('settings.token.rotate')}
					</Button>
					<Button
						variant="outline"
						size="sm"
						onclick={handleCopy}
						disabled={!authStore.token}
						data-testid="settings-token-copy"
					>
						<Copy class="h-3.5 w-3.5" />
						{copied ? t('common.copied') : t('settings.token.copy')}
					</Button>
					<Button
						variant="destructive"
						size="sm"
						onclick={handleRevoke}
						disabled={!authStore.token}
						data-testid="settings-token-revoke"
					>
						<LogOut class="h-3.5 w-3.5" />
						{t('settings.token.revoke')}
					</Button>
				</div>

				{#if !authStore.token}
					<div
						class="text-muted-foreground flex items-center gap-1.5 text-xs"
					>
						<AlertTriangle class="h-3.5 w-3.5" />
						{t('settings.token.none')}
					</div>
				{/if}
			</div>
		</CardContent>
	</Card>

	<!-- Section 4: About -->
	<Card data-testid="settings-about-section">
		<CardHeader>
			<div class="flex items-center gap-3">
				<div
					class="flex h-10 w-10 items-center justify-center rounded-md text-base font-bold text-white"
					style="background: var(--qianxun-accent, #ff7a3d);"
				>
					千
				</div>
				<div class="flex flex-col">
					<CardTitle>{t('settings.about.title')}</CardTitle>
					<CardDescription>{t('settings.about.desc')}</CardDescription>
				</div>
			</div>
		</CardHeader>
		<CardContent>
			<div class="flex flex-col gap-3">
				<div class="grid grid-cols-1 gap-2 text-sm sm:grid-cols-2">
					<div class="flex flex-col gap-0.5">
						<span class="text-muted-foreground text-xs"
							>{t('settings.about.daemon_version')}</span
						>
						{#if daemonStatus}
							<code
								class="bg-muted w-fit rounded px-2 py-0.5 font-mono text-xs"
								data-testid="settings-about-daemon-version"
							>
								v{daemonStatus.version}
							</code>
						{:else if daemonError}
							<span class="text-muted-foreground text-xs italic">—</span>
						{:else}
							<Loading label="" />
						{/if}
					</div>
					<div class="flex flex-col gap-0.5">
						<span class="text-muted-foreground text-xs"
							>{t('settings.about.frontend_version')}</span
						>
						<code
							class="bg-muted w-fit rounded px-2 py-0.5 font-mono text-xs"
							data-testid="settings-about-frontend-version"
						>
							v{FRONTEND_VERSION}
						</code>
					</div>
				</div>

				<div class="flex flex-wrap items-center gap-2">
					<a
						href="https://github.com/maxu/qianxun"
						target="_blank"
						rel="noreferrer noopener"
						class="border-input hover:bg-accent inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs"
						data-testid="settings-about-github"
					>
						<Code2 class="h-3.5 w-3.5" />
						{t('settings.about.github')}
						<ExternalLink class="text-muted-foreground h-3 w-3" />
					</a>
					<a
						href="/docs"
						target="_blank"
						rel="noreferrer noopener"
						class="border-input hover:bg-accent inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs"
						data-testid="settings-about-docs"
					>
						<BookOpen class="h-3.5 w-3.5" />
						{t('settings.about.docs')}
					</a>
					<a
						href="https://github.com/maxu/qianxun/issues"
						target="_blank"
						rel="noreferrer noopener"
						class="border-input hover:bg-accent inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs"
						data-testid="settings-about-feedback"
					>
						<MessageSquareWarning class="h-3.5 w-3.5" />
						{t('settings.about.feedback')}
					</a>
				</div>
			</div>
		</CardContent>
	</Card>
</div>
