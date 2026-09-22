/**
 * e2e 环境自检（R001 D7/D11）：pnpm e2e:doctor
 *
 * 一切浏览器自动化 = playwright + moli（本机不依赖独立 Edge）：
 * - 依赖解析（@playwright/test / playwright-core）
 * - debug 二进制（千寻本体 e2e 经 WebView2 CDP，属应用自身运行时）
 * - moli 可定位（环境变量 → dev 数据目录 settings → 受管目录 → PATH）
 * - 临时目录变量 / CDP 端口 / 千寻实例互斥
 * 只读检查，不启动浏览器；失败给出修复指引，退出码 1。
 */
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';

const PROJECT_ROOT = path.resolve(import.meta.dirname, '..');
const TAURI_BIN = path.join(PROJECT_ROOT, 'src-tauri', 'target', 'debug', 'qianxun.exe');
const CDP_PORT = 10222;

interface CheckResult {
  name: string;
  ok: boolean;
  detail: string;
  fix?: string;
}

const results: CheckResult[] = [];

function check(name: string, ok: boolean, detail: string, fix?: string): void {
  results.push({ name, ok, detail, fix });
}

// 1. @playwright/test 可解析
try {
  const require = createRequire(path.join(PROJECT_ROOT, 'package.json'));
  const pkg = require('@playwright/test/package.json') as { version: string };
  check('playwright 依赖', true, `@playwright/test ${pkg.version}`);
} catch {
  check('playwright 依赖', false, '@playwright/test 无法解析', 'pnpm install');
}

// 2. playwright-core 可解析（browser_* 自动化的驱动端）
try {
  const require = createRequire(path.join(PROJECT_ROOT, 'package.json'));
  const pkg = require('playwright-core/package.json') as { version: string };
  check('playwright-core', true, `v${pkg.version}`);
} catch {
  check('playwright-core', false, '不可解析', 'pnpm add -D playwright-core');
}

// 3. debug 二进制（千寻本体 e2e：spawn debug exe → WebView2 CDP）
if (fs.existsSync(TAURI_BIN)) {
  check('debug 二进制', true, TAURI_BIN);
} else {
  check('debug 二进制', false, `${TAURI_BIN} 不存在`, 'pnpm e2e:build');
}

// 4. moli 可定位：QX_MOLI_PATH → dev 数据目录 settings/受管目录 → PATH
function findMoli(): { ok: boolean; detail: string } {
  const exeName = process.platform === 'win32' ? 'moli.exe' : 'moli';
  const envPath = process.env.QX_MOLI_PATH?.trim();
  if (envPath) {
    return fs.existsSync(envPath)
      ? { ok: true, detail: envPath }
      : { ok: false, detail: `QX_MOLI_PATH 指向不存在的文件：${envPath}` };
  }
  const devDataDir = path.join(os.homedir(), '.qianxun_dev');
  const candidates = [
    path.join(devDataDir, 'tools', 'moli', exeName),
    path.join(devDataDir, 'moli', exeName),
  ];
  try {
    const settingsPath = path.join(devDataDir, 'settings.json');
    const settings = JSON.parse(fs.readFileSync(settingsPath, 'utf8')) as {
      tools?: { moli?: { binaryPath?: string } };
    };
    const custom = settings.tools?.moli?.binaryPath?.trim();
    if (custom) candidates.unshift(custom);
  } catch {
    // 无 dev settings（未跑过 dev）：跳过该源。
  }
  const pathHit = candidates.find((candidate) => candidate !== '' && fs.existsSync(candidate));
  if (pathHit) return { ok: true, detail: pathHit };
  const pathEnv = process.env.PATH ?? '';
  for (const dir of pathEnv.split(path.delimiter)) {
    const candidate = path.join(dir, exeName);
    if (fs.existsSync(candidate)) return { ok: true, detail: candidate };
  }
  return {
    ok: false,
    detail:
      '未定位到 moli（QX_MOLI_PATH / settings.tools.moli.binaryPath / 受管目录 / PATH 均未命中）',
  };
}
const moli = findMoli();
check(
  'moli 无头浏览器',
  moli.ok,
  moli.detail,
  moli.ok
    ? undefined
    : '设置 settings.json 的 tools.moli.binaryPath，或设 QX_MOLI_PATH，或放入 PATH',
);

// 5. 临时目录变量 + os.tmpdir 可用
const tmp = os.tmpdir();
const tmpOk = tmp !== '' && fs.existsSync(tmp);
check(
  '临时目录',
  tmpOk,
  `os.tmpdir() = ${tmp}`,
  tmpOk ? undefined : '确保 TMP/TEMP/SYSTEMDRIVE 环境变量可用（CI 沙箱常见缺口）',
);

// 6. CDP 端口 10222 空闲（e2e spawn 需要占用）
const portFree = await new Promise<boolean>((resolve) => {
  const server = net.createServer();
  server.once('error', () => resolve(false));
  server.once('listening', () => {
    server.close(() => resolve(true));
  });
  server.listen(CDP_PORT, '127.0.0.1');
});
check(
  'CDP 端口',
  portFree,
  portFree ? `127.0.0.1:${CDP_PORT} 空闲` : `127.0.0.1:${CDP_PORT} 被占用`,
  portFree ? undefined : '关闭占用 10222 的进程（dev 调试会话通常持有该端口）',
);

// ---- 汇总 ----
const failed = results.filter((r) => !r.ok);
for (const r of results) {
  const mark = r.ok ? '✓' : '✗';
  console.log(`${mark} ${r.name}：${r.detail}`);
  if (!r.ok && r.fix) console.log(`  修复：${r.fix}`);
}
console.log(
  failed.length === 0
    ? '\ne2e 环境就绪（playwright + moli）。'
    : `\n${failed.length} 项未通过，请按修复指引处理后重试。`,
);
process.exit(failed.length === 0 ? 0 : 1);
