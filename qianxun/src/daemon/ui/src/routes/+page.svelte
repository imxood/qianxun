<script lang="ts">
	// 2026-06-05 fix v2: 加回 `paths.base='/ui'`, 干掉 `goto('/llm')` 跳转.
	// 原因:
	// - paths.base='/ui' 时 `<a href="/llm">` 是绝对路径, SvelteKit 2 不会自动
	//   拼 base, 浏览器跳到 /llm (没 /ui prefix) → daemon 23900/llm 没匹配 → 404.
	// - 必须用 `{base}/llm` (import `base` from `$app/paths`) 让链接跟着 base 走.
	import { ArrowRight } from '@lucide/svelte';
	import { base } from '$app/paths';
</script>

<div class="text-muted-foreground flex h-full flex-col items-center justify-center gap-3 p-12 text-sm">
	<p class="text-base font-medium">千寻 Daemon 控制台</p>
	<p class="flex items-center gap-2">
		从左侧菜单选择模块
		<ArrowRight class="h-3.5 w-3.5" />
		<a href="{base}/llm" class="text-primary underline">LLM Providers</a>
	</p>
</div>
