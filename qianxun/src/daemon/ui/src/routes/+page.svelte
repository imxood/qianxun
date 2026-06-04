<script lang="ts">
	// 2026-06-05 fix: 干掉 `goto('/llm')` 自动跳转. 原因:
	// - Prod (build/ 静态): paths.base='/ui', SvelteKit client router 走 base-relative
	//   goto → /ui/llm. ✓
	// - Dev (vite dev server, base='/ui/'): SvelteKit 2.61 + paths.base='' 的边界 case,
	//   client router 启动时根据当前 URL '/ui' 算 base, 拼 goto('/llm') 解析成 '/ui/llm',
	//   但实际 SvelteKit 路由表里没 `/ui/llm` 路由 → 报 "Not found: /ui".
	// 简化为 welcome 页, 让用户自己点 sidebar 进具体页面 (跟 prod +error.svelte
	// 行为一致, 不会有 routing 错).
	import { ArrowRight } from '@lucide/svelte';
</script>

<div class="text-muted-foreground flex h-full flex-col items-center justify-center gap-3 p-12 text-sm">
	<p class="text-base font-medium">千寻 Daemon 控制台</p>
	<p class="flex items-center gap-2">
		从左侧菜单选择模块
		<ArrowRight class="h-3.5 w-3.5" />
		<a href="/llm" class="text-primary underline">LLM Providers</a>
	</p>
</div>
