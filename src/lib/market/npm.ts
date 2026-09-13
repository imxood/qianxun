/**
 * 插件市场数据源：npm 搜索 + 详情复核（前端侧）。
 *
 * registry base 来自镜像设置，与 Rust settings::registry_url() 同一条映射；
 * fetch 走系统代理（WebView2 网络栈），npmmirror 全端点带 CORS（实测）。
 * 安装的硬门槛在 Rust 安装预检里，这里的兼容结论只影响展示。
 */

import { fetchJson } from './http';
import { rangeMatches } from './semver';
import type { MarketCompatibility, MarketDetail, MarketListing } from './types';

/** 镜像设置 → registry base（与 Rust settings::registry_url 对齐）。 */
export function registryBase(npmRegistry: string | undefined): string {
  if (npmRegistry === 'official') return 'https://registry.npmjs.org/';
  if (npmRegistry === 'npmmirror' || npmRegistry === undefined || npmRegistry === '') {
    return 'https://registry.npmmirror.com/';
  }
  return /^https:\/\//u.test(npmRegistry) ? npmRegistry : 'https://registry.npmmirror.com/';
}

/** registry base 归一为带尾斜杠的前缀。 */
function prefixOf(registry: string): string {
  const base = registryBase(registry);
  return base.endsWith('/') ? base : `${base}/`;
}

/** 空搜索框的发现词：不精选、不运营，registry 搜到什么看什么。 */
const DISCOVERY = 'dsh';

export async function searchNpm(registry: string, query: string): Promise<MarketListing[]> {
  const prefix = prefixOf(registry);
  const text = query.trim() === '' ? DISCOVERY : query.trim();
  const body = (await fetchJson(
    `${prefix}-/v1/search?text=${encodeURIComponent(text)}&size=100`,
  )) as {
    objects?: Array<{
      package?: {
        name?: unknown;
        version?: unknown;
        description?: unknown;
        date?: unknown;
        publisher?: { username?: unknown };
        links?: { homepage?: unknown; repository?: unknown };
      };
      downloads?: { weekly?: unknown };
    }>;
  };
  return (body.objects ?? [])
    .map((entry): MarketListing | null => {
      const pkg = entry.package;
      if (typeof pkg?.name !== 'string') return null;
      return {
        name: pkg.name,
        version: typeof pkg.version === 'string' ? pkg.version : '',
        description: typeof pkg.description === 'string' ? pkg.description : '',
        publisher: typeof pkg.publisher?.username === 'string' ? pkg.publisher.username : '',
        updated: typeof pkg.date === 'string' ? pkg.date.slice(0, 10) : '',
        weeklyDownloads: typeof entry.downloads?.weekly === 'number' ? entry.downloads.weekly : 0,
        stars: null,
        link:
          typeof pkg.links?.homepage === 'string'
            ? pkg.links.homepage
            : typeof pkg.links?.repository === 'string'
              ? pkg.links.repository
              : null,
        categoryIds: [],
        source: 'search',
      };
    })
    .filter((listing): listing is MarketListing => listing !== null);
}

const detailCache = new Map<string, MarketDetail>();

export async function fetchNpmDetail(
  registry: string,
  name: string,
  pinnedDsh: string,
): Promise<MarketDetail> {
  const prefix = prefixOf(registry);
  // 缓存 key 含钉住的 DSH 版本：千寻升级后兼容结论能自动刷新。
  const cacheKey = `${prefix}\0${pinnedDsh}\0${name}`;
  const cached = detailCache.get(cacheKey);
  if (cached) return cached;
  const manifest = (await fetchJson(`${prefix}${name}/latest`)) as Record<string, unknown>;
  const detail = detailFromManifest(name, manifest, pinnedDsh);
  detailCache.set(cacheKey, detail);
  return detail;
}

function detailFromManifest(
  requested: string,
  manifest: Record<string, unknown>,
  pinnedDsh: string,
): MarketDetail {
  const text = (key: string): string =>
    typeof manifest[key] === 'string' ? (manifest[key] as string) : '';
  const opt = (value: unknown): string | null => (typeof value === 'string' ? value : null);
  const version = text('version') || requested;
  const dependency = (field: string): string | null => {
    const group = manifest[field];
    if (typeof group !== 'object' || group === null) return null;
    const requirement = (group as Record<string, unknown>)['@deepseek-ai/dsh'];
    return typeof requirement === 'string' ? requirement.trim() : null;
  };
  const requirement =
    dependency('peerDependencies') ??
    dependency('dependencies') ??
    dependency('optionalDependencies');
  const compatibility: MarketCompatibility = judgeCompatibility(requirement, pinnedDsh);
  const dist = (manifest.dist ?? {}) as Record<string, unknown>;
  const repository = manifest.repository;
  const dsh = manifest.dsh as { bundle?: { patch?: unknown } } | undefined;
  const lifecycleScripts = ['preinstall', 'install', 'postinstall', 'prepare'].filter(
    (script) =>
      typeof manifest.scripts === 'object' &&
      manifest.scripts !== null &&
      script in (manifest.scripts as Record<string, unknown>),
  );
  return {
    name: text('name') || requested,
    version,
    description: text('description'),
    license: text('license'),
    homepage: opt(manifest.homepage),
    repository:
      opt(repository) ??
      (typeof repository === 'object' && repository !== null
        ? opt((repository as Record<string, unknown>).url)
        : null),
    bundle: typeof dsh?.bundle === 'object' && dsh.bundle !== null && 'patch' in dsh.bundle,
    compatibility,
    lifecycleScripts,
    deprecated: text('deprecated') || null,
    unpackedBytes: typeof dist.unpackedSize === 'number' ? dist.unpackedSize : null,
    installSpec: `${text('name') || requested}@${version}`,
  };
}

function judgeCompatibility(requirement: string | null, pinned: string): MarketCompatibility {
  if (requirement === null || requirement === '') return { state: 'unknown' };
  const verdict = rangeMatches(requirement, pinned);
  if (verdict === null) return { state: 'unknown' };
  return verdict
    ? { state: 'compatible', requirement }
    : { state: 'incompatible', requirement, reason: `它要求 ${requirement}` };
}
