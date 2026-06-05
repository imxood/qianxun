<script lang="ts">
	// Stage 9c +error.svelte — SvelteKit 标准错误页
	// 显示: 错误标题 + 错误信息 (折叠) + "回到首页" + "刷新" 按钮
	// 简化: 不在 dev 暴露 stack trace (生产隐藏)
	import { page } from '$app/state';
	import { base } from '$app/paths';
	import { AlertTriangle, Home, RefreshCw } from '@lucide/svelte';

	const status = $derived(page.status);
	const message = $derived(page.error?.message ?? '未知错误');
	const isDev = $derived(import.meta.env.DEV);
	const errorStack = $derived(
		page.error && typeof page.error === 'object' && 'stack' in page.error
			? (page.error as { stack?: string }).stack
			: undefined
	);
</script>

<div
	class="bg-background flex h-screen w-screen items-center justify-center p-4 sm:p-6"
	data-testid="error-page"
	data-status={status}
>
	<div
		class="border-border bg-card text-card-foreground w-full max-w-md rounded-lg border p-6 shadow-sm"
	>
		<div class="flex items-start gap-3">
			<div
				class="bg-destructive/10 text-destructive flex h-10 w-10 shrink-0 items-center justify-center rounded-full"
			>
				<AlertTriangle class="h-5 w-5" />
			</div>
			<div class="min-w-0 flex-1">
				<h1 class="text-lg font-semibold">出错了</h1>
				<p class="text-muted-foreground mt-1 text-sm">
					{status === 404 ? '页面不存在' : status === 500 ? '服务器错误' : '加载页面时出现问题'}
				</p>
			</div>
		</div>

		<details
			class="mt-4 rounded-md border border-dashed border-border p-3 text-xs"
			data-testid="error-details"
		>
			<summary class="text-muted-foreground cursor-pointer">错误详情</summary>
			<pre class="mt-2 whitespace-pre-wrap break-all">{message}</pre>
			{#if isDev && errorStack}
				<pre class="text-muted-foreground mt-2 whitespace-pre-wrap break-all text-[10px]">{errorStack}</pre>
			{/if}
		</details>

		<div class="mt-4 flex flex-col gap-2 sm:flex-row">
			<a
				href="{base}/llm"
				class="border-input bg-background hover:bg-accent inline-flex flex-1 items-center justify-center gap-2 rounded-md border px-3 py-2 text-sm"
				data-testid="error-home-button"
			>
				<Home class="h-3.5 w-3.5" />
				回到首页
			</a>
			<button
				type="button"
				class="bg-primary text-primary-foreground hover:bg-primary/90 inline-flex flex-1 items-center justify-center gap-2 rounded-md px-3 py-2 text-sm"
				onclick={() => location.reload()}
				data-testid="error-reload-button"
			>
				<RefreshCw class="h-3.5 w-3.5" />
				刷新
			</button>
		</div>
	</div>
</div>
