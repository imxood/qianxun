<script lang="ts">
	import type { ContentBlock } from "$lib/types/ipc";

	type Props = {
		block: ContentBlock;
		role: "user" | "assistant";
	};

	let { block, role }: Props = $props();

	/// tool_result.content 是 string | ContentBlock[], 渲染时统一成 string
	const toolResultText = $derived(
		typeof block.content === "string"
			? block.content
			: JSON.stringify(block.content, null, 2)
	);
</script>

<div
	class="my-2 rounded-lg p-3 {role === 'user'
		? 'ml-12 bg-blue-50 dark:bg-blue-950'
		: 'mr-12 bg-muted'}"
>
	{#if block.type === "text"}
		<p class="whitespace-pre-wrap">{block.text ?? ""}</p>
	{:else if block.type === "thinking"}
		<details class="text-xs opacity-70">
			<summary class="cursor-pointer select-none">思考过程</summary>
			<pre class="mt-1 whitespace-pre-wrap font-mono text-[0.7rem]">{block.text ?? ""}</pre>
		</details>
	{:else if block.type === "tool_use"}
		<div class="rounded border bg-background p-2">
			<div class="text-xs font-mono opacity-60">工具调用: {block.name ?? "(unknown)"}</div>
			<code class="mt-1 block text-sm break-all">
				{JSON.stringify(block.input ?? {}, null, 2)}
			</code>
		</div>
	{:else if block.type === "tool_result"}
		<div
			class="rounded border bg-background p-2 text-sm {block.is_error
				? 'border-red-300 dark:border-red-900'
				: ''}"
		>
			<div class="text-xs font-mono opacity-60">
				工具结果 {block.is_error ? "(错误)" : ""} · {block.elapsed_ms ?? 0}ms
			</div>
			<pre class="mt-1 whitespace-pre-wrap font-mono text-xs">{toolResultText}</pre>
		</div>
	{:else if block.type === "image"}
		<div class="text-xs italic opacity-50">[image block — Stage 4 实现]</div>
	{/if}
</div>
