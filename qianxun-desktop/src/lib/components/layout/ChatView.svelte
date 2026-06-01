<script lang="ts">
	import type { Session } from "$lib/types/ipc";
	import { Button } from "$lib/components/ui/button/index.js";
	import * as Card from "$lib/components/ui/card/index.js";
	import Send from "@lucide/svelte/icons/send";
	import Cpu from "@lucide/svelte/icons/cpu";
	import WifiOff from "@lucide/svelte/icons/wifi-off";
	import { connectionStore } from "$lib/stores/connection.svelte";

	type Props = {
		activeSession: Session | null;
	};

	let { activeSession }: Props = $props();

	let inputText = $state("");
</script>

<section class="flex h-full flex-col">
	{#if activeSession}
		<header class="border-b border-border pb-3">
			<h1 class="text-xl font-bold">{activeSession.title}</h1>
			<div class="mt-1 flex items-center gap-3 text-xs text-muted-foreground">
				<span class="flex items-center gap-1">
					<Cpu class="size-3" />
					{activeSession.model}
				</span>
				<span>·</span>
				<span>消息 {activeSession.message_count} 条</span>
			</div>
		</header>

		<div class="flex-1 overflow-y-auto py-4">
			<Card.Root class="mx-auto max-w-2xl">
				<Card.Header>
					<Card.Title>千寻</Card.Title>
					<Card.Description>占位: Stage 2 接入 SSE 流式输出后会替换为真实消息渲染.</Card.Description>
				</Card.Header>
				<Card.Content class="text-sm text-muted-foreground">
					<p>这是一条 mock 消息, 用于展示三栏布局与消息容器.</p>
					<p class="mt-2">用户消息、思考块、工具调用卡片将在 Stage 2 通过 Svelte 5 runes + SSE 客户端实现.</p>
				</Card.Content>
			</Card.Root>
		</div>

		<footer class="border-t border-border pt-3">
			<div class="flex items-end gap-2">
				<textarea
					bind:value={inputText}
					placeholder="输入消息, Enter 发送, Shift+Enter 换行 (Stage 2 启用)"
					rows="2"
					class="flex-1 resize-none rounded-md border border-input bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-50"
					disabled
				></textarea>
				<Button disabled title="Stage 2 启用">
					<Send class="size-3.5" />
					发送
				</Button>
			</div>
			{#if connectionStore.isDegraded}
				<div class="mt-2 flex items-center gap-1.5 text-xs text-red-500">
					<WifiOff class="size-3" />
					<span>Daemon 未连接 — 输入框将在重连后启用 (Stage 2 实现)</span>
				</div>
			{/if}
		</footer>
	{:else}
		<div class="m-auto max-w-md text-center">
			<h1 class="text-2xl font-bold">千寻 TUI</h1>
			<p class="mt-2 text-muted-foreground">从左侧选择一个会话开始对话.</p>
			<p class="mt-4 text-xs text-muted-foreground/70">Stage 1 脚手架 · 不接 Tauri / 真实 Daemon</p>
		</div>
	{/if}
</section>
