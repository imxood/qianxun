// Playwright 配置 — Web Console E2E (Stage 8c)
//
// 端到端测试真连 daemon (port 23913, 由 start-daemon.bat 启动):
// 1. webServer 自动拉起 `pnpm dev` (vite, port 5174), Vite proxy /v1/* → daemon
// 2. 浏览器访问 http://127.0.0.1:5174/, 弹 admin token 输入框, 粘 JWT, 进主界面
// 3. 走完 8 个面板: LLM / Skills / MCP / Tools / Memory / Sessions / Config / System
//
// 启动前: 确保 daemon 在 DAEMON_PORT 端口跑, JWT_SECRET 已知.
// webServer.reuseExistingServer = true, 允许多次跑测试时复用已有 vite.

import { defineConfig, devices } from '@playwright/test';

const DAEMON_PORT = process.env.DAEMON_PORT ?? '23913';

export default defineConfig({
	testDir: './tests/e2e',
	// LLM test 端到端可等 30s+ (走真 provider), 60s 留 buffer
	timeout: 60_000,
	expect: {
		timeout: 10_000
	},
	fullyParallel: false,
	workers: 1,
	retries: 0,
	reporter: [['list'], ['html', { open: 'never', outputFolder: 'tests/e2e/report' }]],
	use: {
		// Vite dev 走 5174; daemon 走 DAEMON_PORT, 但浏览器只发到 5174 (proxy)
		baseURL: 'http://127.0.0.1:5174',
		trace: 'retain-on-failure',
		screenshot: 'only-on-failure',
		viewport: { width: 1280, height: 800 }
	},
	projects: [
		{
			name: 'chromium',
			use: { ...devices['Desktop Chrome'] }
		}
	],
	webServer: {
		// 启动 vite dev, 注入 DAEMON_PORT 让 vite.config 知道 proxy 目标
		command: `pnpm dev`,
		url: 'http://127.0.0.1:5174',
		timeout: 120_000,
		reuseExistingServer: true,
		stdout: 'pipe',
		stderr: 'pipe',
		env: {
			DAEMON_PORT
		}
	}
});
