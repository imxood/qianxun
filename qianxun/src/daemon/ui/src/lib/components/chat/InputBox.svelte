<script lang="ts">
	// Stage 9c — InputBox (Web Console 复制 Tauri)
	// 跟 qianxun-desktop/src/lib/components/chat/InputBox.svelte 一致
	// 区别: 用 chatStore (webui) 代替 sessionStore (Tauri), 无 offlineQueue

	import { chatStore } from '$lib/stores/chat.svelte';
	import Send from '@lucide/svelte/icons/send';
	import Square from '@lucide/svelte/icons/square';

	let text = $state('');
	let textareaEl = $state<HTMLTextAreaElement | null>(null);

	async function submit(): Promise<void> {
		const msg = text.trim();
		if (!msg) return;
		if (chatStore.isStreaming) return;
		text = '';
		await chatStore.sendPrompt(msg);
		textareaEl?.focus();
	}

	function cancel(): void {
		chatStore.cancel();
	}

	function onKeydown(e: KeyboardEvent): void {
		// Enter 发送, Shift+Enter 换行
		if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
			e.preventDefault();
			void submit();
		}
	}
</script>

<div class="flex gap-2 border-t p-3">
	<textarea
		bind:this={textareaEl}
		bind:value={text}
		onkeydown={onKeydown}
		placeholder={'输入消息，回车发送（Shift+Enter 换行）'}
		disabled={chatStore.isStreaming}
		rows="2"
		data-testid="chat-input"
		class="bg-background flex-1 resize-none rounded border p-2 text-sm outline-none min-h-12 max-h-48 focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-50"
	></textarea>
	{#if chatStore.isStreaming}
		<button
			type="button"
			onclick={cancel}
			class="hover:bg-muted flex items-center gap-1 rounded border px-3 py-2 text-sm"
			title="停止生成"
			data-testid="chat-cancel-btn"
		>
			<Square class="size-3.5" />
			停止
		</button>
	{:else}
		<button
			type="button"
			onclick={submit}
			disabled={!text.trim()}
			class="bg-primary text-primary-foreground flex items-center gap-1 rounded px-4 py-2 text-sm font-medium disabled:opacity-50"
			data-testid="chat-send-btn"
		>
			<Send class="size-3.5" />
			发送
		</button>
	{/if}
</div>
