// vitest 配置 — Svelte 5 runes 感知 + jsdom 环境
// 与 qianxun-desktop/vitest.config.ts 保持一致, 避免 mount() 的 server 入口陷阱.
import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath, URL } from 'node:url';

export default defineConfig({
	plugins: [svelte()],
	resolve: {
		alias: {
			$lib: fileURLToPath(new URL('./src/lib', import.meta.url)),
			'$lib/*': fileURLToPath(new URL('./src/lib/*', import.meta.url)),
			// 测试环境 stub — SvelteKit runtime 不在, 我们的 store 用了 $app/environment
			'$app/environment': fileURLToPath(
				new URL('./src/test-stubs/app-environment.ts', import.meta.url)
			),
			'$app/state': fileURLToPath(
				new URL('./src/test-stubs/app-state.ts', import.meta.url)
			),
			'$app/navigation': fileURLToPath(
				new URL('./src/test-stubs/app-navigation.ts', import.meta.url)
			)
		},
		// Svelte 5 mount/unmount 在 jsdom 走 client 入口, 不要默认的 node 条件
		conditions: ['browser']
	},
	test: {
		environment: 'jsdom',
		globals: true,
		include: ['src/**/*.{test,spec}.{js,ts}'],
		// @testing-library/svelte 依赖 afterEach 自动清理 DOM
		// 注意: 这里设成 true 让 vitest 注入 beforeEach/afterEach
		setupFiles: ['./src/test-setup.ts'],
		// 路由页面动态 import .svelte 首次编译慢 (~3s), 给 15s 留 buffer
		testTimeout: 15000,
		server: {
			deps: {
				inline: [/svelte/]
			}
		}
	}
});
