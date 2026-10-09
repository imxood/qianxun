/**
 * 控制台资产的构建入口（`pnpm build:console`）。
 *
 * 一次做两件事：
 *   1. vite build —— Svelte 5 + Tailwind v4 打出 dist/；
 *   2. 预压缩 —— 每个产物再写一份 .gz 到 dist-gz/，Rust 侧按 Accept-Encoding 直接
 *      发压缩件，不再在运行时压。控制台首屏 ~40 KB gzip，自托管局域网也不该
 *      用明文跑。
 *
 * 为什么压缩放在这里而不是 build.rs：build.rs 只能用纯 Rust 可用的 API，
 * 拉一个 flate2 只为构建期压缩不值得；而这一步本来就属于前端构建的收尾。
 * 压缩率对构建耗时没要求，gzipSync level 9 足够。
 */
import { spawnSync } from 'node:child_process';
import { gzipSync } from 'node:zlib';
import { mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');
const repoRoot = resolve(root, '..', '..');

// 直接用 node 跑 vite 的入口脚本，不经过 pnpm/.cmd shim：构建脚本里 spawn
// 一个 .cmd 在 Windows 上容易因为 PATHEXT/shell 差异静默失败。
const viteBin = join(repoRoot, 'node_modules', 'vite', 'bin', 'vite.js');
const vite = spawnSync(
  process.execPath,
  [viteBin, 'build', '--config', 'packages/console-web/vite.config.ts'],
  { cwd: repoRoot, stdio: 'inherit' },
);
if (vite.error) {
  console.error(`无法启动 vite：${vite.error.message}`);
  process.exit(1);
}
if (vite.status !== 0) process.exit(vite.status ?? 1);

const dist = join(root, 'dist');
const distGz = join(root, 'dist-gz');
rmSync(distGz, { recursive: true, force: true });
mkdirSync(distGz, { recursive: true });

/** 递归列出 dist 下的文件（Vite 产物只有两层，但别假设）。 */
function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) yield* walk(full);
    else yield full;
  }
}

let raw = 0;
let gz = 0;
for (const file of walk(dist)) {
  const name = relative(dist, file).replaceAll('\\', '/');
  const bytes = readFileSync(file);
  const packed = gzipSync(bytes, { level: 9 });
  const target = join(distGz, `${name}.gz`);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, packed);
  raw += bytes.length;
  gz += packed.length;
  console.log(`  ${name.padEnd(28)} ${String(bytes.length).padStart(7)} → ${packed.length} B`);
}
console.log(`控制台资产 ${gz} B gzip / ${raw} B 明文 → ${relative(repoRoot, dist)}`);
