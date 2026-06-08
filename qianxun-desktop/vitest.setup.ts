// qianxun-desktop/vitest.setup.ts
// Mock SvelteKit-only modules for vitest (jsdom 跑不到 SvelteKit 内部 runtime)
//
// 覆盖:
//   - $app/environment → { browser: true }
//   - $env/dynamic/public → { env: {} } (env 走 import.meta.env 路径)
//
// 测试时通过 vi.stubEnv('PUBLIC_QIANXUN_DAEMON_URL', ...) 注入运行时变量

import { vi } from 'vitest';

vi.mock('$app/environment', () => ({
	browser: true,
	dev: true,
	building: false,
	version: 'test'
}));

vi.mock('$env/dynamic/public', () => ({
	env: {}
}));
