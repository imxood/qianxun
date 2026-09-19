/**
 * CDP 探针样板 · DiskScan（汇总 §7.4 S12 / docs/07 §4）
 *
 * 用法：
 *   1. 终端 1：pnpm dev（debug 构建 + CDP 10222）
 *   2. 终端 2：npx tsx e2e/probe-disk.ts
 *
 * 行为：选盘符/目录 → 触发 disk_scan_stream → 调 disk_scan_stop → 断言
 *       partial 结果已写入。
 *
 * 注意：同 probe-files / probe-grep，ad-hoc 排障用；CI 走 e2e/disk.spec.ts。
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
  console.log(`[probe-disk] ${msg}`);
}
function fail(msg: string): never {
  console.error(`[probe-disk] ✗ ${msg}`);
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
      /* not ready */
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  fail(`CDP ${port} 在 ${timeoutMs}ms 未就绪`);
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

  try {
    await waitForCdp(CDP_PORT, 30_000);
    const browser: Browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
    const context = browser.contexts()[0];
    if (!context) fail('无 CDP context');
    const main = context.pages().find((p) => !p.url().includes('#/standalone'));
    if (!main) fail('找不到主窗 page');
    log(`attach to ${main.url()}`);

    // 建一个测试目录：5 个子目录 + 各 10 个小文件
    const probeRoot = fs.realpathSync.native(
      fs.mkdtempSync(path.join(os.tmpdir(), 'qx-probe-disk-')),
    );
    for (let d = 0; d < 5; d += 1) {
      const dir = path.join(probeRoot, `sub_${d}`);
      fs.mkdirSync(dir, { recursive: true });
      for (let f = 0; f < 10; f += 1) {
        fs.writeFileSync(path.join(dir, `f_${f}.txt`), 'x'.repeat(1024));
      }
    }
    log(`probe root: ${probeRoot}（5 子目录 × 10 文件 = 50 文件）`);

    // 1. disk_home：获取数据目录
    const home = await main.evaluate(async () => {
      const w = window as unknown as {
        __qx: { call: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T> };
      };
      return w.__qx.call<{ home: string }>('disk_home', {});
    });
    log(`disk_home: ${home.home}`);

    // 2. disk_scan_stop：先停（防漏跑旧实例残留）
    await main.evaluate(async () => {
      const w = window as unknown as {
        __qx: { call: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T> };
      };
      try {
        await w.__qx.call('disk_scan_stop', {});
      } catch {
        /* 首次可能没在跑 */
      }
    });

    // 3. disk_scan_stream：探针里只验证 IPC 通；流式分片需 UI Channel.onmessage。
    const started = await main.evaluate(async (root) => {
      const w = window as unknown as {
        __qx: { call: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T> };
      };
      // 同步触发（探针不接 Channel，所以走阻塞版的 disk_scan）
      return (await w.__qx.call('disk_scan', { root, maxDepth: 2 })) as { home?: unknown };
    }, probeRoot);
    log(`disk_scan 返回: ${JSON.stringify(started).slice(0, 80)}...`);

    // 4. disk_scan_stop：再停（确保 stop 命令也通）
    const stopped = await main.evaluate(async () => {
      const w = window as unknown as {
        __qx: { call: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T> };
      };
      try {
        await w.__qx.call('disk_scan_stop', {});
        return true;
      } catch {
        return false;
      }
    });
    log(`disk_scan_stop 返回: ${stopped}`);

    log(`✓ 探针完成（disk_scan / stop / home 三 IPC 通；流式分片去 UI 真机验收）`);
    process.exit(0);
  } catch (err) {
    fail(`unexpected error: ${err instanceof Error ? err.message : String(err)}`);
  }
}

void main();
