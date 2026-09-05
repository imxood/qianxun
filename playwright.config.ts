import { defineConfig } from '@playwright/test';

/**
 * 千寻 e2e（WebView2 CDP）。
 *
 * 约束：只允许 debug 构建（target/debug/qianxun.exe）。debug 二进制经
 * cfg!(debug_assertions) 把数据目录隔离到 ~/.qianxun_dev，绝不触碰
 * 安装版（release）的 ~/.qianxun 生产数据——fixture 里有双重断言兜底。
 *
 * 运行：pnpm e2e（先 vite build + cargo build，再起 e2e）。
 * 单实例互斥：运行前先关掉正在跑的 dev/安装版实例，否则 spawn 的
 * 进程会被 single_instance 拦截退出（fixture 报错提示）。
 */
export default defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  // 多窗口 + 固定 CDP 端口：必须串行，避免实例间互相打架。
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: 'list',
  use: {
    trace: 'retain-on-failure',
  },
});
