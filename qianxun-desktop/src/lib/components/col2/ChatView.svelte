<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import { sessionStore } from '$lib/stores/session.svelte';
	import { subSessionStore } from '$lib/stores/sub_session.svelte';
	import { chatStore } from '$lib/stores/chat.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import ChatStream from './ChatStream.svelte';
	import ChatNewView from './ChatNewView.svelte';

	const view = $derived(uiStore.activeView);
	const active = $derived(sessionStore.active);
	const messages = $derived(sessionStore.activeMessages);
	const activeSub = $derived(subSessionStore.active);
	const subMessages = $derived(view.kind === 'sub_session' && activeSub ? activeSub.messages : []);
	const subMode = $derived<'task' | 'followup'>(activeSub && !subSessionStore.isActive(activeSub) ? 'followup' : 'task');

	async function sendToActiveSession(text: string) {
		if (!active) return;
		await chatStore.send(active.id, text);
	}

	async function sendToActiveSubSession(text: string) {
		if (!activeSub) return;
		await chatStore.sendToSubSession(activeSub.id, text);
	}

	// 2026-06-12 (Phase D.8): 重试按钮回调 — 调 chatStore.resend 重发最近 user 消息.
	async function retryActive() {
		if (view.kind === 'session' && active) {
			await chatStore.resend(active.id);
		}
		// sub_session 流也类似, 但 v0.4 再加 (目前 subSessionStore 没 lastUserMessage 跟踪)
	}
</script>

{#if view.kind === 'session' && active}
	<ChatStream
		messages={messages}
		onSend={sendToActiveSession}
		onRetry={retryActive}
		placeholder="输入消息开始... (Enter 发送 · Shift+Enter 换行)"
	/>
{:else if view.kind === 'sub_session' && activeSub}
	<ChatStream
		messages={subMessages}
		onSend={sendToActiveSubSession}
		mode={subMode}
	/>
{:else if view.kind === 'new'}
	<!-- 2026-06-09: 居中卡片式新对话入口, 底部 1 个项目下拉 (用户截图设计) -->
	<ChatNewView />
{:else}
	<!-- 空状态 (无 session) -->
	<div class="flex-1 flex flex-col items-center justify-center bg-zinc-50 dark:bg-zinc-950">
		<Icon name="message-square-dashed" class="w-12 h-12 text-zinc-300 dark:text-zinc-700 mb-2" />
		<p class="text-sm text-zinc-500">还没有会话</p>
		<p class="text-[11px] text-zinc-400 dark:text-zinc-600 mt-1">点 "新建任务" 开始</p>
	</div>
{/if}
