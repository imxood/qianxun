<!--
  ConnectionBanner.svelte — Daemon 连接状态降级 UI (Stage 4)
  与 docs/30_子项目规划/03-tauri-desktop.md §10.1/§10.2 完全一致.
  渲染模型: 4 态 (offline / reconnecting / degraded / connected)
  - connected     → 不渲染任何内容
  - reconnecting  → 黄色条幅 + 脉冲点 + "第 N 次重试"
  - degraded      → 红色条幅 + "立即重试" 按钮
  - offline       → 红色条幅 + "立即重试" 按钮 (与 degraded 共享样式, 但文案区分)

  Stage 4 简化: VPS 状态 (VpsStore) 暂不接入本组件, 留 Settings 页
  ("VPS 不可达, 团队功能暂时不可用", 见 §10.4).
-->
<script lang="ts">
	import { connectionStore } from "$lib/stores/connection.svelte";
	import { t } from "$lib/i18n";

	type Props = {
		/// 可选: 父级 onRetry 钩子 (例如刷新项目/会话列表)
		onRetry?: () => void;
	};

	let { onRetry }: Props = $props();

	/// ago 秒数 (degraded 状态显示, 标识最近一次失败距今)
	const ago = $derived.by(() => {
		const ts = connectionStore.lastError?.ts ?? Date.now();
		return Math.max(0, Math.floor((Date.now() - ts) / 1000));
	});
</script>

{#if connectionStore.daemonState === "reconnecting"}
	<div
		role="status"
		aria-live="polite"
		class="flex items-center gap-2 border-l-4 border-yellow-500 bg-yellow-50 px-4 py-2 text-sm text-yellow-900 dark:bg-yellow-950 dark:text-yellow-200"
	>
		<span class="inline-block size-2 animate-pulse rounded-full bg-yellow-500"></span>
		<span>{t("connection.reconnecting")} ({t("retry")} {connectionStore.attempt})</span>
	</div>
{:else if connectionStore.daemonState === "degraded"}
	<div
		role="alert"
		class="flex items-center justify-between gap-2 border-l-4 border-red-500 bg-red-50 px-4 py-2 text-sm text-red-900 dark:bg-red-950 dark:text-red-200"
	>
		<div class="flex items-center gap-2">
			<span class="inline-block size-2 rounded-full bg-red-500"></span>
			<span>
				<strong>{t("connection.degraded")}</strong>
				{#if connectionStore.lastError}
					<span class="ml-1 opacity-75"
						>({ago}s 前: {connectionStore.lastError.message})</span
					>
				{/if}
			</span>
		</div>
		<button
			type="button"
			onclick={() => {
				connectionStore.retry();
				onRetry?.();
			}}
			class="rounded border border-red-300 px-3 py-1 text-xs font-medium hover:bg-red-100 dark:border-red-800 dark:hover:bg-red-900"
		>
			{t("retry")}
		</button>
	</div>
{:else if connectionStore.daemonState === "offline"}
	<div
		role="alert"
		class="flex items-center justify-between gap-2 border-l-4 border-zinc-500 bg-zinc-50 px-4 py-2 text-sm text-zinc-900 dark:bg-zinc-900 dark:text-zinc-200"
	>
		<div class="flex items-center gap-2">
			<span class="inline-block size-2 rounded-full bg-zinc-500"></span>
			<span>
				<strong>{t("connection.offline")}</strong>
				{#if connectionStore.lastError}
					<span class="ml-1 opacity-75"
						>({ago}s 前: {connectionStore.lastError.message})</span
					>
				{/if}
			</span>
		</div>
		<button
			type="button"
			onclick={() => {
				connectionStore.retry();
				onRetry?.();
			}}
			class="rounded border border-zinc-300 px-3 py-1 text-xs font-medium hover:bg-zinc-100 dark:border-zinc-700 dark:hover:bg-zinc-800"
		>
			{t("retry")}
		</button>
	</div>
{/if}
