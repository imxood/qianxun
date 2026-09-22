/**
 * CDP 探针 · Laya 集成环境搭建(docs/09 §6 e2e 前置)
 *
 * 用法:npx tsx e2e/probe-laya-setup.ts
 *
 * 行为(对 dev 数据目录 .qianxun_dev):
 *   1. harness_install —— 安装/升级匹配版本的 DSH runtime(耗时:分钟级)
 *   2. harness_start   —— 启动 DSH
 *   3. 轮询 harness_status 直到 running/ready
 *   4. 重新枚举 DOM,定位 DSH 会话输入框
 */

import { spawn, type ChildProcess } from 'node:child_process';
import { chromium, type Browser, type Page } from '@playwright/test';
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
  console.log(`[setup] ${msg}`);
}
function fail(msg: string): never {
  console.error(`[setup] ✗ ${msg}`);
  process.exit(1);
}

async function waitForCdp(port: number, timeoutMs: number): Promise<void> {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/version`);
      if (res.ok) return;
    } catch {
      /* not ready */
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  fail(`CDP ${port} 在 ${timeoutMs}ms 未就绪`);
}

async function qx<T>(page: Page, cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  return page.evaluate(
    async ([cmd, args]) => {
      const w = window as unknown as {
        __qx: { call: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T> };
      };
      return w.__qx.call<T>(cmd, args);
    },
    [cmd, args] as const,
  );
}

async function main(): Promise<void> {
  if (!fs.existsSync(TAURI_BIN)) fail(`找不到 debug 构建:${TAURI_BIN}`);
  const userDataDir = fs.realpathSync.native(
    fs.mkdtempSync(path.join(os.tmpdir(), 'qx-setup-webview2-')),
  );

  log('spawn debug binary');
  const proc: ChildProcess = spawn(TAURI_BIN, [], {
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
      WEBVIEW2_USER_DATA_FOLDER: userDataDir,
      RUST_LOG: 'info',
    },
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  proc.on('error', (err) => fail(`spawn 失败:${err.message}`));
  const cleanup = (): void => {
    try {
      proc.kill();
    } catch {
      /* ignore */
    }
  };
  process.on('exit', cleanup);

  try {
    await waitForCdp(CDP_PORT, 60_000);
    const browser: Browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
    const context = browser.contexts()[0];
    if (!context) fail('无 CDP context');
    const main = context.pages().find((p) => !p.url().includes('#/standalone')) ?? context.pages()[0];
    if (!main) fail('无 page');
    await main.waitForFunction(() => {
      const w = window as unknown as { __qx?: unknown };
      return Boolean(w.__qx);
    }, undefined, { timeout: 30_000 });
    log('__qx 就绪');

    let env = await qx<Record<string, unknown>>(main, 'harness_environment', {});
    log(`dshVersion=${env['dshVersion']} matches=${env['dshVersionMatches']}`);
    if (!env['dshVersionMatches']) {
      log('harness_install 开始(分钟级,请耐心)…');
      await qx<unknown>(main, 'harness_install', {});
      env = await qx<Record<string, unknown>>(main, 'harness_environment', {});
      log(`install 后 dshVersion=${env['dshVersion']} matches=${env['dshVersionMatches']}`);
      if (!env['dshVersionMatches']) fail('安装后版本仍不匹配');
    }

    log('harness_start …');
    const origin = await qx<string>(main, 'harness_start', {});
    log(`harness_start → ${origin}`);

    // 轮询状态
    for (let i = 0; i < 60; i += 1) {
      const status = (await qx<Record<string, unknown>>(main, 'harness_status', {})) as {
        phase?: string;
      };
      log(`status.phase = ${status.phase}`);
      if (status.phase === 'running' || status.phase === 'ready' || status.phase === 'healthy') break;
      if (status.phase === 'failed') fail(`DSH 启动失败:${JSON.stringify(status)}`);
      await new Promise((r) => setTimeout(r, 2000));
    }

    // DOM 侦察
    for (const frame of main.frames()) {
      const editors = await frame.evaluate(() => {
        const nodes = Array.from(
          document.querySelectorAll('textarea, [contenteditable="true"], [role="textbox"]'),
        );
        return nodes.slice(0, 5).map((n) => ({
          tag: n.tagName.toLowerCase(),
          placeholder: (n as HTMLTextAreaElement).placeholder ?? '',
        }));
      });
      log(`frame url=${frame.url().slice(0, 90)} textbox=${editors.length}`);
      for (const e of editors) log(`  · <${e.tag}> placeholder="${e.placeholder.slice(0, 50)}"`);
    }

    log('✓ setup 完成(DSH 已运行)');
    process.exit(0);
  } catch (err) {
    fail(`unexpected: ${err instanceof Error ? err.message : String(err)}`);
  }
}

void main();
