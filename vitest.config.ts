import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  test: {
    environment: 'node',
    include: [
      'src/**/*.test.ts',
      'e2e/**/*.test.ts',
      // 控制台的纯函数（格式化/二维码）也要有回归网：docs/20 §4.3 的
      // 「缺桶补 0」这类形状逻辑，错一次就是图表画错而不是报错。
      'packages/console-web/src/**/*.test.ts',
      'src-tauri/src/bridge/assets/**/*.test.js',
      'src-tauri/src/bridge/assets/**/*.test.ts',
      'src-tauri/src/playwright/assets/**/*.test.ts',
    ],
  },
});
