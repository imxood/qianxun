import tailwindcss from '@tailwindcss/vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [tailwindcss(), sveltekit()],
	// 2026-06-09 启动优化: 显式预打包 Svelte 核心 + 我们最重的依赖,
	// 避免 dev 模式首屏 5-10s 按需编译等待. Svelte 5 + SvelteKit 2 默认行为已较优,
	// 但显式列出可跳过 .vite/deps 懒发现.
	optimizeDeps: {
		include: ['svelte', 'svelte/store', '@lucide/svelte', '@tauri-apps/api/core', '@tauri-apps/api/event'],
	},
	server: {
		// 2026-06-09 启动优化: dev 模式启动后立刻预热主入口编译,
		// webview 第一次请求时 (来自 Tauri) Vite 已编译完, 立即推 HTML.
		// 二次启动效果好, 首次启动也能让 webview 看到 Vite 而不是 7s 编译.
		warmup: {
			clientFiles: ['src/routes/+page.svelte', 'src/app.html'],
		},
	},
});
