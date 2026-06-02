// Vitest stub for $app/state
// 测试环境需要 page.url.pathname 让 Sidebar 不报 undefined
export const page = {
	url: new URL('http://localhost:3000/'),
	params: {},
	route: { id: null },
	status: 200,
	error: null,
	data: {}
};
