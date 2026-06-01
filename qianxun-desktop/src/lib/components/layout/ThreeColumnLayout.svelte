<script lang="ts">
	import { connectionStore } from "$lib/stores/connection.svelte";
	import { onMount, onDestroy } from "svelte";

	let { children, sidebar, sessions }: {
		children?: import("svelte").Snippet;
		sidebar?: import("svelte").Snippet;
		sessions?: import("svelte").Snippet;
	} = $props();

	// 必须用 onMount 而非 $effect: startHealthCheck() 内部写 $state (daemonState/attempt),
	// 会进入 Svelte 5 的 read-write effect cycle, 触发 effect_update_depth_exceeded
	// bailout, 整页响应式废掉 (点击项目/会话不更新 UI).
	// onMount 在 setup 阶段执行, 不进入 reactive tracking 链.
	// setInterval 改由 onDestroy 清理, 不再被 effect 取消.
	onMount(() => {
		connectionStore.startHealthCheck();
	});

	onDestroy(() => {
		connectionStore.stopHealthCheck();
	});

	// 状态点颜色 (4 态)
	const stateColor: Record<string, string> = {
		connected: "bg-green-500",
		reconnecting: "bg-yellow-500 animate-pulse",
		degraded: "bg-red-500",
		offline: "bg-zinc-500",
	};
</script>

<div class="grid h-screen grid-cols-[200px_280px_1fr] bg-background text-foreground">
	<aside class="overflow-y-auto border-r border-border p-3">
		<div class="mb-3 flex items-center gap-2">
			<span class="size-2 rounded-full {stateColor[connectionStore.daemonState] ?? 'bg-zinc-500'}"></span>
			<span class="text-xs font-medium uppercase tracking-wide text-muted-foreground">
				{connectionStore.daemonState}
			</span>
		</div>
		{@render sidebar?.()}
	</aside>
	<aside class="overflow-y-auto border-r border-border p-3">
		{@render sessions?.()}
	</aside>
	<main class="overflow-y-auto p-3">
		{@render children?.()}
	</main>
</div>

<div class="pointer-events-none fixed top-2 right-3 select-none text-xs text-muted-foreground">
	<span class="opacity-60">Daemon:</span>
	<span class="font-mono font-bold {connectionStore.isDegraded ? 'text-red-500' : 'text-green-600'}">
		{connectionStore.daemonState}
	</span>
	{#if connectionStore.lastError}
		<span class="ml-2 opacity-60" title={connectionStore.lastError.message}>
			{connectionStore.lastErrorDisplay}
		</span>
	{/if}
</div>
