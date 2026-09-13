/**
 * 推荐目录源：dsh-plugin-catalog。
 *
 * awesome-dsh-plugin（社区策展）每日构建发布到 npm，天然跟随国内镜像；
 * tar.gz 里的 plugins.json 由前端解包（DecompressionStream + 手写 tar
 * reader，零依赖）。失败即报错——陈旧的目录是错误答案，不是降级答案。
 */

import { fetchBytes, fetchJson } from './http';
import { fileFromTarGz } from './tar';
import type { CatalogCategory, CatalogData, MarketListing } from './types';

const CATALOG_PACKAGE = 'dsh-plugin-catalog';
const CATALOG_FILE = 'package/plugins.json';
const CATALOG_TTL_MS = 10 * 60 * 1000;

interface RawPlugin {
  name: string;
  owner: string;
  url?: string;
  category?: string | string[];
  description?: { en?: string; zh?: string };
  npm?: string | null;
  version?: string | null;
  stars?: number | null;
  downloads?: number | null;
  added?: string;
}

interface RawCatalog {
  updated?: string;
  count?: number;
  categories?: Record<string, { en?: string; zh?: string }>;
  plugins?: RawPlugin[];
}

let cache: { prefix: string; fetchedAt: number; data: CatalogData } | null = null;

/**
 * 拉推荐目录。跟随用户配置的 registry（镜像内直达）；10 分钟内存 TTL。
 * @param registry registry base（组件侧已按镜像设置解析）。
 * @param force 跳过 TTL 强制刷新。
 */
export async function loadCatalog(registry: string, force = false): Promise<CatalogData> {
  const prefix = registry.endsWith('/') ? registry : `${registry}/`;
  if (!force && cache && cache.prefix === prefix && Date.now() - cache.fetchedAt < CATALOG_TTL_MS) {
    return cache.data;
  }
  const meta = (await fetchJson(`${prefix}${CATALOG_PACKAGE}/latest`)) as {
    dist?: { tarball?: unknown };
  };
  const tarball = meta.dist?.tarball;
  if (typeof tarball !== 'string') throw new Error('目录包缺少 tarball 地址');
  // 跟随 dist.tarball 而不是自己拼 URL：镜像会把它重写为自己的 host，
  // 跟着走才能留在镜像内。
  const json = await fileFromTarGz(await fetchBytes(tarball), CATALOG_FILE);
  if (json === null) throw new Error('目录包里没有 plugins.json');
  const data = normalizeCatalog(JSON.parse(json) as RawCatalog);
  cache = { prefix, fetchedAt: Date.now(), data };
  return data;
}

function normalizeCatalog(raw: RawCatalog): CatalogData {
  // 一期只收录发布了 npm 包的条目（GitHub 源安装是二期）。
  const entries = (raw.plugins ?? [])
    .filter((plugin) => typeof plugin.npm === 'string' && plugin.npm !== '')
    .map((plugin): MarketListing => {
      const categoryIds = Array.isArray(plugin.category)
        ? plugin.category
        : plugin.category
          ? [plugin.category]
          : [];
      return {
        name: plugin.npm ?? plugin.name,
        version: plugin.version ?? '',
        description: plugin.description?.zh || plugin.description?.en || '',
        publisher: plugin.owner,
        updated: (plugin.added ?? '').slice(0, 10),
        weeklyDownloads: plugin.downloads ?? 0,
        stars: plugin.stars ?? null,
        link: plugin.url ?? null,
        categoryIds,
        source: 'featured',
      };
    });
  const categories: CatalogCategory[] = Object.entries(raw.categories ?? {}).map(
    ([id, labels]) => ({ id, label: labels?.zh || labels?.en || id }),
  );
  return {
    updated: raw.updated ?? '',
    total: raw.count ?? raw.plugins?.length ?? 0,
    entries,
    categories,
  };
}
