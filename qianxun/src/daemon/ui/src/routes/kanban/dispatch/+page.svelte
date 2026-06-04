<script lang="ts">
	// /kanban/dispatch — 手动派发 (2026-06-04 阶段 3)
	import { onMount } from 'svelte';
	import { Zap, ArrowLeft } from '@lucide/svelte';
	import { dispatchNow, listBoards } from '$lib/api/kanban';
	import { authStore } from '$lib/stores/auth.svelte';
	import type { Board } from '$lib/types/kanban';

	let prompt = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);
	let result = $state<{ dispatched: boolean; task_id?: string; run_id?: string; profile_name?: string; reason?: string } | null>(null);
	let boards = $state<Board[]>([]);

	async function refresh() {
		try {
			boards = await listBoards();
		} catch (e) {
			// 静默
		}
	}

	async function go() {
		if (!prompt.trim()) return;
		busy = true;
		error = null;
		result = null;
		try {
			result = await dispatchNow(prompt);
		} catch (e) {
			error = e instanceof Error ? e.message : '派发失败';
		} finally {
			busy = false;
		}
	}

	onMount(refresh);

	// 2026-06-04 fix: 登录后自动重 fetch (见 llm/+page.svelte 注释)
	let firstRun = true;
	$effect(() => {
		const token = authStore.token;
		if (firstRun) {
			firstRun = false;
			return;
		}
		if (token) {
			void refresh();
		}
	});
</script>

<svelte:head>
	<title>派发 · 千寻 Kanban</title>
</svelte:head>

<div class="mx-auto flex max-w-2xl flex-col gap-4 p-6">
	<header class="flex items-center gap-2">
		<a href="/kanban" class="hover:bg-accent rounded p-1" data-testid="back-link">
			<ArrowLeft class="size-4" />
		</a>
		<h1 class="text-lg font-semibold">手动派发 (Dispatch)</h1>
	</header>

	<div class="text-muted-foreground text-xs">
		输入需求描述, Kanban dispatcher 会拾起 ready task 派给 worker profile. 当前 boards: {boards.length}.
	</div>

	<textarea
		bind:value={prompt}
		placeholder="例如: 调研 Rust 2025 生态最新动态, 列出 5 大主题和代表项目"
		class="min-h-[200px] rounded border bg-card p-3 text-sm"
		disabled={busy}
		data-testid="dispatch-prompt"
	></textarea>

	<button
		type="button"
		class="bg-primary text-primary-foreground hover:bg-primary/90 inline-flex items-center gap-2 self-start rounded px-4 py-2 text-sm disabled:opacity-50"
		onclick={go}
		disabled={busy || !prompt.trim()}
		data-testid="dispatch-submit"
	>
		<Zap class="size-4" />
		{busy ? '派发中...' : '派发到 Kanban'}
	</button>

	{#if error}
		<div class="rounded border border-red-300 bg-red-50 p-3 text-xs text-red-700" data-testid="error">
			{error}
		</div>
	{/if}

	{#if result}
		<div
			class="rounded border p-3 text-xs {result.dispatched ? 'border-emerald-300 bg-emerald-50 dark:bg-emerald-950' : 'border-amber-300 bg-amber-50 dark:bg-amber-950'}"
			data-testid="dispatch-result"
		>
			{#if result.dispatched}
				<div class="font-semibold text-emerald-700 dark:text-emerald-300">✅ 派发成功</div>
				<div class="mt-1 font-mono text-[10px]">
					task_id: {result.task_id}<br />
					run_id: {result.run_id}<br />
					profile: {result.profile_name}
				</div>
				<a href="/kanban" class="text-primary mt-2 inline-block underline">→ 跳到 Kanban</a>
			{:else}
				<div class="font-semibold text-amber-700 dark:text-amber-300">⚠️ 未派发</div>
				<div class="mt-1">{result.reason}</div>
			{/if}
		</div>
	{/if}
</div>
