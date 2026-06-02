<script lang="ts">
	// Stage 9c — /chat 路由
	// 跟 docs/30_子项目规划/01b-daemon-web-console.md §10 Chat 视图 一致
	// 3 栏布局:
	//   左 (240px) — Provider + Session 列表 (active provider + session list + 新建按钮)
	//   中 (1fr)   — 主聊天区: session 头部 + messages + InputBox + ConnectionBanner
	//   右 (320px) — 元数据/调试: token usage + cancel + lastError + active session info

	import { onMount } from 'svelte';
	import { Plus, Cpu, Trash2, Ban, RefreshCw, Zap } from '@lucide/svelte';
	import ThreeColumnLayout from '$lib/components/layout/ThreeColumnLayout.svelte';
	import MessageBubble from '$lib/components/chat/MessageBubble.svelte';
	import InputBox from '$lib/components/chat/InputBox.svelte';
	import ConnectionBanner from '$lib/components/chat/ConnectionBanner.svelte';
	import { chatStore } from '$lib/stores/chat.svelte';
	import { listProviders } from '$lib/api/llm';
	import type { LlmProviderSummary } from '$lib/types/api';

	let providers = $state<LlmProviderSummary[]>([]);
	let providerError = $state<string | null>(null);

	async function loadProviders() {
		providerError = null;
		try {
			providers = await listProviders();
		} catch (e) {
			providerError = e instanceof Error ? e.message : '加载 providers 失败';
		}
	}

	const activeProvider = $derived(providers.find((p) => p.active) ?? null);

	async function refresh() {
		await Promise.all([chatStore.loadSessions(), loadProviders()]);
	}

	async function newSession() {
		await chatStore.createNewSession();
	}

	async function deleteActive() {
		if (!chatStore.activeSessionId) return;
		if (!confirm(`确认删除 session "${chatStore.activeSessionId}" ?`)) return;
		await chatStore.deleteSession(chatStore.activeSessionId);
	}

	function cancelPrompt() {
		chatStore.cancel();
	}

	// 反应式: active session 变化时滚动到底部
	let messagesEl = $state<HTMLDivElement | null>(null);
	$effect(() => {
		// 订阅 messages 长度变化 (svelte 5 $effect 自动跟踪)
		if (messagesEl && chatStore.messages.length) {
			setTimeout(() => {
				messagesEl?.scrollTo({ top: messagesEl.scrollHeight, behavior: 'smooth' });
			}, 0);
		}
	});

	onMount(() => {
		void refresh();
	});
</script>

<ThreeColumnLayout>
	{#snippet sidebar()}
		<section class="flex flex-col gap-3">
			<header class="flex items-center justify-between">
				<h2 class="text-sm font-semibold">Provider</h2>
				<button
					type="button"
					class="text-muted-foreground hover:text-foreground"
					onclick={refresh}
					title="刷新"
					aria-label="刷新"
					data-testid="chat-refresh"
				>
					<RefreshCw class="size-3.5" />
				</button>
			</header>
			<div class="text-xs" data-testid="chat-active-provider">
				{#if providerError}
					<span class="text-destructive">{providerError}</span>
				{:else if activeProvider}
					<div class="flex items-center gap-1.5">
						<Cpu class="text-muted-foreground size-3" />
						<span class="font-mono">{activeProvider.id}</span>
					</div>
					<div class="text-muted-foreground mt-0.5 font-mono text-[10px]">
						{activeProvider.provider} · {activeProvider.model}
					</div>
				{:else}
					<span class="text-muted-foreground">无 active provider</span>
				{/if}
			</div>

			<div class="border-border border-t pt-3">
				<div class="mb-2 flex items-center justify-between">
					<h3 class="text-sm font-semibold">Sessions</h3>
					<button
						type="button"
						class="bg-primary text-primary-foreground flex items-center gap-1 rounded px-2 py-1 text-xs"
						onclick={newSession}
						data-testid="chat-new-session"
					>
						<Plus class="size-3" />
						新建
					</button>
				</div>
				{#if chatStore.loadError}
					<div class="text-destructive text-xs">{chatStore.loadError}</div>
				{:else if chatStore.loading && chatStore.sessions.length === 0}
					<div class="text-muted-foreground text-xs">加载中…</div>
				{:else if chatStore.sessions.length === 0}
					<div class="text-muted-foreground text-xs">暂无 session</div>
				{:else}
					<div class="flex flex-col gap-1">
						{#each chatStore.sessions as s (s.id)}
							<button
								type="button"
								class="hover:bg-accent rounded p-1.5 text-left text-xs"
								class:bg-accent={s.id === chatStore.activeSessionId}
								onclick={() => chatStore.selectSession(s.id)}
								data-testid={`chat-session-${s.id}`}
							>
								<div class="truncate font-mono">{s.id.slice(0, 16)}…</div>
								<div class="text-muted-foreground mt-0.5 flex gap-1 text-[10px]">
									<span>{s.model}</span>
									<span>·</span>
									<span>{s.message_count} 条</span>
									<span>·</span>
									<span class="capitalize">{s.status}</span>
								</div>
							</button>
						{/each}
					</div>
				{/if}
			</div>
		</section>
	{/snippet}

	<ConnectionBanner onRetry={refresh} />

	{#if !chatStore.activeSessionId}
		<div
			class="text-muted-foreground flex h-full items-center justify-center"
			data-testid="chat-empty"
		>
			<div class="text-center">
				<h1 class="text-2xl font-bold">千寻 Chat</h1>
				<p class="mt-2 text-sm">从左侧选择 session 或新建一个开始对话</p>
			</div>
		</div>
	{:else}
		<section class="flex h-full min-h-0 flex-col" data-testid="chat-active-pane">
			<header class="border-border border-b pb-3">
				<div class="flex items-center justify-between gap-2">
					<h1 class="truncate font-mono text-base font-semibold" data-testid="chat-session-id">
						{chatStore.activeSessionId}
					</h1>
					<div class="flex items-center gap-1">
						<button
							type="button"
							class="hover:bg-muted text-muted-foreground flex items-center gap-1 rounded px-2 py-1 text-xs"
							onclick={deleteActive}
							title="删除 session"
							data-testid="chat-delete-session"
						>
							<Trash2 class="size-3" />
							删除
						</button>
					</div>
				</div>
			</header>

			<div
				bind:this={messagesEl}
				class="flex-1 overflow-y-auto py-4"
				data-testid="chat-messages"
			>
				<div class="mx-auto max-w-2xl">
					{#if chatStore.messages.length === 0}
						<div class="text-muted-foreground py-12 text-center text-sm">
							输入消息开始对话
						</div>
					{/if}
					{#each chatStore.messages as msg (msg.id)}
						{#each msg.content as block, i (i)}
							<MessageBubble {block} role={msg.role} />
						{/each}
						{#if msg.error}
							<div
								class="mt-1 rounded border border-red-300 bg-red-50 p-2 text-xs text-red-700 dark:border-red-900 dark:bg-red-950 dark:text-red-300"
								data-testid="chat-msg-error"
							>
								{msg.error}
							</div>
						{/if}
					{/each}
				</div>
			</div>

			<footer class="border-border border-t pt-2">
				<InputBox />
			</footer>
		</section>
	{/if}

	{#snippet aside()}
		<section class="flex flex-col gap-3 text-xs" data-testid="chat-aside">
			<div>
				<h3 class="text-muted-foreground mb-1 text-[10px] font-semibold uppercase tracking-wider">
					Session
				</h3>
				{#if chatStore.activeSessionId}
					<div class="bg-muted rounded p-2 font-mono text-[10px] break-all">
						{chatStore.activeSessionId}
					</div>
				{:else}
					<div class="text-muted-foreground">未选中</div>
				{/if}
			</div>

			<div>
				<h3 class="text-muted-foreground mb-1 text-[10px] font-semibold uppercase tracking-wider">
					Model
				</h3>
				<div class="flex items-center gap-1 font-mono">
					<Cpu class="text-muted-foreground size-3" />
					{chatStore.model}
				</div>
				{#if activeProvider}
					<div class="text-muted-foreground mt-0.5 text-[10px]">
						via {activeProvider.id} ({activeProvider.provider})
					</div>
				{/if}
			</div>

			<div>
				<h3 class="text-muted-foreground mb-1 text-[10px] font-semibold uppercase tracking-wider">
					Token usage
				</h3>
				{#if chatStore.usage$}
					<div class="space-y-0.5 font-mono" data-testid="chat-token-usage">
						<div>in: <span class="font-semibold">{chatStore.usage$.input}</span></div>
						<div>out: <span class="font-semibold">{chatStore.usage$.output}</span></div>
						{#if chatStore.stopReason}
							<div class="text-muted-foreground">stop: {chatStore.stopReason}</div>
						{/if}
					</div>
				{:else}
					<div class="text-muted-foreground">暂无</div>
				{/if}
			</div>

			<div>
				<h3 class="text-muted-foreground mb-1 text-[10px] font-semibold uppercase tracking-wider">
					Actions
				</h3>
				<button
					type="button"
					class="hover:bg-muted flex w-full items-center gap-1.5 rounded border px-2 py-1.5 text-left"
					onclick={cancelPrompt}
					disabled={!chatStore.isStreaming}
					title="取消正在跑的 prompt"
					data-testid="chat-cancel-prompt"
				>
					<Ban class="size-3" />
					取消生成
				</button>
				<button
					type="button"
					class="hover:bg-muted mt-1 flex w-full items-center gap-1.5 rounded border px-2 py-1.5 text-left"
					onclick={refresh}
					title="刷新 sessions + providers"
				>
					<Zap class="size-3" />
					刷新
				</button>
			</div>

			{#if chatStore.lastError}
				<div data-testid="chat-last-error">
					<h3 class="text-destructive mb-1 text-[10px] font-semibold uppercase tracking-wider">
						Last error
					</h3>
					<div
						class="rounded border border-red-300 bg-red-50 p-2 font-mono text-[10px] text-red-700 break-all dark:border-red-900 dark:bg-red-950 dark:text-red-300"
					>
						{chatStore.lastError}
					</div>
				</div>
			{/if}

			<div class="text-muted-foreground mt-auto text-[10px]">
				<div>Web Console · Stage 9c</div>
				<div class="mt-0.5">SSE 12-event parser ready</div>
			</div>
		</section>
	{/snippet}
</ThreeColumnLayout>
