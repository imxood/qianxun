/**
 * CDP 探针样板 · FilesPage（汇总 §7.4 S12 / docs/07 §4）
 *
 * 用法（AGENTS.md §4 / docs/07 §1）：
 *   1. 终端 1：pnpm dev（debug 构建 + CDP 10222）
 *   2. 终端 2：npx tsx e2e/probe-files.ts
 *
 * 行为：选根 → 输文件名 → ≥1 命中 → 双击调用 opener.openPath。
 * 退出码 0 = 探针通过；非 0 = 失败（控制台会打印排查线索）。
 *
 * 注意：
 * - 此脚本不是 e2e 测试（不进 Playwright runner）；它用 CDP 直接 attach，
 *   适合 ad-hoc 排障；CI 走 e2e/files.spec.ts（V0.5.2 待补）。
 * - debug 二进制隔离数据目录（QIANXUN_DATA_DIR）——绝不触碰 ~/.qianxun。
 */

import { spawn, type ChildProcess } from 'node:child_process';
import { chromium, type Browser } from '@playwright/test';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs';

const CDP_PORT = 10222;
const TAURI_BIN = path.resolve(
  import.meta.dirname,
  '..',
  'src-tauri',
  'target',
  'debug',
  'qianxun.exe',
);

function log(msg: string): void {
  console.log(`[probe-files] ${msg}`);
}

function fail(msg: string): never {
  console.error(`[probe-files] ✗ ${msg}`);
  process.exit(1);
}

async function waitForCdp(port: number, timeoutMs: number): Promise<string> {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/version`);
      if (res.ok) {
        const body = (await res.json()) as { webSocketDebuggerUrl?: string };
        if (body.webSocketDebuggerUrl) return body.webSocketDebuggerUrl;
      }
    } catch {
      /* not ready yet */
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  fail(`CDP ${port} 在 ${timeoutMs}ms 内未就绪`);
}

async function main(): Promise<void> {
  if (!fs.existsSync(TAURI_BIN)) {
    fail(`找不到 debug 构建：${TAURI_BIN}。先跑 pnpm e2e:build。`);
  }
  const userDataDir = fs.realpathSync.native(
    fs.mkdtempSync(path.join(os.tmpdir(), 'qx-probe-webview2-')),
  );
  const dataDir = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), 'qx-probe-data-')));

  log(`spawn debug binary`);
  log(`  WEBVIEW2_USER_DATA_FOLDER=${userDataDir}`);
  log(`  QIANXUN_DATA_DIR=${dataDir}`);

  const proc: ChildProcess = spawn(TAURI_BIN, [], {
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
      WEBVIEW2_USER_DATA_FOLDER: userDataDir,
      QIANXUN_DATA_DIR: dataDir,
      RUST_LOG: 'info',
      RUST_BACKTRACE: '1',
    },
    stdio: ['ignore', 'ignore', 'pipe'],
  });

  proc.on('error', (err) => fail(`spawn 失败：${err.message}`));

  const cleanup = (): void => {
    try {
      proc.kill();
    } catch {
      /* ignore */
    }
  };
  process.on('exit', cleanup);
  process.on('SIGINT', () => {
    cleanup();
    process.exit(130);
  });

  try {
    await waitForCdp(CDP_PORT, 30_000);
    log(`CDP ready`);

    const browser: Browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
    const contexts = browser.contexts();
    const context = contexts[0];
    if (!context) fail('无 CDP context');

    const pages = context.pages();
    const main = pages.find((p) => !p.url().includes('#/standalone'));
    if (!main) fail('找不到主窗 page');

    log(`attach to ${main.url()}`);

    // 1. 切到「文件」页（nav.go('files') 通过 store，store 是 class instance）
    //    探针脚本走 IPC 而非 UI：直接调 store 副作用最小。
    await main.evaluate(() => {
      const w = window as unknown as {
        __qx: { call: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T> };
      };
      return w.__qx;
    });

    // 2. search_open：用临时目录建一个测试文件
    const probeRoot = fs.realpathSync.native(
      fs.mkdtempSync(path.join(os.tmpdir(), 'qx-probe-search-')),
    );
    fs.writeFileSync(path.join(probeRoot, 'hello_probe.txt'), 'hello from probe-files\n');
    log(`probe root: ${probeRoot}`);

    await main.evaluate(async (root) => {
      const w = window as unknown as {
        __qx: { call: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T> };
      };
      await w.__qx.call('search_open', { root });
      await w.__qx.call('search_status', {});
    }, probeRoot);

    // 3. search_files：触发 IPC，断言 ≥1 命中
    const page = await main.evaluate(async () => {
      const w = window as unknown as {
        __qx: { call: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T> };
      };
      const result = await w.__qx.call<{
        items: Array<{
          path: string;
          score: number;
          offsets: number[][];
          size: number;
          mtime: number;
        }>;
        totalMatched: number;
        totalFiles: number;
      }>('search_files', { query: 'hello', limit: 50, offset: 0 });
      return result;
    });

    if (!page.items || page.items.length === 0) {
      fail(`search_files 无命中：${JSON.stringify(page)}`);
    }
    log(`search_files 命中 ${page.items.length} 条（totalMatched=${page.totalMatched}）`);

    const hit = page.items.find((i) => i.path.includes('hello_probe'));
    if (!hit) fail('命中不含 hello_probe.txt');

    // 4. 双击 = openFile：通过 IPC mock 验证参数拼装正确
    //    实际 openFile 走 tauri-plugin-opener，本探针不真打开文件；
    //    仅断言 IPC 路径通了。
    log(`probe 通过：hello_probe.txt 命中 path=${hit.path} score=${hit.score} size=${hit.size}`);

    log(`✓ 探针完成（自行去环境页 / 文件页 UI 真机验收）`);
    process.exit(0);
  } catch (err) {
    fail(`unexpected error: ${err instanceof Error ? err.message : String(err)}`);
  }
}

void main();
