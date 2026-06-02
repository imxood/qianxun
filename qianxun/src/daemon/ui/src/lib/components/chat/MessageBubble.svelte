<script lang="ts">
	// Stage 9c — MessageBubble (Web Console 复制 Tauri)
	// 跟 qianxun-desktop/src/lib/components/chat/MessageBubble.svelte 一致
	// 区别: ContentBlock 类型用 webui 自己的 (lib/types/chat.ts), 路径改 $lib/types/chat

	import type { ContentBlock, MessageRole } from '$lib/types/chat';

	type Props = {
		block: ContentBlock;
		role: MessageRole;
	};

	let { block, role }: Props = $props();

	/// tool_result.content 是 string, 直接展示
	const toolResultText = $derived(
		block.type === 'tool_result' && typeof block.content === 'string'
			? block.content
			: ''
	);
</script>

<div
	class="my-2 rounded-lg p-3 {role === 'user'
		? 'ml-12 bg-blue-50 dark:bg-blue-950'
		: 'mr-12 bg-muted'}"
>
	{#if block.type === 'text'}
		<p class="whitespace-pre-wrap">{block.text ?? ''}</p>
	{:else if block.type === 'thinking'}
		<details class="text-xs opacity-70">
			<summary class="cursor-pointer select-none">思考过程</summary>
			<pre class="mt-1 whitespace-pre-wrap font-mono text-[0.7rem]">{block.text ?? ''}</pre>
		</details>
	{:else if block.type === 'tool_use'}
		<div class="bg-background rounded border p-2">
			<div class="font-mono text-xs opacity-60">工具调用: {block.name || '(unknown)'}</div>
			<code class="mt-1 block break-all text-sm">
				{JSON.stringify(block.input ?? {}, null, 2)}
			</code>
		</div>
	{:else if block.type === 'tool_result'}
		<div
			class="bg-background rounded border p-2 text-sm {block.is_error
				? 'border-red-300 dark:border-red-900'
				: ''}"
		>
			<div class="font-mono text-xs opacity-60">
				工具结果 {block.is_error ? '(错误)' : ''} · {block.elapsed_ms ?? 0}ms
			</div>
			<pre class="mt-1 whitespace-pre-wrap font-mono text-xs">{toolResultText}</pre>
		</div>
	{:else if block.type === 'image'}
		<div class="text-xs italic opacity-50">[image block — Stage 9d 实现]</div>
	{/if}
</div>
