// 校验内嵌资产与 websearch 包构建产物**逐字节一致**（docs/18 C6）。
//
// 背景：`sync-assets.mjs` 把 7 个文件从 packages/websearch 复制到
// src-tauri/src/websearch/assets，两处各存一份（含两个 1.2MB 的 dist/main.js）。
// 此前同步完全靠人工跑 `pnpm sync:websearch`，没有任何门禁——漏跑时
// 编译期资产与包源码静默漂移，而运行时 `plugin_stale` 自愈只能修用户
// profile，修不了编译期 assets。
//
// 用法：
//   node scripts/verify-assets.mjs          漂移则 exit 1 并列出差异文件
//   node scripts/verify-assets.mjs --fix    漂移则自动同步（等价 sync-assets）
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const pkg = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const assets = resolve(pkg, '../../src-tauri/src/websearch/assets');

/** 与 sync-assets.mjs 的清单保持一致（单一事实源在下方，二者共用）。 */
export const COPIES = [
  ['package.json', 'package.json'],
  ['dsh/index.js', 'dsh/index.js'],
  ['dsh/client.js', 'dsh/client.js'],
  ['dsh/spawnHidden.js', 'dsh/spawnHidden.js'],
  ['dsh/search-schema.json', 'dsh/search-schema.json'],
  ['dsh/fetch-schema.json', 'dsh/fetch-schema.json'],
  ['dist/main.js', 'dist/main.js'],
];

const sha256 = (path) => createHash('sha256').update(readFileSync(path)).digest('hex');

const fix = process.argv.includes('--fix');
const drifted = [];
const missing = [];

for (const [from, to] of COPIES) {
  const src = join(pkg, from);
  const target = join(assets, to);
  if (!existsSync(src)) {
    missing.push(`${from}（源不存在——先跑 pnpm build:websearch）`);
    continue;
  }
  if (!existsSync(target)) {
    drifted.push([from, to, '目标缺失']);
    if (fix) {
      mkdirSync(dirname(target), { recursive: true });
      copyFileSync(src, target);
    }
    continue;
  }
  const a = sha256(src);
  const b = sha256(target);
  if (a !== b) {
    drifted.push([from, to, `${a.slice(0, 12)} ≠ ${b.slice(0, 12)}`]);
    if (fix) {
      mkdirSync(dirname(target), { recursive: true });
      copyFileSync(src, target);
    }
  }
}

if (missing.length > 0) {
  console.error('✗ websearch 资产校验失败：源文件缺失');
  for (const item of missing) console.error(`  - ${item}`);
  process.exit(1);
}

if (drifted.length === 0) {
  console.log(`✓ websearch 资产一致（${COPIES.length} 个文件，SHA-256 逐一比对）`);
} else if (fix) {
  console.log(`⟳ 已同步 ${drifted.length} 个漂移文件：`);
  for (const [from, to] of drifted) console.log(`  ${from} -> ${to}`);
} else {
  console.error(`✗ websearch 资产漂移（${drifted.length}/${COPIES.length} 个文件不一致）：`);
  for (const [from, to, why] of drifted) console.error(`  ${from} -> ${to}  ${why}`);
  console.error('\n修复：pnpm sync:websearch（或 node scripts/verify-assets.mjs --fix）');
  process.exit(1);
}
