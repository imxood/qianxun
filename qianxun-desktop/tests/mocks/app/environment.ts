// Mock for $app/environment (SvelteKit module that doesn't exist in vitest)
// 用法: 在 vitest.config.ts 的 resolve.alias 指向这个文件
export const browser = true;
export const dev = true;
export const building = false;
export const version = 'test';
