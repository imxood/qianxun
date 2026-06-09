<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import { sessionStore } from '$lib/stores/session.svelte';
	import { subSessionStore } from '$lib/stores/sub_session.svelte';
	import { chatStore } from '$lib/stores/chat.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import ChatStream from './ChatStream.svelte';

	const view = $derived(uiStore.activeView);
	const active = $derived(sessionStore.active);
	const messages = $derived(sessionStore.activeMessages);
	const activeSub = $derived(subSessionStore.active);
	const subMessages = $derived(view.kind === 'sub_session' && activeSub ? activeSub.messages : []);
	const subMode = $derived<'task' | 'followup'>(activeSub && !subSessionStore.isActive(activeSub) ? 'followup' : 'task');

	let newInputEl: HTMLTextAreaElement | undefined = $state();
	let newInputValue = $state('');

	$effect(() => {
		if (view.kind === 'new' && newInputEl) {
			setTimeout(() => newInputEl?.focus(), 50);
		}
	});

	async function sendToActiveSession(text: string) {
		if (!active) return;
		await chatStore.send(active.id, text);
	}

	async function sendToActiveSubSession(text: string) {
		if (!activeSub) return;
		await chatStore.sendToSubSession(activeSub.id, text);
	}

	async function sendNew(text: string) {
		// 2026-06-09: 'new' view 发送第一条消息时, 调 chatStore.send(null, text),
		// chatStore.send 内部 lazy create session (后端生成真 ID) 后再 sendMessage.
		await chatStore.send(null, text);
	}
</script>

{#if view.kind === 'session' && active}
	<ChatStream
		messages={messages}
		onSend={sendToActiveSession}
		placeholder="输入消息开始... (Enter 发送 · Shift+Enter 换行)"
	/>
{:else if view.kind === 'sub_session' && activeSub}
	<ChatStream
		messages={subMessages}
		onSend={sendToActiveSubSession}
		mode={subMode}
	/>
{:else if view.kind === 'new'}
	<!-- 2026-06-09: 新建任务时显示空 ChatStream, 用户输入第一条消息触发 lazy create -->
	<ChatStream
		messages={[]}
		onSend={sendNew}
		placeholder="输入第一条消息开始... (Enter 发送 · Shift+Enter 换行)"
	/>
{:else}
	<!-- 空状态 (无 session) -->
	<div class="flex-1 flex flex-col items-center justify-center bg-zinc-50 dark:bg-zinc-950">
		<Icon name="message-square-dashed" class="w-12 h-12 text-zinc-300 dark:text-zinc-700 mb-2" />
		<p class="text-sm text-zinc-500">还没有会话</p>
		<p class="text-[11px] text-zinc-400 dark:text-zinc-600 mt-1">点 "新建任务" 开始</p>
	</div>
{/if}
