<script lang="ts">
	import { sessionStore } from "$lib/stores/session.svelte";
	import Send from "@lucide/svelte/icons/send";
	import Square from "@lucide/svelte/icons/square";

	type Props = {
		model?: string;
	};

	let { model = "MiniMax-M3" }: Props = $props();

	let text = $state("");
	let textareaEl = $state<HTMLTextAreaElement | null>(null);

	async function submit(): Promise<void> {
		const msg = text.trim();
		if (!msg) return;
		if (sessionStore.isStreaming) return;
		text = "";
		await sessionStore.send(msg, model);
		// 重新聚焦输入框
		textareaEl?.focus();
	}

	function cancel(): void {
		sessionStore.cancel();
	}

	function onKeydown(e: KeyboardEvent): void {
		// Enter 发送, Shift+Enter 换行
		if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
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
		placeholder="输入消息, Enter 发送, Shift+Enter 换行"
		disabled={sessionStore.isStreaming}
		rows="2"
		class="flex-1 min-h-12 max-h-48 resize-none rounded border bg-background p-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-50"
	></textarea>
	{#if sessionStore.isStreaming}
		<button
			type="button"
			onclick={cancel}
			class="flex items-center gap-1 rounded border px-3 py-2 text-sm hover:bg-muted"
			title="停止生成"
		>
			<Square class="size-3.5" />
			停止
		</button>
	{:else}
		<button
			type="button"
			onclick={submit}
			disabled={!text.trim()}
			class="flex items-center gap-1 rounded bg-primary px-4 py-2 text-sm font-medium text-primary-foreground disabled:opacity-50"
		>
			<Send class="size-3.5" />
			发送
		</button>
	{/if}
</div>
