/**
 * 同步 playwright MCP 资源到 src-tauri/vendor/playwright-mcp/，作为打包
 * 资源随千寻分发（bundle.resources → resource_dir/playwright-mcp →
 * 部署拷贝到 DSH profile node_modules）：
 *
 * @playwright/mcp + playwright + playwright-core 三件套——内置 MCP 版
 * playwright 工具（browser use），免 npx 运行时下载。
 *
 * playwright / playwright-core 必须取 @playwright/mcp 自己声明的版本
 * （可能是 alpha 线，与仓库顶层依赖不同），从 mcp 包位置锚定解析。
 *
 * 幂等：版本一致跳过；源缺失（如 CI 未装依赖）时跳过不报错。
 */
import { createRequire } from 'node:module';
import process from 'node:process';
import { cpSync, rmSync, readFileSync, realpathSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const vendorRoot = resolve(dirnameOf(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'vendor');

function dirnameOf(file) {
  return resolve(file, '..');
}

function installedVersion(dir) {
  try {
    return JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8')).version;
  } catch {
    return null;
  }
}

function copyTree(sourceDir, destDir) {
  rmSync(destDir, { recursive: true, force: true });
  cpSync(sourceDir, destDir, { recursive: true });
}

// ---- playwright-mcp 三件套（MCP 版 playwright 工具） ----
const mcpPkg = (() => {
  try {
    return realpathSync(require.resolve('@playwright/mcp/package.json'));
  } catch {
    return null;
  }
})();
if (!mcpPkg) {
  console.warn('[sync-pwc] 未找到 @playwright/mcp 依赖，跳过同步');
  process.exit(0);
}

const mcpRequire = createRequire(mcpPkg);
const resolveMcpDep = (name) => {
  try {
    return realpathSync(mcpRequire.resolve(`${name}/package.json`));
  } catch {
    return null;
  }
};

const nodeModules = join(vendorRoot, 'playwright-mcp', 'node_modules');
for (const name of ['@playwright/mcp', 'playwright', 'playwright-core']) {
  const pkg = name === '@playwright/mcp' ? mcpPkg : resolveMcpDep(name);
  if (!pkg) {
    console.warn(`[sync-pwc] 未找到 ${name}，三件套不完整`);
    continue;
  }
  const dest = join(nodeModules, name);
  const version = installedVersion(dirnameOf(pkg));
  if (version && version === installedVersion(dest)) {
    console.log(`[sync-pwc] ${name} ${version} 已同步，跳过`);
  } else {
    copyTree(dirnameOf(pkg), dest);
    console.log(`[sync-pwc] ${name} ${version} → vendor/playwright-mcp/node_modules/`);
  }
}
