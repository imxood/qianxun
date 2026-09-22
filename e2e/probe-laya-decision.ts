/**
 * CDP 探针 · decision-laya 端到端验证(docs/09 §6,需求 3)
 *
 * 用法:npx tsx e2e/probe-laya-decision.ts [--times 10]
 *
 * 前置(一次性):
 *   - models/laya-multilingual-onnx 已就位
 *   - laya-server 已在 127.0.0.1:10230 运行(multilingual bundle)
 *   - dev profile(.qianxun_dev)已安装 decision-laya(Junction + cordis.patch.yml)
 *
 * 流程:spawn debug 千寻 → harness_start → 进入 DSH 视图 → 向 DSH 会话发送
 * N 条中文工单指令(要求调用 laya_decide)→ 断言每次工具结果出现且含预期
 * decision 字段 → 汇总 ≥N 次验证通过。
 */

import { spawn, type ChildProcess } from 'node:child_process';
import { chromium, type Browser, type Frame, type Page } from '@playwright/test';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs';

const CDP_PORT = 10222;
const TAURI_BIN = path.resolve(import.meta.dirname, '..', 'src-tauri', 'target', 'debug', 'qianxun.exe');
const TIMES = Number(process.argv.includes('--times') ? process.argv[process.argv.indexOf('--times') + 1] : 10);

function log(msg: string): void {
  console.log(`[laya-e2e] ${msg}`);
}
function fail(msg: string): never {
  console.error(`[laya-e2e] ✗ ${msg}`);
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
  fail(`CDP ${port} 未就绪`);
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

/** 10 个中文验证用例:state + 期望 department 选择 */
const CASES: Array<{ name: string; state: unknown; expectChoice: string }> = [
  { name: '重复扣款退款', state: { ticket: '发票被重复扣款了,请立刻退款,否则投诉', channel: 'email' }, expectChoice: 'billing' },
  { name: '登录 500', state: { ticket: '系统登录一直报 500 错误,所有人都用不了', channel: 'web' }, expectChoice: 'technical' },
  { name: '企业版报价', state: { ticket: '想了解企业版的报价和合同签订流程', channel: 'phone' }, expectChoice: 'sales' },
  { name: '余额发票', state: { ticket: '上个月的发票还没开给我,麻烦补开发票', channel: 'email' }, expectChoice: 'billing' },
  { name: '接口报错', state: { ticket: '调用导出接口报 502,业务停摆,急需处理', channel: 'web' }, expectChoice: 'technical' },
  { name: '购买意向', state: { ticket: '我们公司想采购 200 个账号,给我一份报价单', channel: 'phone' }, expectChoice: 'sales' },
  { name: '重复扣款电话', state: { ticket: '你们是不是多扣了我一笔钱?要求退回', channel: 'phone' }, expectChoice: 'billing' },
  { name: '白屏故障', state: { ticket: '打开就白屏,缓存清了也没用,生产环境受阻', channel: 'web' }, expectChoice: 'technical' },
  { name: '续费折扣', state: { ticket: '准备续费,问问有没有折扣和合同方案', channel: 'email' }, expectChoice: 'sales' },
  { name: '退款到账', state: { ticket: '之前申请的退款到现在还没到账,帮我查一下', channel: 'email' }, expectChoice: 'billing' },
];

const QUESTIONS_JSON = JSON.stringify({
  department: {
    type: 'choice',
    instructions: 'Which department should handle this ticket?',
    criteria: {
      billing: '发票、扣款、退款、账单',
      technical: '故障、报错、系统问题',
      sales: '报价、购买、续费、合同',
    },
  },
});

function promptFor(c: { name: string; state: unknown }): string {
  return [
    `请调用 laya_decide 工具(不要用其他工具,不要自己回答)处理以下工单:`,
    `state = ${JSON.stringify(c.state)}`,
    `questions = ${QUESTIONS_JSON}`,
    `调用完成后,仅回复一行:部门=<choice 值>,置信度=<confidence>。`,
  ].join('\n');
}

async function findChatFrame(page: Page): Promise<Frame | undefined> {
  for (let i = 0; i < 40; i += 1) {
    for (const frame of page.frames()) {
      const has = await frame
        .evaluate(() => {
          const q = document.querySelector('textarea, [contenteditable="true"], [role="textbox"]');
          return Boolean(q);
        })
        .catch(() => false);
      if (has) return frame;
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  return undefined;
}

async function dumpNav(page: Page): Promise<void> {
  const texts = await page
    .evaluate(() => {
      const els = Array.from(document.querySelectorAll('button, a, [role="tab"], [role="button"], [role="menuitem"]'));
      return els
        .map((e) => (e.textContent ?? '').trim().replace(/\s+/g, ' '))
        .filter((t) => t.length > 0 && t.length < 24)
        .slice(0, 40);
    })
    .catch(() => [] as string[]);
  log(`nav candidates: ${JSON.stringify(texts)}`);
}

async function main(): Promise<void> {
  if (!fs.existsSync(TAURI_BIN)) fail(`找不到 debug 构建:${TAURI_BIN}`);
  const userDataDir = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), 'qx-laya-e2e-')));

  // debug 二进制的 UI 来自 vite dev server(5190);不在 pnpm dev 下运行时需自带
  let viteProc: ChildProcess | undefined;
  const viteUp = await fetch('http://localhost:5190/')
    .then((r) => r.ok)
    .catch(() => false);
  if (!viteUp) {
    log('启动 vite dev:web(5190)…');
    viteProc = spawn(process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm', ['dev:web'], {
      cwd: path.resolve(import.meta.dirname, '..'),
      env: process.env,
      stdio: ['ignore', 'ignore', 'pipe'],
      shell: process.platform === 'win32',
    });
    const started = Date.now();
    while (Date.now() - started < 60_000) {
      const up = await fetch('http://localhost:5190/')
        .then((r) => r.ok)
        .catch(() => false);
      if (up) break;
      await new Promise((r) => setTimeout(r, 500));
    }
    log('vite 就绪');
  }

  log(`spawn debug binary(验证 ${TIMES} 次)`);
  const proc: ChildProcess = spawn(TAURI_BIN, [], {
    env: {
      ...process.env,
      HTTP_PROXY: '',
      HTTPS_PROXY: '',
      http_proxy: '',
      https_proxy: '',
      ALL_PROXY: '',
      NO_PROXY: '127.0.0.1,localhost',
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
      WEBVIEW2_USER_DATA_FOLDER: userDataDir,
      RUST_LOG: 'info',
    },
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  // Windows 上 shell:true 会有 cmd→node 中间层,kill() 只杀直接子进程;
  // 用 taskkill /T 连树杀(仅限探针自己 spawn 的 PID,绝不碰其他进程)
  const killTree = (child: ChildProcess | undefined): void => {
    if (!child?.pid) return;
    try {
      if (process.platform === 'win32') {
        spawn('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
      } else {
        child.kill('SIGTERM');
      }
    } catch {
      /* ignore */
    }
  };
  const cleanup = (): void => {
    killTree(proc);
    killTree(viteProc);
  };
  process.on('exit', cleanup);

  try {
    await waitForCdp(CDP_PORT, 60_000);
    const browser: Browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
    const context = browser.contexts()[0];
    if (!context) fail('无 CDP context');
    const main = context.pages().find((p) => !p.url().includes('#/standalone')) ?? context.pages()[0];
    if (!main) fail('无 page');
    await main.waitForFunction(() => Boolean((window as unknown as { __qx?: unknown }).__qx), undefined, {
      timeout: 30_000,
    });

    // DSH 启动
    let status = (await qx<Record<string, unknown>>(main, 'harness_status', {})) as { phase?: string };
    if (status.phase !== 'running' && status.phase !== 'ready' && status.phase !== 'healthy') {
      const origin = await qx<string>(main, 'harness_start', {});
      log(`harness_start → ${origin}`);
    }
    for (let i = 0; i < 60; i += 1) {
      status = (await qx<Record<string, unknown>>(main, 'harness_status', {})) as { phase?: string };
      if (['running', 'ready', 'healthy'].includes(status.phase ?? '')) break;
      await new Promise((r) => setTimeout(r, 2000));
    }
    log(`harness_status = ${(status as { phase?: string }).phase}`);

    // 侧车健康检查(不是 DSH 的一部分,但工具依赖它)
    try {
      const r = await fetch('http://127.0.0.1:10230/health', { signal: AbortSignal.timeout(8000) });
      const j = (await r.json()) as { ok?: boolean };
      if (!j.ok) fail(`laya-server 响应异常:${JSON.stringify(j)}`);
      log('laya-server 健康');
    } catch (e) {
      fail(
        `laya-server 不可达:${e instanceof Error ? `${e.message} / cause=${String((e as { cause?: unknown }).cause ?? '-')}` : String(e)}`,
      );
    }

    // DSH origin:优先 harness_proxy_url,退而在 frames 中找非 5190 的 http 源
    let dshOrigin = await qx<string | null>(main, 'harness_proxy_url', {}).catch(() => null);
    log(`harness_proxy_url = ${dshOrigin ?? '(null)'}`);

    // 进入 DSH 视图:优先点击含 DSH/对话 文案的导航
    await dumpNav(main);
    for (const label of ['DSH', 'dsh', '对话', '会话', 'Chat']) {
      const btn = main.locator(`button:has-text("${label}"), a:has-text("${label}"), [role="tab"]:has-text("${label}")`).first();
      if ((await btn.count().catch(() => 0)) > 0) {
        await btn.click({ timeout: 3000 }).catch(() => {});
        log(`clicked nav "${label}"`);
        break;
      }
    }
    await new Promise((r) => setTimeout(r, 3000));

    // 找聊天 frame:优先 dsh origin;否则逐个检查非 5190 的 http frame
    let chat: Frame | undefined;
    for (let round = 0; round < 20 && !chat; round += 1) {
      const frames = main.frames();
      const urls = frames.map((f) => f.url());
      if (round % 5 === 0) log(`frames: ${JSON.stringify(urls.slice(0, 6))}`);
      const candidates = frames.filter((f) => {
        const u = f.url();
        // 空 URL frame = qianxun 内嵌 DSH 的 iframe(src 不可读);排除 vite 主页面
        return u === '' || (u.startsWith('http') && !u.includes('localhost:5190'));
      });
      // dsh origin 优先
      if (dshOrigin) {
        const byOrigin = candidates.find((f) => f.url().startsWith(dshOrigin));
        if (byOrigin) candidates.unshift(byOrigin);
      }
      // 空 URL frame 放最后候选
      candidates.sort((a, b) => Number(b.url() === '') - Number(a.url() === ''));
      for (const f of candidates) {
        const probeInfo = await f
          .evaluate(() => ({
            hasBox: Boolean(document.querySelector('textarea, [contenteditable="true"], [role="textbox"]')),
            title: document.title.slice(0, 40),
            text: document.body.innerText.slice(0, 80).replace(/\s+/g, ' '),
          }))
          .catch(() => null);
        if (probeInfo?.hasBox) {
          log(`candidate frame title="${probeInfo.title}" text="${probeInfo.text}"`);
          chat = f;
          break;
        }
      }
      if (!chat) await new Promise((r) => setTimeout(r, 1500));
    }
    if (!chat) {
      await dumpNav(main);
      fail('未找到 DSH 会话输入框(已 dump 导航候选)');
    }
    log(`chat frame: ${chat.url().slice(0, 80)}`);

    // 新建会话(当前在会话列表页,textbox 是搜索框)
    // 注意:侧栏可能存在名字就叫"新会话"的旧会话,必须精确匹配文本
    const newChat = chat.getByText('新会话', { exact: true }).first();
    if ((await newChat.count().catch(() => 0)) > 0) {
      await newChat.click({ timeout: 5000 }).catch(() => {});
      log('clicked 新会话(exact)');
      await new Promise((r) => setTimeout(r, 3000));
    } else {
      log('未找到 新会话 按钮(可能已在会话内)');
    }

    // 若默认模型是 MiniMax(dev 账号 429),经顶栏选择器切到 GLM-5.3-Flash
    try {
      const beforeText = await chat.evaluate(() => document.body.innerText);
      if (beforeText.includes('MiniMax')) {
        const trigger = chat
          .locator(
            'button:has-text("MiniMax"), [role="combobox"]:has-text("MiniMax"), [role="button"]:has-text("MiniMax"), [aria-expanded]:has-text("MiniMax")',
          )
          .first();
        await trigger.click({ timeout: 5000 });
        await new Promise((r) => setTimeout(r, 1200));
        // dump 弹层结构(菜单/popover/dropdown)
        const menuDump = await chat
          .evaluate(() => {
            const sels = [
              '[role="menu"]',
              '[role="listbox"]',
              '[role="dialog"]',
              '[class*="menu" i]',
              '[class*="popover" i]',
              '[class*="dropdown" i]',
              '[data-radix-popper-content-wrapper]',
            ];
            const out: Array<{ cls: string; text: string }> = [];
            for (const s of sels) {
              for (const el of Array.from(document.querySelectorAll(s))) {
                const text = (el.textContent ?? '').replace(/\s+/g, ' ').slice(0, 200);
                if (text.includes('GLM') || text.includes('MiniMax')) {
                  out.push({ cls: (el.className ?? '').toString().slice(0, 60), text });
                }
              }
            }
            return out.slice(0, 6);
          })
          .catch(() => []);
        log(`menu dump: ${JSON.stringify(menuDump)}`);
        // 二级菜单:先点第一层里的"模型: MiniMax-M3"行,展开模型列表
        const modelRow = chat
          .locator('[class*="menu" i] >> text=MiniMax-M3')
          .first();
        await modelRow.click({ timeout: 5000 });
        await new Promise((r) => setTimeout(r, 1000));
        // 模型列表里选 GLM-5.3-Flash(portal 渲染在 body 尾部,用 last 兜底)
        const item = chat.locator('text=GLM-5.3-Flash').last();
        if ((await item.count().catch(() => 0)) > 0) {
          await item.click({ timeout: 5000 });
          log('switched model → GLM-5.3-Flash');
        } else {
          const tail = (await chat.evaluate(() => document.body.innerText)).slice(-500).replace(/\s+/g, ' ');
          log(`[dump] 模型菜单未找到 GLM-5.3-Flash,页面尾部: ${tail}`);
          await chat.locator('body').press('Escape').catch(() => {});
        }
        await new Promise((r) => setTimeout(r, 800));
      } else {
        log('默认模型非 MiniMax,无需切换');
      }
    } catch (e) {
      log(`切模型失败(继续用默认):${e instanceof Error ? e.message : String(e)}`);
    }

    // 选真正的消息输入框:composer placeholder 含 "调用指令"/"描述你想要";找不到再取最后一个
    async function pickEditor(): Promise<ReturnType<Frame['locator']>> {
      const all = chat!.locator('textarea, [contenteditable="true"], [role="textbox"]');
      const n = await all.count();
      for (let k = 0; k < n; k += 1) {
        const ph = (await all.nth(k).getAttribute('placeholder').catch(() => '')) ?? '';
        if (/调用指令|描述你想要|文件或对话/.test(ph)) return all.nth(k);
      }
      for (let k = 0; k < n; k += 1) {
        const ph = (await all.nth(k).getAttribute('placeholder').catch(() => '')) ?? '';
        if (/输入|消息|message|ask|send/i.test(ph)) return all.nth(k);
      }
      return all.nth(Math.max(0, n - 1));
    }

    // 逐条发送并验证
    let pass = 0;
    const results: string[] = [];
    for (let i = 0; i < TIMES; i += 1) {
      const c = CASES[i % CASES.length];
      const prompt = promptFor(c);
      const beforeCount = await chat
        .evaluate(() => (document.body.innerText.match(/laya_decide/g) ?? []).length)
        .catch(() => 0);
      const editor = await pickEditor();
      await editor.fill(prompt);
      // 校验 fill 是否进入 composer(读值)
      const filled = await editor
        .evaluate((el) =>
          el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement
            ? el.value.slice(0, 30)
            : (el.textContent ?? '').slice(0, 30),
        )
        .catch(() => '');
      log(`fill 前 30 字: ${filled}`);
      await editor.press('Enter');
      await new Promise((r) => setTimeout(r, 2500));
      // 发送校验:输入框应被清空;否则点发送按钮兜底
      const leftover = await editor
        .evaluate((el) =>
          el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement
            ? el.value
            : (el.textContent ?? ''),
        )
        .catch(() => 'gone');
      if (leftover && leftover.length > 10) {
        log(`Enter 后输入框仍有 ${leftover.length} 字,尝试发送按钮`);
        const sendBtn = chat
          .locator(
            'button[aria-label*="发送" i], button[type="submit"], form button[type="submit"], button:has-text("发送")',
          )
          .first();
        if ((await sendBtn.count().catch(() => 0)) > 0) {
          await sendBtn.click({ timeout: 3000 }).catch(() => {});
          await new Promise((r) => setTimeout(r, 2500));
        }
      }

      // 轮询结果:agent 回复 "部门=<choice>,置信度=…"(最多 90s;
      // 处理完成后视图可能跳回会话列表,60s 时点开最近会话再读)
      const successRe = new RegExp(`部门\\s*[=:]\\s*${c.expectChoice}`);
      let ok = false;
      for (let t = 0; t < 90; t += 1) {
        const body = await chat.evaluate(() => document.body.innerText).catch(() => '');
        if (body.includes('laya_decide') && successRe.test(body) && !body.includes('本轮运行失败')) {
          ok = true;
          break;
        }
        if (t === 60) {
          // 处理完成会跳回列表:点开最近的 laya_decide 会话读取工具卡片
          await chat
            .locator('text=请调用 laya_decide 工具')
            .first()
            .click({ timeout: 5000 })
            .catch(() => {});
        }
        if (t === 89) {
          // dump:用户消息之后的内容 + 关键状态词
          const after = await chat
            .evaluate(() => {
              const text = document.body.innerText;
              const idx = text.lastIndexOf('调用完成后');
              return text.slice(idx >= 0 ? idx : Math.max(0, text.length - 700));
            })
            .catch(() => '');
          const flags = ['laya_decide', '处理失败', 'RATE_LIMIT', '429', '部门', '置信度']
            .map((k) => `${k}=${body.includes(k) ? 1 : 0}`)
            .join(' ');
          log(`[dump] ${c.name} flags: ${flags} | after: ${after.slice(0, 400).replace(/\s+/g, ' ')}`);
        }
        await new Promise((r) => setTimeout(r, 1000));
      }
      if (ok) pass += 1;
      results.push(`${i + 1}. ${c.name} → expect=${c.expectChoice} ${ok ? '✓' : '✗'}`);
      log(results[results.length - 1]);
    }

    log(`结果:${pass}/${TIMES} 通过`);
    if (pass >= TIMES) {
      log('✓ e2e 验证全部通过');
      process.exit(0);
    } else {
      fail(`仅 ${pass}/${TIMES} 通过`);
    }
  } catch (err) {
    fail(`unexpected: ${err instanceof Error ? err.message : String(err)}`);
  }
}

void main();
