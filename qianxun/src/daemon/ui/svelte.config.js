import adapter from '@sveltejs/adapter-static';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	compilerOptions: {
		// Force runes mode for the project, except for libraries.
		runes: ({ filename }) =>
			filename.split(/[/\\]/).includes('node_modules') ? undefined : true
	},
	kit: {
		// Stage 7a §2.2: adapter-static + fallback 'index.html'
		// 走 SPA 模式, 所有非资源路径 fall back 到 index.html (svelte-router 接管).
		adapter: adapter({
			fallback: 'index.html',
			precompress: false,
			strict: false
		}),
		// 2026-06-05 fix v6: `paths.base='/ui'` 在 prod build 会被注入到
		// `__sveltekit_xxx.base = "/ui"` (跟 dev 模式的 __sveltekit_dev 不同).
		// prod 模式跑 daemon 静态模式: 浏览器在 /ui, base='/ui' → 路由根='/' → 找
		// +page.svelte (welcome) ✓. dev 模式 base 公式固定返空 跟 paths.base
		// 冲突, dev mode 仍是 broken (单独走 vite 直连 5174 调试, 不靠 daemon
		// 反代). 这次 focus 是 prod mode 跑通 — daemon 静态 + 浏览器 /ui 入口.
		paths: {
			base: '/ui',
			relative: true
			// 注: SvelteKit 2.61 没有 `paths.trailingSlash` option. 改用
			// `paths.relative: true` + daemon router 加 redirect /ui → /ui/
			// (v7 fix 在 router.rs).
		},
		alias: {
			$components: 'src/lib/components',
			$utils: 'src/lib/utils',
			$ui: 'src/lib/components/ui',
			$hooks: 'src/lib/hooks'
		}
	}
};

export default config;
