/**
 * CDP 探针 · Laya Reflex Tetris 真机验证
 *
 * 前置:pnpm dev(vite 5190 + debug 二进制 CDP 10222)、laya-server 10230。
 * 流程:进入「游戏」→ 打开 Laya Reflex Tetris → 开始 → 等 8s →
 * 断言决策日志出现 EXEC 行 → 截图到 .tmp/。
 */

import { spawn, type ChildProcess } from 'node:child_process';
import { chromium } from '@playwright/test';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs';

const CDP_PORT = 10222;
const TAURI_BIN = path.resolve(import.meta.dirname, '..', 'src-tauri', 'target', 'debug', 'qianxun.exe');

function log(msg: string): void {
  console.log(`[tetris-e2e] ${msg}`);
}
function fail(msg: string): never {
  console.error(`[tetris-e2e] ✗ ${msg}`);
  process.exit(1);
}

async function main(): Promise<void> {
  const userDataDir = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), 'qx-tetris-')));
  let viteProc: ChildProcess | undefined;
  const viteUp = await fetch('http://localhost:5190/').then((r) => r.ok).catch(() => false);
  if (!viteUp) {
    log('启动 vite…');
    viteProc = spawn('pnpm.cmd', ['dev:web'], {
      cwd: path.resolve(import.meta.dirname, '..'),
      stdio: 'ignore',
      shell: process.platform === 'win32',
    });
    for (let i = 0; i < 60; i += 1) {
      if (await fetch('http://localhost:5190/').then((r) => r.ok).catch(() => false)) break;
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
  process.on('exit', () => {
    killTree(proc);
    killTree(viteProc);
  });

  for (let i = 0; i < 120; i += 1) {
    if (await fetch(`http://127.0.0.1:${CDP_PORT}/json/version`).then((r) => r.ok).catch(() => false)) break;
    await new Promise((r) => setTimeout(r, 500));
  }
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
  const main = browser.contexts()[0]?.pages()[0];
  if (!main) fail('无 page');
  await main.waitForFunction(() => Boolean((window as unknown as { __qx?: unknown }).__qx), undefined, { timeout: 30_000 });

  // 进入游戏页
  await main.locator('[data-testid="nav-games"]').click({ timeout: 10_000 });
  log('clicked nav 游戏');
  await main.locator('text=Laya Reflex · 俄罗斯方块').first().click();
  log('opened tetris lab');
  await new Promise((r) => setTimeout(r, 1000));

  // 开始(Laya 反射默认模式)
  await main.locator('button:has-text("开始")').first().click();
  log('clicked 开始');

  // 等决策日志出现 EXEC 行(最多 30s)
  let execLines = 0;
  for (let t = 0; t < 30; t += 1) {
    execLines = await main
      .evaluate(() => (document.body.innerText.match(/EXEC \w+ conf=/g) ?? []).length)
      .catch(() => 0);
    if (execLines >= 3) break;
    await new Promise((r) => setTimeout(r, 1000));
  }
  await main.screenshot({ path: path.resolve(import.meta.dirname, '..', '.tmp', 'tetris-e2e.png'), fullPage: false });
  const bodyTail = await main.evaluate(() => document.body.innerText.slice(-900).replace(/\s+/g, ' '));
  log(`EXEC 决策行数: ${execLines}`);
  log(`页面尾部: ${bodyTail.slice(0, 500)}`);
  if (execLines >= 3) {
    log('✓ Laya Tetris 端到端验证通过(截图 .tmp/tetris-e2e.png)');
    process.exit(0);
  }
  fail('决策日志未出现 EXEC 行');
}

void main();
