/**
 * CDP 探针 · Laya Jump 马里奥 1-1 真机验证(docs/11 §9 M4)
 *
 * 前置:pnpm e2e:build(debug 二进制)+ vite(探针自动拉起)。
 * 默认自驾模式(autopilot,规划器直控)——不依赖 laya-server,CI 可重复;
 * reflex/pure 需 release 侧先启动 laya-server(:10230),游戏页只读探活、
 * 绝不代启(docs/11 §1.1)。
 *
 * 流程:进入「游戏」→ 打开 Laya Jump · 1-1 → 切自驾 → 开始 →
 * 追踪马里奥进度点位移 → 断言在推进 → 截图到 .tmp/mario-e2e.png。
 */

import { spawn, type ChildProcess } from 'node:child_process';
import { chromium } from '@playwright/test';
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
/** 探针驱动模式:autopilot(默认,不依赖 laya)/ reflex / pure(MARIO_PROBE_MODE 覆盖)。 */
const MODE = (process.env.MARIO_PROBE_MODE ?? 'autopilot') as 'autopilot' | 'reflex' | 'pure';

function log(msg: string): void {
  console.log(`[mario-e2e] ${msg}`);
}
function fail(msg: string): never {
  console.error(`[mario-e2e] ✗ ${msg}`);
  process.exit(1);
}

async function main(): Promise<void> {
  const userDataDir = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), 'qx-mario-')));
  let viteProc: ChildProcess | undefined;
  const viteUp = await fetch('http://localhost:5190/')
    .then((r) => r.ok)
    .catch(() => false);
  if (!viteUp) {
    log('启动 vite…');
    viteProc = spawn(process.execPath, ['node_modules/vite/bin/vite.js'], {
      cwd: path.resolve(import.meta.dirname, '..'),
      stdio: 'ignore',
    });
    for (let i = 0; i < 60; i += 1) {
      if (
        await fetch('http://localhost:5190/')
          .then((r) => r.ok)
          .catch(() => false)
      )
        break;
      await new Promise((r) => setTimeout(r, 500));
    }
  }
  const proc: ChildProcess = spawn(TAURI_BIN, [], {
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
      WEBVIEW2_USER_DATA_FOLDER: userDataDir,
      RUST_LOG: 'info',
    },
    stdio: 'ignore',
  });
  const killTree = (child?: ChildProcess): void => {
    if (!child?.pid) return;
    if (process.platform === 'win32') {
      spawn('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
    } else child.kill('SIGTERM');
  };
  const cleanup = (): void => {
    killTree(proc);
    killTree(viteProc);
  };
  process.on('exit', cleanup);
  process.on('SIGINT', () => {
    cleanup();
    process.exit(130);
  });
  process.on('SIGTERM', () => {
    cleanup();
    process.exit(143);
  });

  for (let i = 0; i < 120; i += 1) {
    if (
      await fetch(`http://127.0.0.1:${CDP_PORT}/json/version`)
        .then((r) => r.ok)
        .catch(() => false)
    )
      break;
    await new Promise((r) => setTimeout(r, 500));
  }
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
  const main = browser.contexts()[0]?.pages()[0];
  if (!main) fail('无 page');
  await main.waitForFunction(
    () => Boolean((window as unknown as { __qx?: unknown }).__qx),
    undefined,
    { timeout: 30_000 },
  );

  // 进入游戏页 → 打开马里奥
  await main.locator('[data-testid="nav-games"]').click({ timeout: 10_000 });
  log('clicked nav 游戏');
  await main.locator('text=Laya Jump · 超级马里奥 1-1').first().click();
  log('opened mario lab');
  await main.locator('[data-testid="mario-canvas"]').waitFor({ timeout: 10_000 });

  // 切模式 + 开始
  await main.locator(`[data-testid="mode-${MODE}"]`).click();
  await main.locator('[data-testid="btn-start"]').click();
  log(`${MODE} started`);

  // 追踪进度点位移(20s 内必须稳定推进;进度条用 calc% 定位,读 rect 而非 style)
  const readLeft = (): Promise<number> =>
    main.evaluate(() => {
      const el = document.querySelector('[data-testid="mario-dot"]');
      if (!el) return -1;
      return el.getBoundingClientRect().left;
    });
  const l0 = await readLeft();
  for (let t = 0; t < 20; t += 1) {
    await new Promise((r) => setTimeout(r, 1000));
    const l = await readLeft();
    if (MODE !== 'autopilot') {
      // reflex/pure:断言 Laya 决策确实在跑(遥测条统计"决策 N"且 EXEC 行出现)
      const stats = await main
        .evaluate(() => {
          const el = document.querySelector('[data-testid="telem-stats"]');
          return el ? (el.textContent ?? '') : '';
        })
        .catch(() => '');
      const m = stats.match(/决策\s*(\d+)/);
      const decisions = m ? Number(m[1]) : 0;
      if (decisions >= 3) {
        await main.screenshot({
          path: path.resolve(import.meta.dirname, '..', '.tmp', 'mario-e2e.png'),
          fullPage: false,
        });
        log(`✓ Laya 决策在跑:stats="${stats.trim().slice(0, 120)}"(截图 .tmp/mario-e2e.png)`);
        process.exit(0);
      }
    }
    if (l >= l0 + 30) {
      await main.screenshot({
        path: path.resolve(import.meta.dirname, '..', '.tmp', 'mario-e2e.png'),
        fullPage: false,
      });
      log(`✓ 马里奥在推进:${l0.toFixed(0)}px → ${l.toFixed(0)}px(截图 .tmp/mario-e2e.png)`);
      process.exit(0);
    }
  }
  await main.screenshot({
    path: path.resolve(import.meta.dirname, '..', '.tmp', 'mario-e2e-stuck.png'),
    fullPage: false,
  });
  fail(`马里奥 10s 未推进(left=${l0.toFixed(1)}%,截图 .tmp/mario-e2e-stuck.png)`);
}

void main();
