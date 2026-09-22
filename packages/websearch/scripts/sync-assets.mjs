// 把本包构建产物同步进宿主内嵌资产（src-tauri/src/websearch/assets）。
// 用法：pnpm sync:websearch（= build:websearch + 本脚本）。
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const pkg = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const assets = resolve(pkg, '../../src-tauri/src/websearch/assets');

const copies = [
  ['package.json', 'package.json'],
  ['dsh/index.js', 'dsh/index.js'],
  ['dsh/client.js', 'dsh/client.js'],
  ['dsh/spawnHidden.js', 'dsh/spawnHidden.js'],
  ['dsh/search-schema.json', 'dsh/search-schema.json'],
  ['dsh/fetch-schema.json', 'dsh/fetch-schema.json'],
  ['dist/main.js', 'dist/main.js'],
];

for (const [from, to] of copies) {
  const target = join(assets, to);
  mkdirSync(dirname(target), { recursive: true });
  copyFileSync(join(pkg, from), target);
  console.log(`synced ${from} -> ${to}`);
}
console.log('assets 已同步：重新 cargo build 后随安装包生效');
