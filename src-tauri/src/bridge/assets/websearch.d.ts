/**
 * websearch.js 的类型声明：桥源保持零构建纯 JS（部署即文件拷贝），
 * TS 侧（vitest 测试 / 未来前端复用）经本声明获得类型。
 */

export interface SearchItem {
  title: string;
  url: string;
  snippet: string;
  engine: string;
}

export type EngineKind = "bing" | "baidu" | "duckduckgo" | "searxng";

export interface EngineSpec {
  kind: EngineKind;
  endpoint?: string;
}

/** 内置引擎 URL 模板（kind → 含 {Q}/{E} 占位符的模板）。 */
export const ENGINE_TEMPLATES: Record<EngineKind, string>;

/**
 * 拼搜索 URL。engine 为内置 kind 字符串或 { kind, endpoint }；
 * 空查询 / 未知 kind / searxng 缺 https endpoint 抛错。
 */
export function buildSearchUrl(engine: string | EngineSpec, query: string): string;

/**
 * 从搜索引擎结果页 Markdown 提取结构化条目：
 * 过滤引擎自身噪音链接，按 URL（去 hash）去重，limit 截断。
 */
export function parseSearchMarkdown(
  markdown: string,
  engineLabel: string,
  limit?: number,
): SearchItem[];

/** items → Markdown 列表（agent 与 UI 共用的最终形态）；空数组输出占位文案。 */
export function itemsToMarkdown(items: SearchItem[]): string;
