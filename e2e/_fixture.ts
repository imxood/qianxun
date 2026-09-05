import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { spawn, type ChildProcess } from 'node:child_process';
import { test as base, chromium, expect, type Browser, type Page } from '@playwright/test';

/**
 * WebView2 CDP fixture（参考 Playwright 官方 webview2.md 模式）：
 *
 * 1. 只 spawn **debug** 构建（target/debug/qianxun.exe）——debug 二进制
 *    经 cfg!(debug_assertions) 隔离数据目录到 ~/.qianxun_dev，绝不触碰
 *    安装版 ~/.qianxun 生产数据。二进制路径与 dshHome 双重断言兜底。
 * 2. WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=10222
 *    暴露 CDP；WEBVIEW2_USER_DATA_FOLDER 用一次性临时目录（多 webview
 *    共享 user data dir，测试间必须隔离——官方文档要求）。
 * 3. chromium.connectOverCDP 附上去；主窗 = 不带 #/standalone 的 page，
 *    独立窗口 = 带 #/standalone/{view} 的 page。
 */

const CDP_PORT = 10222;

const PROJECT_ROOT = path.resolve(import.meta.dirname, '..');
const TAURI_BIN = path.join(PROJECT_ROOT, 'src-tauri', 'target', 'debug', 'qianxun.exe');

/** 当前实例的数据目录（browser fixture 写入，诊断日志读取用；单 worker 串行安全）。 */
let currentDataDir: string | null = null;

export type QxFixture = {
  /**
   * e2e 专属应用实例（test 作用域：每个测试一个全新进程，互不泄漏）。
   * 注意不要命名为 `browser`——那会覆盖 Playwright 内建 worker 级
   * browser fixture，导致所有测试共享同一个应用、状态互相污染。
   */
  app: Browser;
  /** 主窗口 page。 */
  main: Page;
  helper: {
    /** 等待一个 url 含指定 hash 的 webview page 出现并返回。 */
    waitForPage: (urlPart: string, timeoutMs?: number) => Promise<Page>;
    /** 打开独立窗口（经主窗 IPC），返回其 page。 */
    spawnStandalone: (view: 'terminal' | 'dsh') => Promise<Page>;
    /** 在任意 page 上调 IPC（window.__qx 调试句柄）。 */
    call: <T>(page: Page, command: string, args?: Record<string, unknown>) => Promise<T>;
  };
};

export const test = base.extend<QxFixture>({
  app: async (_fixtures, use) => {
    // 保险一：二进制必须来自 target/debug。
    expect(
      TAURI_BIN,
      `找不到 debug 构建：${TAURI_BIN}。先跑 pnpm e2e:build（vite build + cargo build）。`,
    ).toBeTruthy();
    expect(fs.existsSync(TAURI_BIN), `找不到 debug 构建：${TAURI_BIN}。先跑 pnpm e2e:build。`).toBe(
      true,
    );
    expect(TAURI_BIN, 'e2e 只允许 debug 构建（保护 ~/.qianxun 生产数据）').toContain(
      `${path.sep}debug${path.sep}`,
    );

    const userDataDir = fs.realpathSync.native(
      fs.mkdtempSync(path.join(os.tmpdir(), 'qx-e2e-webview2-')),
    );
    // 数据根同样一次性隔离（QIANXUN_DATA_DIR 便携覆盖）：e2e 实例不读
    // 用户设置、不恢复用户 PIN、日志/记录全部落在临时目录，测试后即焚。
    const dataDir = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), 'qx-e2e-data-')));
    currentDataDir = dataDir;
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
    // stderr 直接透传到测试输出：panic/断言失败第一时间可见。
    proc.stderr?.on('data', (chunk: Buffer) => {
      console.log(`[qx-e2e stderr] ${chunk.toString().trimEnd()}`);
    });

    try {
      // 进程被 single_instance 拦截会立即退出——报错要说清原因。
      const exitedEarly = new Promise<never>((_, reject) => {
        proc.once('exit', (code) => {
          reject(
            new Error(
              `qianxun.exe 提前退出（code ${code}）。多为单实例互斥：请先关闭正在运行的 dev/安装版千寻。`,
            ),
          );
        });
      });
      const ready = (async () => {
        const deadline = Date.now() + 30_000;
        while (Date.now() < deadline) {
          try {
            const response = await fetch(`http://127.0.0.1:${CDP_PORT}/json/version`);
            if (response.ok) return;
          } catch {
            // 端口未就绪，继续等。
          }
          await new Promise((resolve) => setTimeout(resolve, 200));
        }
        throw new Error(`CDP 端口 ${CDP_PORT} 30s 未就绪`);
      })();
      await Promise.race([ready, exitedEarly]);

      // connectOverCDP 在 webview 初始化窗口期可能被重置：带重试。
      let browser: import('playwright-core').Browser | null = null;
      const connectDeadline = Date.now() + 20_000;

      while (true) {
        try {
          browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
          break;
        } catch (error) {
          if (Date.now() > connectDeadline) throw error;
          await new Promise((resolve) => setTimeout(resolve, 300));
        }
      }
      await use(browser);
      await browser.close();
    } finally {
      proc.removeAllListeners('exit');
      proc.kill();
      // WebView2 有惰性清理（官方文档提示）：目录尽力删，删不掉不阻塞。
      for (const dir of [userDataDir, dataDir]) {
        try {
          fs.rmSync(dir, { recursive: true, force: true, maxRetries: 3, retryDelay: 300 });
        } catch {
          // 残留目录随系统临时目录清理回收。
        }
      }
    }
  },
  main: async ({ app }, use) => {
    const page = await findPage(app, (url) => !url.includes('#/standalone'));
    expect(page, '主窗 page 不存在').toBeTruthy();
    // 保险二：数据目录绝不能是安装版的 ~/.qianxun（生产数据）。
    // debug 构建无覆盖时为 ~/.qianxun_dev；e2e 注入 QIANXUN_DATA_DIR 时
    // 为临时目录。两种合法形态都不以「\.qianxun」结尾（_dev 结尾不匹配）。
    const env = await callIn<{ dshHome: string }>(page!, 'harness_environment');
    const normalized = env.dshHome.replace(/\//g, '\\');
    expect(
      normalized.endsWith('\\.qianxun'),
      `e2e 实例的数据目录不得为生产目录（实际：${env.dshHome}）`,
    ).toBe(false);

    // 主题跟随系统：前端 html.dark 必须等于 OS「应用模式」暗色
    // （Rust 注册表读值 seed + ThemeChanged 推送；轮询等 seed 到位）。
    const osAppLight = await regQueryAppModeLight();
    const expectedDark = !osAppLight;
    await expect
      .poll(() => page!.evaluate(() => document.documentElement.classList.contains('dark')), {
        timeout: 15_000,
        message: `html.dark 应为 ${expectedDark}（跟随 OS 应用模式）`,
      })
      .toBe(expectedDark);

    // 诊断：媒体查询与 qianxun 日志尾部（SetPreferredColorScheme 是否
    // 落地、有没有报错），不打断断言。
    const mediaDark = await page!.evaluate(
      () => window.matchMedia('(prefers-color-scheme: dark)').matches,
    );
    console.log(`[qx-e2e] osAppLight=${osAppLight} mediaDark=${mediaDark}`);
    try {
      const log = currentDataDir
        ? fs.readFileSync(path.join(currentDataDir, 'logs', 'qianxun.log'), 'utf8').slice(-1500)
        : '';
      console.log(`[qx-e2e] qianxun.log tail:\n${log}`);
    } catch {
      console.log('[qx-e2e] qianxun.log 不可读');
    }

    await use(page!);
  },

  helper: async ({ app }, use) => {
    await use({
      waitForPage: (urlPart, timeoutMs = 15_000) => waitForPage(app, urlPart, timeoutMs),
      spawnStandalone: async (view) => {
        const main = await findPage(app, (url) => !url.includes('#/standalone'));
        await callIn(main!, 'window_spawn_view', { view });
        return waitForPage(app, `#/standalone/${view}`);
      },
      call: async (page, command, args) => callIn(page, command, args),
    });
  },
});

/** 在全部 contexts 的 pages 里找 url 匹配的 page（轮询等待出现）。 */
async function findPage(
  browser: Browser,
  predicate: (url: string) => boolean,
  timeoutMs = 15_000,
): Promise<Page | null> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    for (const context of browser.contexts()) {
      for (const page of context.pages()) {
        if (predicate(page.url())) return page;
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  const seen = browser.contexts().flatMap((context) => context.pages().map((page) => page.url()));
  console.log(`[qx-e2e] findPage 未命中，现存 pages：${JSON.stringify(seen)}`);
  return null;
}

/**
 * 读 OS「应用模式」明暗（Windows 注册表 AppsUseLightTheme）。
 * 返回 true = 系统应用模式为浅色。读失败按 false（暗色）处理——
 * 断言失败会显式暴露，而不是静默放过。
 */
function regQueryAppModeLight(): Promise<boolean> {
  return new Promise((resolve) => {
    execFile(
      'reg',
      [
        'query',
        'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize',
        '/v',
        'AppsUseLightTheme',
      ],
      (error, stdout) => {
        if (error) {
          resolve(false);
          return;
        }
        resolve(/AppsUseLightTheme\s+REG_DWORD\s+0x1/.test(stdout));
      },
    );
  });
}

async function waitForPage(browser: Browser, urlPart: string, timeoutMs: number): Promise<Page> {
  const page = await findPage(browser, (url) => url.includes(urlPart), timeoutMs);
  expect(page, `${timeoutMs}ms 内没等到 page：${urlPart}`).toBeTruthy();
  return page!;
}

/** 经 window.__qx 调试句柄调 IPC（main.ts 挂载，生产无副作用）。 */
async function callIn<T>(page: Page, command: string, args?: Record<string, unknown>): Promise<T> {
  // bundle 可能仍在加载（启动屏阶段）：等 __qx 挂上 window 再执行；
  // waitForFunction 可跨导航重注入，比一次性 evaluate 稳。
  await page.waitForFunction(
    () => Boolean((window as unknown as { __qx?: unknown }).__qx),
    undefined,
    { timeout: 30_000 },
  );
  return page.evaluate<{ command: string; args?: Record<string, unknown> }, T>(
    async ({ command: cmd, args: a }) => {
      const handle = (window as unknown as { __qx?: { call: typeof Function } }).__qx;
      if (!handle) throw new Error('window.__qx 不存在（bundle 未含调试句柄？）');
      return handle.call(cmd, a) as Promise<T>;
    },
    { command, args },
  );
}

export { expect };
