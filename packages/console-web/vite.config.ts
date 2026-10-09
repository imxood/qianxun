import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';

// 服务端控制台（docs/20 §7.4 决策 1-3）：独立于桌面端的第二个前端包，
// 产物 dist/ 由 qx-server 的 build.rs 编译进二进制（单二进制铁律）。
//
// 复用仓库根的依赖（svelte / vite / tailwind 都已在根 package.json 里），
// 所以这里没有独立的 package.json / node_modules——pnpm 不装第二份依赖树，
// 也不需要第二个 lockfile。配置单独一份，不污染根 vite.config.ts（5190 端口、
// 桌面端的 optimizeDeps 与 watch 规则都与本包无关）。

const root = fileURLToPath(new URL('.', import.meta.url));
const proxyTarget = process.env.QX_SERVER ?? 'http://127.0.0.1:23100';

export default defineConfig({
  root,
  base: '/console/',
  plugins: [tailwindcss(), svelte()],
  clearScreen: false,
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // 自托管局域网管理面：现代浏览器基线即可，产物不必为旧 Safari 降级。
    target: 'es2022',
    assetsInlineLimit: 0, // 二维码等已内联进 SVG；其余保持独立文件便于长期缓存
    rollupOptions: {
      output: {
        // 单 chunk：控制台首屏即全部，拆包只会多几个 RTT。
        manualChunks: undefined,
      },
    },
  },
  server: {
    // 5191：紧邻桌面端 5190，避免与其他 Vite 应用撞端口。
    port: 5191,
    strictPort: true,
    proxy: {
      // dev 时 API 打到本地 qx-server，Cookie 同源免 CORS。
      '/console/api': { target: proxyTarget, changeOrigin: false },
      '/login': { target: proxyTarget, changeOrigin: false },
      // 登录页底部的「去配对」指向服务端的零构建手写页：必须打给真实服务端，
      // 否则 vite 的 SPA fallback 会拿 index.html 顶掉它（链接看着能点，点过去是控制台自己）。
      '/qx-pair': { target: proxyTarget, changeOrigin: false },
    },
    watch: {
      ignored: ['**/src-tauri/target/**', '**/.*.tmpdir/**', '**/.tmp/**'],
    },
  },
});
