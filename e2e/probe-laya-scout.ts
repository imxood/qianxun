/**
 * CDP 探针 · Laya 集成侦察(docs/09 §6 e2e 前置)
 *
 * 用法:npx tsx e2e/probe-laya-scout.ts
 *
 * 行为:
 *   1. spawn debug 千寻(默认数据目录 .qianxun_dev,与 release 隔离)
 *   2. attach CDP 10222
 *   3. harness_environment / harness_status —— 确认 DSH 运行时与启动状态
 *   4. 枚举 pages/frames,定位 DSH 会话输入框(textarea/contenteditable)
 *
 * 不改任何状态;为 probe-laya-decision.ts 提供依据。
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
  console.log(`[scout] ${msg}`);
}
function fail(msg: string): never {
  console.error(`[scout] ✗ ${msg}`);
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
    fs.mkdtempSync(path.join(os.tmpdir(), 'qx-scout-webview2-')),
  );

  log('spawn debug binary(默认数据目录,共享 .qianxun_dev)');
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
    const pages = context.pages();
    log(`pages: ${pages.length}`);
    const main = pages.find((p) => !p.url().includes('#/standalone')) ?? pages[0];
    if (!main) fail('无 page');
    log(`main page: ${main.url()}`);

    // 等前端就绪(__qx 注入)
    await main.waitForFunction(() => {
      const w = window as unknown as { __qx?: unknown };
      return Boolean(w.__qx);
    }, undefined, { timeout: 30_000 });
    log('__qx 就绪');

    const env = await qx<Record<string, unknown>>(main, 'harness_environment', {});
    log(`harness_environment: ${JSON.stringify(env).slice(0, 400)}`);
    try {
      const status = await qx<Record<string, unknown>>(main, 'harness_status', {});
      log(`harness_status: ${JSON.stringify(status).slice(0, 300)}`);
    } catch (e) {
      log(`harness_status 失败:${e instanceof Error ? e.message : String(e)}`);
    }

    // DSH GUI DOM 侦察
    for (const frame of main.frames()) {
      const editors = await frame.evaluate(() => {
        const nodes = Array.from(
          document.querySelectorAll('textarea, [contenteditable="true"], [role="textbox"]'),
        );
        return nodes.slice(0, 5).map((n) => ({
          tag: n.tagName.toLowerCase(),
          placeholder: (n as HTMLTextAreaElement).placeholder ?? '',
          cls: (n as HTMLElement).className?.toString().slice(0, 80) ?? '',
        }));
      });
      log(`frame url=${frame.url().slice(0, 90)} textbox=${editors.length}`);
      for (const e of editors) log(`  · <${e.tag}> placeholder="${e.placeholder.slice(0, 40)}" cls="${e.cls}"`);
    }

    log('✓ 侦察完成');
    process.exit(0);
  } catch (err) {
    fail(`unexpected: ${err instanceof Error ? err.message : String(err)}`);
  }
}

void main();
