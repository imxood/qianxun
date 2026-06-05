<script lang="ts">
	// Stage 9c +layout.svelte
	// - Sidebar (drawer 移动端 + 固定桌面端) + TopBar (汉堡)
	// - mode-watcher 集成 (Stage 7b)
	// - authStore 初始化 + token 弹窗
	// - 主题初始化
	// - 响应式主区 padding
	// - svelte:boundary 包裹 slot, 组件崩溃 fallback
	// - 离线检测 banner (daemon unreachable 时显示)
	// - Sidebar/TopBar 接入 connectionStore (集中健康检查)

	import '../app.css';
	import { onMount } from 'svelte';
	import { ModeWatcher } from 'mode-watcher';
	import { WifiOff, RefreshCw } from '@lucide/svelte';
	import Sidebar from '$lib/components/layout/Sidebar.svelte';
	import TopBar from '$lib/components/layout/TopBar.svelte';
	import PasswordDialog from '$lib/components/auth/PasswordDialog.svelte';
	import { authStore } from '$lib/stores/auth.svelte';
	import { themeStore } from '$lib/stores/theme.svelte';
	import { connectionStore } from '$lib/stores/connection.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';

	let { children } = $props();

	let tokenDialogOpen = $state(false);
	let boundaryError = $state<Error | null>(null);

	// 2026-06-05 fix v9: authStore.init() 提到 script 顶层, 在所有 child
	// component mount 之前跑. 之前在 onMount 内跑, 但 Svelte 5 mount 顺序
	// child-first, child 的 onMount (e.g. /skills +page.svelte 的 refresh())
	// 先于 layout onMount 跑, 此时 authStore.token=null, fetchWithAuth 不带
	// Authorization 头 → 401. module 顶层 code 在 component 初始化时跑 (在
	// 任何 onMount 之前), 完美.
	authStore.init();
	themeStore.init();

	function onAuthFailed() {
		tokenDialogOpen = true;
	}

	async function manualRecheck() {
		await connectionStore.checkReachable();
	}

	onMount(() => {
		// 没 token 且已初始化 → 弹框
		if (!authStore.isAuthenticated) {
			tokenDialogOpen = true;
		}

		window.addEventListener('qianxun:auth:failed', onAuthFailed);
		return () => {
			window.removeEventListener('qianxun:auth:failed', onAuthFailed);
		};
	});
</script>

<ModeWatcher defaultMode="system" />

<div class="bg-background flex h-screen w-screen overflow-hidden">
	<Sidebar />
	<div class="flex min-w-0 flex-1 flex-col">
		<!-- 离线检测 banner (Stage 9c C.3) -->
		{#if !connectionStore.daemonReachable}
			<div
				role="alert"
				data-testid="connection-banner"
				class="flex items-center gap-2 border-b border-red-300 bg-red-500/15 px-3 py-2 text-xs text-red-700 sm:text-sm dark:text-red-300"
			>
				<WifiOff class="h-3.5 w-3.5 shrink-0" />
				<span class="flex-1">
					无法连接 daemon — 确认 daemon 在跑 (<code class="bg-red-500/20 rounded px-1">qx --daemon --port 23900</code>)
					{#if connectionStore.lastError}
						<span class="text-red-600/70 dark:text-red-400/70">— {connectionStore.lastError}</span>
					{/if}
				</span>
				<button
					type="button"
					class="inline-flex items-center gap-1 rounded-md border border-red-300 bg-red-500/10 px-2 py-1 text-xs hover:bg-red-500/20"
					onclick={manualRecheck}
					data-testid="connection-banner-retry"
				>
					<RefreshCw class="h-3 w-3" />
					重试
				</button>
			</div>
		{/if}

		<TopBar onConfigureToken={() => (tokenDialogOpen = true)} />

		<main
			class="flex-1 overflow-auto p-4 sm:p-6 lg:p-8"
			data-testid="layout-main"
		>
			<svelte:boundary onerror={(e) => {
				boundaryError = e instanceof Error ? e : new Error(String(e));
				console.error('[layout boundary]', e);
			}}>
				{@render children?.()}

				{#snippet failed(error, reset)}
					{@const errMsg = error instanceof Error ? error.message : String(error)}
					<div
						role="alert"
						class="border-destructive/40 bg-destructive/5 text-destructive flex flex-col gap-3 rounded-md border p-6"
						data-testid="boundary-fallback"
					>
						<div>
							<div class="text-base font-semibold">页面渲染出错</div>
							<div class="text-muted-foreground mt-1 text-sm">
								{errMsg}
							</div>
						</div>
						<button
							type="button"
							class="border-input hover:bg-accent inline-flex w-fit items-center gap-2 rounded-md border px-3 py-1.5 text-sm"
							onclick={() => {
								boundaryError = null;
								reset();
							}}
						>
							<RefreshCw class="h-3.5 w-3.5" />
							重试
						</button>
					</div>
				{/snippet}
			</svelte:boundary>
		</main>
	</div>
</div>

<PasswordDialog open={tokenDialogOpen} onOpenChange={(v) => (tokenDialogOpen = v)} />
