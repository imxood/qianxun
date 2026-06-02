// Vitest stub for $app/environment
// 实际 SvelteKit 提供 browser / dev / building, 这里在测试环境下模拟浏览器
// (让 stores 的 localStorage / document 路径都激活).
export const browser = true;
export const dev = true;
export const building = false;
export const version = 'test';
