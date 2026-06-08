// vitest 配置 — Svelte 5 runes 感知
// 与 docs/30_子项目规划/03-tauri-desktop.md §12 一致 (Stage 4 仅 P0 单测)
import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
	plugins: [svelte()],
	resolve: {
		alias: {
			$lib: fileURLToPath(new URL("./src/lib", import.meta.url)),
			"$lib/*": fileURLToPath(new URL("./src/lib/*", import.meta.url)),
			// SvelteKit 模块在 vitest 里不存在, 用 mock 文件替代
			// (vi.mock 在 setup 里 hoist 不稳定, alias 方式更稳)
			"$app/environment": fileURLToPath(
				new URL("./tests/mocks/app/environment.ts", import.meta.url)
			),
		},
		// Svelte 5 mount()/unmount() 在 jsdom 环境下走 client 入口, 不要
		// 默认的 node 条件 (会命中 index-server.js → lifecycle_function_unavailable)
		conditions: ["browser"],
	},
	test: {
		// Svelte 5 runes 需要 jsdom 模拟 (crypto.randomUUID / localStorage)
		environment: "jsdom",
		globals: false,
		include: ["src/**/*.{test,spec}.{js,ts}"],
		setupFiles: ["./vitest.setup.ts"],
		// jsdom + Svelte 5 mount 兼容: 让 svelte 包用 browser 条件
		server: {
			deps: {
				inline: [/svelte/],
			},
		},
	},
});
