// Vitest stub for $app/paths
// 测试环境需要 `base` 字符串让 Sidebar/页面渲染不报 undefined.
// 跟 svelte.config.js `kit.paths.base = '/ui'` 保持一致 — Sidebar 测试期望
// `href="/ui/llm"` 等. 测试渲染时 base 必须是 `/ui` (带尾斜杠会算错 href).
export const base = '/ui';
