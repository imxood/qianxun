<script lang="ts">
	import type { Session } from "$lib/types/ipc";
	import { sessionStore } from "$lib/stores/session.svelte";
	import { connectionStore } from "$lib/stores/connection.svelte";
	import MessageBubble from "$lib/components/chat/MessageBubble.svelte";
	import InputBox from "$lib/components/chat/InputBox.svelte";
	import Cpu from "@lucide/svelte/icons/cpu";
	import WifiOff from "@lucide/svelte/icons/wifi-off";

	type Props = {
		activeSession: Session | null;
	};

	let { activeSession }: Props = $props();

	/// active session 的 model 优先; 否则用 sessionStore 当前 model; 都没有则用兜底.
	const model = $derived(activeSession?.model ?? sessionStore.runtime?.model ?? "MiniMax-M3");
</script>

<section class="flex h-full flex-col">
	{#if activeSession}
		<header class="border-b border-border pb-3">
			<h1 class="text-xl font-bold">{activeSession.title}</h1>
			<div class="mt-1 flex items-center gap-3 text-xs text-muted-foreground">
				<span class="flex items-center gap-1">
					<Cpu class="size-3" />
					{model}
				</span>
				<span>·</span>
				<span>消息 {activeSession.message_count} 条</span>
				{#if sessionStore.runtime}
					<span>·</span>
					<span class="font-mono text-[0.65rem] opacity-60">
						{sessionStore.runtime.sessionId.slice(0, 8)}
					</span>
				{/if}
			</div>
		</header>
	{/if}

	<div class="flex-1 overflow-y-auto py-4">
		{#if !sessionStore.runtime}
			<div class="m-auto max-w-md text-center">
				<h1 class="text-2xl font-bold">千寻</h1>
				<p class="mt-2 text-muted-foreground">从左侧选择一个会话开始对话.</p>
				<p class="mt-4 text-xs text-muted-foreground/70">
					Stage 3 — SSE 流式消费 + 12 事件处理已就绪
				</p>
			</div>
		{:else}
			<div class="mx-auto max-w-2xl">
				{#each sessionStore.messages as block, i (i)}
					<MessageBubble
						{block}
						role={i % 2 === 0 ? "user" : "assistant"}
					/>
				{/each}

				{#if sessionStore.isStreaming}
					<div class="my-2 mr-12 flex items-center gap-1 text-xs text-muted-foreground">
						<span class="inline-block size-1.5 animate-pulse rounded-full bg-yellow-500"></span>
						<span>正在生成...</span>
					</div>
				{/if}

				{#if sessionStore.runtime.lastError}
					<div
						class="mt-2 rounded border border-red-300 bg-red-50 p-2 text-xs text-red-700 dark:border-red-900 dark:bg-red-950 dark:text-red-300"
					>
						错误: {sessionStore.runtime.lastError}
					</div>
				{/if}

				{#if sessionStore.runtime.usage}
					<div class="mt-2 text-right text-[0.65rem] text-muted-foreground/60">
						tokens: {sessionStore.runtime.usage.input} in / {sessionStore.runtime.usage.output} out
						{#if sessionStore.runtime.stopReason}
							· stop: {sessionStore.runtime.stopReason}
						{/if}
					</div>
				{/if}
			</div>
		{/if}
	</div>

	<footer class="border-t border-border pt-3">
		<InputBox {model} />
		{#if connectionStore.isDegraded}
			<div class="mt-2 flex items-center gap-1.5 text-xs text-red-500">
				<WifiOff class="size-3" />
				<span>Daemon 未连接 — 重连后可继续输入</span>
			</div>
		{/if}
	</footer>
</section>
