<script lang="ts">
	// Stage 9c — ThreeColumnLayout (Web Console 复制 Tauri)
	// 跟 qianxun-desktop/src/lib/components/layout/ThreeColumnLayout.svelte 对齐
	// 区别: webui 调整布局 — children 走中间 (主聊天区), aside 走右边 (元数据/调试)
	//       Tauri 是 sidebar (左) + sessions (中) + children (右), 因为 Tauri 有
	//       独立的 Sidebar 组件 (含 team/project), webui 把这些都合并到 left 栏.

	type Props = {
		/// 中间 (320px) — 主聊天内容
		children?: import('svelte').Snippet;
		/// 左边 (240px) — 项目/provider/session 列表
		sidebar?: import('svelte').Snippet;
		/// 右边 (1fr) — 元数据 / 调试
		aside?: import('svelte').Snippet;
	};

	let { children, sidebar, aside }: Props = $props();
</script>

<div
	class="bg-background text-foreground grid h-screen"
	style="grid-template-columns: 240px 1fr 320px;"
>
	<aside class="overflow-y-auto border-r border-border p-3">
		{@render sidebar?.()}
	</aside>
	<main class="flex min-w-0 flex-col overflow-hidden">
		{@render children?.()}
	</main>
	<aside class="overflow-y-auto border-l border-border p-3">
		{@render aside?.()}
	</aside>
</div>
