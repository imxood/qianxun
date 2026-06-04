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
		// 2026-06-05 fix: SvelteKit paths.base 留空, base path 改用 vite
		// `base: '/ui/'` (见 vite.config.ts). SvelteKit 2.61 dev 模式 + paths.base
		// 在客户端 router 启动时会跟当前 URL 冲突, 报 "Not found: /ui".
		// Prod build 模式下, base 由 vite adapter 自动处理.
		alias: {
			$components: 'src/lib/components',
			$utils: 'src/lib/utils',
			$ui: 'src/lib/components/ui',
			$hooks: 'src/lib/hooks'
		}
	}
};

export default config;
