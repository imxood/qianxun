import tailwindcss from '@tailwindcss/vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [tailwindcss(), sveltekit()],
	server: {
		port: 5174,
		// Stage 7a §7.2: dev proxy /v1/* → daemon
		// daemon 端后续加 /_ui/* serve 静态, 但 dev 模式 UI 走 vite, 所以
		// 浏览器同源请求 /v1/* 全部转发到 23900 端口 daemon.
		proxy: {
			'/v1': {
				target: 'http://127.0.0.1:23900',
				changeOrigin: true
			}
		}
	},
	preview: {
		port: 5174
	}
});
