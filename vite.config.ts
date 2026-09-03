import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';

// 5180：避开本机其他 Vite 应用。strictPort 保证
// tauri.conf.json 里的 devUrl 永远指向这里，端口漂移会让白屏很难排查。
export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  clearScreen: false,
  server: {
    port: 5180,
    strictPort: true,
    warmup: {
      // tauri dev 先起 Vite、再编译 Rust：利用编译窗口把整张模块图预先
      // 转换好，窗口打开时首屏不再一段段现拉现转（冷启动白屏的主因）。
      clientFiles: ['./index.html', './src/main.ts', './src/**/*.svelte', './src/**/*.ts'],
    },
    watch: {
      // 两个会崩掉 dev server 的目录：Rust 构建产物（Windows 锁着其中的
      // .dll，chokidar fs.watch 撞 EBUSY 直接退出）与编辑工具原子写的
      // `.<name>.<pid>.<uuid>.tmpdir` 临时目录（同样会撞锁）。
      ignored: ['**/src-tauri/target/**', '**/.*.tmpdir/**'],
    },
  },
  optimizeDeps: {
    // 首次冷启动就把重依赖预打包完：否则 Vite 会在页面加载中途发现新
    // 依赖 → 重新预构建 → 整页 reload，体感是「白屏后重来一遍」。
    include: [
      'codemirror',
      '@codemirror/view',
      '@codemirror/state',
      '@codemirror/lang-markdown',
      '@codemirror/language-data',
      '@xterm/xterm',
      '@xterm/addon-fit',
      '@xterm/addon-webgl',
      'marked',
      'qrcode',
      '@tauri-apps/api/core',
      '@tauri-apps/api/event',
      '@tauri-apps/api/window',
      '@tauri-apps/plugin-opener',
    ],
  },
});
