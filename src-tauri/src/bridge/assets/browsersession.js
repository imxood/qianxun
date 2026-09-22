/**
 * 浏览器自动化会话（R001 D11）：playwright-core 经 CDP 驱动 moli serve
 * --layout。依赖全部注入（loadChromium / startServer），本模块只管
 * 会话状态机——单元测试用假 connector，真实链路由 index.js 组装。
 */

const OPEN_TIMEOUT_MS = 20_000;
const OP_TIMEOUT_MS = 10_000;
const DEFAULT_IDLE_MS = 3 * 60 * 1000;

/**
 * @param {{
 *   loadChromium: () => Promise<{ connectOverCDP: (url: string) => Promise<any> }>,
 *   startServer: () => Promise<{ endpoint: string, stop: () => void }>,
 *   idleMs?: number,
 *   now?: () => number,
 * }} deps
 */
export function createBrowserAutomation(deps) {
  const idleMs = deps.idleMs ?? DEFAULT_IDLE_MS;
  let session = null; // { browser, page, endpoint, stop, idleTimer }
  let starting = null;

  async function ensureSession() {
    if (session) return session;
    if (starting) return starting;
    starting = (async () => {
      const server = await deps.startServer();
      try {
        const chromium = await deps.loadChromium();
        const browser = await chromium.connectOverCDP(server.endpoint);
        const context = browser.contexts()[0] ?? (await browser.newContext());
        const page = context.pages()[0] ?? (await context.newPage());
        session = { browser, page, endpoint: server.endpoint, stop: server.stop };
        armIdleTimer();
        return session;
      } catch (cause) {
        server.stop();
        starting = null;
        throw cause;
      }
    })();
    try {
      return await starting;
    } finally {
      starting = null;
    }
  }

  function armIdleTimer() {
    if (session?.idleTimer) clearTimeout(session.idleTimer);
    if (idleMs <= 0 || !session) return;
    session.idleTimer = setTimeout(() => {
      void closeSession();
    }, idleMs);
  }

  async function closeSession() {
    const current = session;
    session = null;
    if (!current) return;
    if (current.idleTimer) clearTimeout(current.idleTimer);
    try {
      await current.browser.close();
    } catch {
      // 连接已断开视为关闭成功。
    }
    try {
      current.stop();
    } catch {
      // serve 进程已退出。
    }
  }

  return {
    /** 会话是否存活（测试与工具描述用）。 */
    is_open() {
      return session !== null;
    },
    /** 打开 URL：无会话则先拉起 moli serve --layout。返回页面标题。 */
    async open(url) {
      const target = String(url ?? "").trim();
      if (!/^https?:\/\//i.test(target)) throw new Error("仅支持 http(s) URL");
      const current = await ensureSession();
      await current.page.goto(target, {
        waitUntil: "domcontentloaded",
        timeout: OPEN_TIMEOUT_MS,
      });
      armIdleTimer();
      return current.page.title();
    },
    /** 页面可交互元素清单：tag + 定位线索（id/name/文本）。 */
    async snapshot() {
      const current = await ensureSession();
      armIdleTimer();
      const items = await current.page.$$eval(
        "a, button, input, select, textarea, [role]",
        (nodes) =>
          nodes.slice(0, 100).map((node) => {
            const tag = node.tagName.toLowerCase();
            const id = node instanceof HTMLElement && node.id ? "#" + node.id : "";
            const label =
              node.getAttribute("aria-label") ??
              (node instanceof HTMLInputElement ? node.placeholder : null) ??
              (node.textContent ?? "").trim().slice(0, 40);
            return `${tag}${id} ${label ?? ""}`.trim();
          }),
      );
      return items.length > 0 ? items.join("\n") : "（无可交互元素）";
    },
    /** 点击元素（CSS 选择器）。 */
    async click(selector) {
      const current = await ensureSession();
      armIdleTimer();
      await current.page.click(String(selector), { timeout: OP_TIMEOUT_MS });
      return `已点击 ${selector}`;
    },
    /** 填充输入（CSS 选择器 + 文本）。 */
    async fill(selector, text) {
      const current = await ensureSession();
      armIdleTimer();
      await current.page.fill(String(selector), String(text ?? ""), {
        timeout: OP_TIMEOUT_MS,
      });
      return `已填充 ${selector}`;
    },
    /** 视口截图（serve 需 --layout；moli 软件渲染）。 */
    async screenshot(filePath) {
      const current = await ensureSession();
      armIdleTimer();
      await current.page.screenshot({ path: String(filePath), timeout: OP_TIMEOUT_MS });
      return filePath;
    },
    /** 关闭会话（浏览器断连 + serve 进程退出）。 */
    async close() {
      await closeSession();
      return "浏览器会话已关闭";
    },
  };
}
