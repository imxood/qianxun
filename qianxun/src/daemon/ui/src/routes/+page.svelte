<script lang="ts">
	import { goto } from '$app/navigation';
	import { onMount } from 'svelte';

	onMount(() => {
		// 2026-06-04 fix: SvelteKit 2 `goto('/llm')` 在 `paths.base='/ui'` 下
		// 解析成 `/llm` (host root), 不是 `/ui/llm` (base 下). 用 `/ui/llm` 显式
		// 走 base 路径, 跟 daemon `nest_service("/ui", ...)` 1:1 对齐.
		void goto('/ui/llm', { replaceState: true });
	});
</script>

<div class="text-muted-foreground flex items-center justify-center p-12 text-sm">
	跳转到 <a href="/ui/llm" class="ml-1 underline">/llm</a> …
</div>
