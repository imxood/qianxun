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
		alias: {
			$components: 'src/lib/components',
			$utils: 'src/lib/utils',
			$ui: 'src/lib/components/ui',
			$hooks: 'src/lib/hooks'
		}
	}
};

export default config;
