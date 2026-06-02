import adapter from '@sveltejs/adapter-static';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	compilerOptions: {
		// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
		runes: ({ filename }) => (filename.split(/[/\\]/).includes('node_modules') ? undefined : true)
	},
	kit: {
		// adapter-static (Stage 5 §12): 全平台生产打包输出静态资源,
		// Tauri 走 frontendDist 加载. fallback: 'index.html' 让 SvelteKit
		// 处理 SPA 路由 fallback (Tauri 客户端单页应用).
		adapter: adapter({
			fallback: 'index.html',
			precompress: false,
			strict: false
		}),
		// Path aliases for shadcn-svelte
		alias: {
			$components: 'src/lib/components',
			$utils: 'src/lib/utils',
			$ui: 'src/lib/components/ui',
			$hooks: 'src/lib/hooks'
		}
	}
};

export default config;
