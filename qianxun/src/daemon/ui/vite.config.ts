import tailwindcss from '@tailwindcss/vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

// Stage 8c: 端口跟 Stage 7a 默认 23900 保持, 也允许 DAEMON_PORT env 覆盖
// (例如 E2E 跑 23913). Playwright 配置里 webServer.reuseExistingServer + 5174
// 直接走 vite dev, 浏览器 fetch('/v1/...') → 5174 → proxy → daemon.
const DAEMON_TARGET = process.env.DAEMON_PORT
	? `http://127.0.0.1:${process.env.DAEMON_PORT}`
	: 'http://127.0.0.1:23900';

export default defineConfig({
	plugins: [tailwindcss(), sveltekit()],
	server: {
		port: 5174,
		// Stage 7a §7.2: dev proxy /v1/* → daemon
		// daemon 端后续加 /ui/* serve 静态, 但 dev 模式 UI 走 vite, 所以
		// 浏览器同源请求 /v1/* 全部转发到 DAEMON_TARGET 端口 daemon.
		proxy: {
			'/v1': {
				target: DAEMON_TARGET,
				changeOrigin: true
			}
		}
	},
	preview: {
		port: 5174
	}
});
