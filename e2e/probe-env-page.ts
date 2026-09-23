/**
 * 环境页真机验证:进「环境」页 → Laya 状态探测(10230 外部实例应显示运行中)
 * → 截图(暗色)。前置:laya-server 已在 10230(或不在,验证"已停止"态)。
 */
import { spawn, type ChildProcess } from 'node:child_process';
import { chromium } from '@playwright/test';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs';

const CDP_PORT = 10222;
const ROOT = path.resolve(import.meta.dirname, '..');
const TAURI_BIN = path.join(ROOT, 'src-tauri', 'target', 'debug', 'qianxun.exe');

function log(msg: string): void {
  console.log(`[env-e2e] ${msg}`);
}

async function main(): Promise<void> {
  const userDataDir = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), 'qx-env-')));
  let viteProc: ChildProcess | undefined;
  const viteUp = await fetch('http://localhost:5190/')
    .then((r) => r.ok)
    .catch(() => false);
  if (!viteUp) {
    log('启动 vite…');
    viteProc = spawn(process.execPath, ['node_modules/vite/bin/vite.js'], {
      cwd: ROOT,
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
      // 裸 cargo build 的 debug exe 不合并 tauri.debug.conf.json（那是
      // tauri-cli 的工作），CDP 端口由此处 env 提供；隔离 user-data-dir
      // 防止挂到常驻 release 实例的 browser 进程上。
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
      WEBVIEW2_USER_DATA_FOLDER: userDataDir,
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

  for (let i = 0; i < 60; i += 1) {
    if (
      await fetch(`http://127.0.0.1:${CDP_PORT}/json/version`)
        .then((r) => r.ok)
        .catch(() => false)
    )
      break;
    await new Promise((r) => setTimeout(r, 500));
  }
  // 诊断:CDP HTTP 端点的原始返回(定位 ws 检索失败原因)
  const raw = await fetch(`http://127.0.0.1:${CDP_PORT}/json/version`)
    .then((r) => r.text())
    .catch((e) => `FETCH FAIL: ${e}`);
  log(`/json/version → ${String(raw).slice(0, 400)}`);
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
  const main = browser.contexts()[0]?.pages()[0];
  if (!main) throw new Error('无 page');
  await main.waitForFunction(
    () => Boolean((window as unknown as { __qx?: unknown }).__qx),
    undefined,
    {
      timeout: 30_000,
    },
  );

  // 环境页是默认首页;等 Laya 状态行出现(说明 laya_status 已回)
  await main.waitForSelector('[data-testid="laya-line"]', { timeout: 20_000 });
  await new Promise((r) => setTimeout(r, 1200));
  const layaLine = await main.locator('[data-testid="laya-line"]').innerText();
  const qwenLine = await main.locator('[data-testid="qwen-line"]').innerText();
  const moliLine = await main.locator('[data-testid="moli-line"]').innerText();
  log(`laya: ${layaLine}`);
  log(`qwen: ${qwenLine}`);
  log(`moli: ${moliLine}`);

  fs.mkdirSync(path.join(ROOT, '.tmp'), { recursive: true });
  await main.screenshot({ path: path.join(ROOT, '.tmp', 'env-page-dark.png') });

  // 配置展开交互验证
  await main.locator('button', { hasText: '配置' }).first().click();
  await new Promise((r) => setTimeout(r, 300));
  await main.screenshot({ path: path.join(ROOT, '.tmp', 'env-page-config.png') });

  log('✓ 环境页验证完成(截图 .tmp/env-page-dark.png / env-page-config.png)');
  process.exit(0);
}

void main();
