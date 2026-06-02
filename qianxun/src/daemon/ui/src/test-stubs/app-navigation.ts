// Vitest stub for $app/navigation
// ui.svelte.ts 在 onMount 里注册 afterNavigate 关闭 sidebar,
// 测试环境没有真正的 SvelteKit 路由, 暴露一个 noop 即可.
export const afterNavigate = (_fn: () => void) => {
	// no-op in test
	return () => {
		/* no-op teardown */
	};
};
export const beforeNavigate = (_fn: () => void) => {
	return () => {
		/* no-op teardown */
	};
};
export const goto = async (_url: string, _opts?: unknown) => {
	/* no-op */
};
export const invalidate = async (_key?: unknown) => {
	/* no-op */
};
export const invalidateAll = async () => {
	/* no-op */
};
