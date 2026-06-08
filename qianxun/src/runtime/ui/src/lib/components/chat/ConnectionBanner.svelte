<!--
  ConnectionBanner.svelte — Daemon 连接状态降级 UI (Stage 9c)
  跟 qianxun-desktop/src/lib/components/chat/ConnectionBanner.svelte 对齐
  区别: Web Console 用简单 3 态 (connected / disconnected / unknown) — 没有
  Tauri 那套 connectionStore.healthCheck 复杂状态机, 这里走 TopBar 已经检测
  过的 daemon 状态, 弹简单的 "daemon 不可达" banner.

  Stage 9c 简化: 从 authStore + 简单的 fetch /v1/system/health 检测
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { authStore } from '$lib/stores/auth.svelte';
	import { RefreshCw, WifiOff } from '@lucide/svelte';

	type Props = {
		/// 可选: 父级 onRetry 钩子 (例如刷新项目/会话列表)
		onRetry?: () => void;
	};

	let { onRetry }: Props = $props();

	let healthy = $state<boolean | null>(null);
	let checking = $state(false);

	async function check() {
		if (checking) return;
		checking = true;
		try {
			const headers: Record<string, string> = { Accept: 'application/json' };
			if (authStore.token) headers['Authorization'] = `Bearer ${authStore.token}`;
			const r = await fetch('/v1/system/health', { headers });
			healthy = r.ok;
		} catch {
			healthy = false;
		} finally {
			checking = false;
		}
	}

	function retry() {
		void check();
		onRetry?.();
	}

	onMount(() => {
		void check();
	});
</script>

{#if healthy === false}
	<div
		role="alert"
		class="flex items-center justify-between gap-2 border-l-4 border-red-500 bg-red-50 px-4 py-2 text-sm text-red-900 dark:bg-red-950 dark:text-red-200"
		data-testid="connection-banner-disconnected"
	>
		<div class="flex items-center gap-2">
			<WifiOff class="size-3.5" />
			<span><strong>Daemon 未连接</strong> — 检查 127.0.0.1:23900 是否启动</span>
		</div>
		<button
			type="button"
			onclick={retry}
			class="rounded border border-red-300 px-3 py-1 text-xs font-medium hover:bg-red-100 dark:border-red-800 dark:hover:bg-red-900"
			data-testid="connection-banner-retry"
		>
			<RefreshCw class="inline size-3" /> 立即重试
		</button>
	</div>
{:else if healthy === null}
	<div
		role="status"
		aria-live="polite"
		class="flex items-center gap-2 border-l-4 border-yellow-500 bg-yellow-50 px-4 py-2 text-sm text-yellow-900 dark:bg-yellow-950 dark:text-yellow-200"
	>
		<span class="inline-block size-2 animate-pulse rounded-full bg-yellow-500"></span>
		<span>检查 daemon 状态…</span>
	</div>
{/if}
