/**
 * 插件市场的浏览数据类型（前端自有，不经 IPC）。
 *
 * 浏览/搜索在 webview 里直接 fetch（npmmirror 全端点带 CORS，走系统代理）；
 * Rust 侧只保留变更面（安装预检/卸载/已装清单），见 market/mod.rs。
 */

/** 统一列表行：推荐（catalog）与搜索（npm search）都归一化到这个形状。 */
export interface MarketListing {
  /** 可安装的 npm 包名（一期只收录发布了 npm 包的条目）。 */
  name: string;
  version: string;
  /** 中文描述优先。 */
  description: string;
  publisher: string;
  /** YYYY-MM-DD。 */
  updated: string;
  weeklyDownloads: number;
  stars: number | null;
  link: string | null;
  /** 所属分类 id（推荐源；搜索源为空）。 */
  categoryIds: string[];
  source: 'featured' | 'search';
}

/** 兼容结论（展示用；安装时 Rust 侧另有硬门槛，这里误判也不会放行坏包）。 */
export type MarketCompatibility =
  | { state: 'compatible'; requirement: string }
  | { state: 'unknown' }
  | { state: 'incompatible'; requirement: string; reason: string };

/** npm manifest 详情（展开行 + 安装版本的依据）。 */
export interface MarketDetail {
  name: string;
  version: string;
  description: string;
  license: string;
  homepage: string | null;
  repository: string | null;
  /** 声明了 dsh.bundle.patch = 插件；否则只是普通包。 */
  bundle: boolean;
  compatibility: MarketCompatibility;
  lifecycleScripts: string[];
  /** 存在即展示弃用警告（Rust 安装预检会直接拒绝）。 */
  deprecated: string | null;
  unpackedBytes: number | null;
  installSpec: string;
}

/** 推荐目录的一个分类（label 取中文名）。 */
export interface CatalogCategory {
  id: string;
  label: string;
}

/** 推荐目录（plugins.json 归一化后；只保留发布了 npm 包的条目）。 */
export interface CatalogData {
  /** 目录构建日期（YYYY-MM-DD）。 */
  updated: string;
  /** 上游原始条数（含 GitHub-only）。 */
  total: number;
  entries: MarketListing[];
  categories: CatalogCategory[];
}
