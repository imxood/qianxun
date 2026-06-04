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
	// 2026-06-05 fix: vite `base: '/ui/'` 取代 SvelteKit `kit.paths.base`.
	// 之前 commit 1 改 `kit.paths.base='/ui'`, 但 SvelteKit 2.61 dev 模式 client
	// router 启动时把 base 应用到当前 URL (`/ui/llm`), 试图 find 路由
	// `(/ui)` 找不到 → "Not found: /ui" 错. 改用 vite `base` (控制资源 URL 前缀),
	// 路由 base 留空 — SvelteKit 路由直接用 `/llm` 路径, 客户端 router 不剥
	// base. 这样:
	//   - 资源 (js/css): /ui/_app/... ✓ (vite base)
	//   - 路由: /llm → 客户端 router 拼 base=/ui/ → navigate /ui/llm ✓
	//   - 浏览器 URL: /ui/llm (SvelteKit 自动拼)
	//   - daemon 反代: /ui/* → vite /ui/* (我 router.rs handler 拼 /ui prefix)
	base: '/ui/',
	server: {
		port: 5174,
		// Stage 12: bind 0.0.0.0 让 daemon 反代 (Python urllib / Invoke-WebRequest
		// / 系统代理) 都能连. 默认 Vite 只 bind localhost (Windows 上解析到 ::1
		// IPv6), 跟 127.0.0.1 IPv4 测试不互通 → 502. 改成 '0.0.0.0' 让 IPv4/IPv6
		// 都能连. strictPort 避免启动失败时悄悄切端口.
		host: '0.0.0.0',
		strictPort: true,
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
		port: 5174,
		host: '0.0.0.0',
		strictPort: true
	}
});
