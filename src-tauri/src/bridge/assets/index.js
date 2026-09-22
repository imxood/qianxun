/**
 * qx-bridge —— 千寻笔记桥（DSH Host 插件，M6）。
 *
 * 由千寻外壳部署到 `<DSH_HOME>/profiles/web/node_modules/qx-bridge/`，
 * 经 profile 的 cordis.patch.yml 插入根条目装配。零第三方依赖：
 * - 注册三个 agent 全局工具：note_search / note_read / note_write，
 *   直接读写千寻笔记库（frontmatter 语义与千寻 Rust 侧一致）；
 * - 在 DSH webServer 上挂 `POST /qx/notes/organize`：千寻前端跨源调用，
 *   服务端组装上下文并经 `llm` 服务流式生成「AI 整理」结果。
 */
import { execFile } from "node:child_process";
import { tmpdir } from "node:os";
import { randomUUID } from "node:crypto";
import { buildFetchArgs, buildSearchUrl, itemsToMarkdown, parseBingRss, parseSearchMarkdown, parseSearxngJson } from "./websearch.js";
import { createRequire } from "node:module";
import { createBrowserAutomation } from "./browsersession.js";
import { readFile, readdir, rename, writeFile } from "node:fs/promises";
import { join, dirname, resolve as resolvePath } from "node:path";

export const name = "qx-bridge";

export const inject = ["tools", "systemPrompt"];

const MAX_LIST = 50;
const MAX_READ_BYTES = 256 * 1024;
const ORGANIZE_ENDPOINT = "/qx/notes/organize";

/** 前台工具说明：注册进系统提示，教 agent 何时用笔记工具。 */
const PROMPT_SECTIONS = {
  search:
    "千寻笔记库检索：当用户提到「我的笔记」「笔记里找」或需要回忆个人记录时，先用 note_search 再用 note_read。",
  read: "note_read 读取千寻笔记全文（含 frontmatter 后的正文）。",
  write:
    "note_write 原子写入千寻笔记（整篇替换，须保留原 frontmatter 结构）。AI 整理、改写、归档笔记后用它落盘。",
  webSearch:
    "联网搜索：当需要最新资料、教程、文档、新闻或任何训练知识之外的信息时，先用 web_search 获取结果列表，再用 web_read 读取关键来源的正文。",
  webRead:
    "读取网页正文并转成 Markdown：传入 web_search 结果或用户提供的 URL，返回可读正文（经 Moli 渲染，动态内容也能拿到）。",
};

// ---- 联网搜索/抓取（R001） ----

const MAX_WEB_BYTES = 256 * 1024;
const WEB_TIMEOUT_MS = 25_000;

/** 桥内 web 能力：moli 优先，无 moli 时降级 Node fetch 纯文本。 */
function makeWeb(config) {
  const moliPath = String(config.moliPath ?? "").trim();
  // TUN/fake-ip 网络适配（settings.web.proxy → patch proxy）：空 = 直连，
  // "off" = 显式直连，其它 = moli --http-proxy（代理侧解析，守卫不适用）。
  const proxy = String(config.proxy ?? "").trim();

  function spawnMoli(args) {
    if (!moliPath) return Promise.reject(new Error("moli 未配置"));
    return new Promise((resolve, reject) => {
      execFile(
        moliPath,
        args,
        { timeout: WEB_TIMEOUT_MS, maxBuffer: 4 * 1024 * 1024, windowsHide: true },
        (error, stdout) => {
          if (error) {
            reject(new Error(`moli 执行失败：${error.message}`));
          } else {
            resolve(String(stdout));
          }
        },
      );
    });
  }

  async function moliMarkdown(url) {
    return spawnMoli([
      ...buildFetchArgs({ proxy, timeoutMs: WEB_TIMEOUT_MS - 5_000 }),
      url,
    ]);
  }

  /** Node fetch 降级：剥掉标签取近纯文本（无 moli 时的兜底形态）。 */
  async function fallbackRead(url) {
    const response = await fetch(url, { signal: AbortSignal.timeout(WEB_TIMEOUT_MS) });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const raw = await response.text();
    const text = raw
      .replace(/<script[\s\S]*?<\/script>/gi, "")
      .replace(/<style[\s\S]*?<\/style>/gi, "")
      .replace(/<[^>]+>/g, " ")
      .replace(/\s+\n\s+/g, "\n")
      .trim();
    return `(降级纯文本：未检测到 Moli，内容未经渲染)\n\n${text}`;
  }

  return {
    hasMoli: () => moliPath !== "",
    async read(url) {
      const target = String(url ?? "").trim();
      if (!/^https?:\/\//i.test(target)) throw new Error("仅支持 http(s) URL");
      try {
        return await moliMarkdown(target);
      } catch (cause) {
        if (!moliPath) return fallbackRead(target);
        throw cause;
      }
    },
    async search({ query, engine, limit }) {
      const kind = typeof engine === "string" ? engine : engine?.kind ?? "duckduckgo";
      const url = buildSearchUrl(engine ?? "duckduckgo", query);
      const max = Math.max(1, Math.min(50, Number(limit) || 10));
      let markdown;
      try {
        markdown = await moliMarkdown(url);
      } catch (cause) {
        if (!moliPath) {
          // 降级：Node fetch 搜索页（噪音大，只提取链接）。
          const response = await fetch(url, { signal: AbortSignal.timeout(WEB_TIMEOUT_MS) });
          if (!response.ok) throw new Error(`HTTP ${response.status}`);
          markdown = await response.text();
        } else {
          throw cause;
        }
      }
      // bing 走 RSS（网页版是无 cookie 反爬壳）；searxng 走 JSON；
      // 其余（duckduckgo）走通用 Markdown 链接解析。
      const items =
        kind === "bing"
          ? parseBingRss(markdown, kind, max)
          : kind === "searxng"
            ? (parseSearxngJson(markdown, kind, max) ?? parseSearchMarkdown(markdown, kind, max))
            : parseSearchMarkdown(markdown, kind, max);
      const summary = itemsToMarkdown(items);
      return { items, markdown: summary, raw: markdown.slice(0, MAX_WEB_BYTES) };
    },
  };
}

/** 加载 playwright-core：优先用户配置路径，其次桥所在解析链（开发环境）。 */
/** CJS/ESM interop 归一：拿到底层 chromium 命名空间。 */
function pickChromium(mod) {
  return mod?.chromium ?? mod?.default?.chromium ?? mod;
}

function loadChromium(preferredPath) {
  return async () => {
    const require = createRequire(import.meta.url);
    if (preferredPath) {
      try {
        return pickChromium(require(preferredPath));
      } catch {
        // 路径失效 → 落到裸解析再试一次。
      }
    }
    try {
      return pickChromium(require("playwright-core"));
    } catch {
      throw new Error(
        "未找到 playwright-core：请在设置页配置其安装路径（npm 包 playwright-core，零依赖，解压即用）",
      );
    }
  };
}

/** 拉起 moli serve --layout（CDP 端口临时分配），等待 /json/version 就绪。 */
function startMoliServe(moliPath) {
  return async () => {
    if (!moliPath) throw new Error("moli 未配置，浏览器自动化不可用");
    const { spawn } = await import("node:child_process");
    const port = 19290 + Math.floor(Math.random() * 100);
    const child = spawn(moliPath, ["serve", "--layout", "--port", String(port)], {
      stdio: "ignore",
      windowsHide: true,
    });
    const endpoint = `http://127.0.0.1:${port}`;
    const deadline = Date.now() + 15_000;
    while (Date.now() < deadline) {
      if (child.exitCode !== null) {
        throw new Error(`moli serve 提前退出（code ${child.exitCode}）`);
      }
      try {
        const response = await fetch(`${endpoint}/json/version`);
        if (response.ok) {
          return { endpoint, stop: () => child.kill() };
        }
      } catch {
        // 端口未就绪，继续等。
      }
      await new Promise((resolve) => setTimeout(resolve, 150));
    }
    child.kill();
    throw new Error("moli serve 15s 未就绪");
  };
}

/** 浏览器自动化（playwright-core + moli serve --layout，R001 D11）。 */
function makeAutomation(config) {
  const moliPath = String(config.moliPath ?? "").trim();
  return createBrowserAutomation({
    loadChromium: loadChromium(String(config.playwrightCorePath ?? "").trim()),
    startServer: startMoliServe(moliPath),
  });
}

// ---- frontmatter（与千寻 Rust notes::commands 语义一致） ----

function parseFrontmatter(text) {
  if (!text.startsWith("---\n")) return undefined;
  const rest = text.slice(4);
  const end = rest.indexOf("\n---");
  if (end < 0) return undefined;
  let title;
  const tags = [];
  for (const line of rest.slice(0, end).split("\n")) {
    if (line.startsWith("title:")) {
      title = line.slice(6).trim().replace(/^["']|["']$/g, "");
    } else if (line.startsWith("tags:")) {
      const inner = line.slice(5).trim().replace(/^\[|\]$/g, "");
      for (const tag of inner.split(",")) {
        const clean = tag.trim().replace(/^["']|["']$/g, "");
        if (clean) tags.push(clean);
      }
    }
  }
  return title === undefined ? undefined : { title, tags };
}

function stripFrontmatter(text) {
  if (!text.startsWith("---\n")) return text;
  const rest = text.slice(4);
  const end = rest.indexOf("\n---");
  if (end < 0) return text;
  return rest.slice(end + 4).replace(/^\n+/, "");
}

// ---- 库操作 ----

function makeVault(config) {
  // QX_VAULT：测试/开发逃生口（正常路径 = 部署时写入 config.vault）。
  const raw = String(config.vault ?? process.env.QX_VAULT ?? "");
  const root = resolvePath(raw);
  if (!raw) throw new Error("qx-bridge: 未配置笔记库目录（config.vault / QX_VAULT）");
  return {
    root,
    /** 相对路径安全化：拒绝绝对路径与 `..` 越界。 */
    resolve(relative) {
      if (typeof relative !== "string" || relative.length === 0) {
        throw new Error("qx-bridge: 笔记路径不能为空");
      }
      const normalized = relative.replace(/\\/g, "/");
      if (normalized.startsWith("/") || /^[A-Za-z]:/.test(normalized)) {
        throw new Error(`qx-bridge: 非法笔记路径（绝对路径）：${relative}`);
      }
      if (normalized.split("/").some((part) => part === "..")) {
        throw new Error(`qx-bridge: 非法笔记路径（越界）：${relative}`);
      }
      return join(root, normalized);
    },
    async list() {
      const out = [];
      async function walk(dir, depth) {
        if (depth > 4 || out.length > 2000) return;
        let entries;
        try {
          entries = await readdir(dir, { withFileTypes: true });
        } catch {
          return;
        }
        for (const entry of entries) {
          if (entry.name.startsWith(".")) continue;
          const full = join(dir, entry.name);
          if (entry.isDirectory()) {
            await walk(full, depth + 1);
          } else if (entry.name.toLowerCase().endsWith(".md")) {
            let text = "";
            try {
              text = await readFile(full, "utf8");
            } catch {
              continue;
            }
            const fm = parseFrontmatter(text);
            out.push({
              path: full.slice(root.length + 1).replace(/\\/g, "/"),
              title: fm?.title ?? entry.name.replace(/\.md$/i, ""),
              tags: fm?.tags ?? [],
              body: stripFrontmatter(text),
            });
          }
        }
      }
      await walk(root, 0);
      return out;
    },
  };
}

async function atomicWrite(file, content) {
  const temp = join(dirname(file), `.qx-${process.pid}-${Date.now()}.tmp`);
  await writeFile(temp, content, "utf8");
  await rename(temp, file);
}

/** 工具输出渲染：全部走纯文本信封（个人工具，无需富卡片）。 */
function textEnvelope(title, body) {
  return [{ type: "text", text: `<${title}>\n${body}\n</${title}>` }];
}

const stringOut = {
  type: "object",
  properties: { result: { type: "string" } },
  required: ["result"],
};

function defineSimpleTool(definition) {
  // 与官方 defineTool 的产物同形：parameters 已是原生 JSON Schema，
  // execute 自带最小校验。零依赖实现（不能 import dsh-tools：
  // 本包不进 pnpm 依赖树，部署侧只拷贝文件）。
  return definition;
}

export function apply(ctx, config) {
  const vault = makeVault(config);

  // ---- note_search ----
  ctx.systemPrompt.section({ name: "tool:note_search", order: 100, text: PROMPT_SECTIONS.search });
  ctx.tools.register(
    defineSimpleTool({
      name: "note_search",
      description: "在千寻笔记库中检索：按关键词匹配标题、标签与正文，返回最近匹配列表（不读全文）。",
      parameters: {
        type: "object",
        properties: {
          query: { type: "string", description: "关键词（大小写不敏感，空串 = 列出全部）" },
        },
        required: ["query"],
      },
      output: { schema: stringOut, render: (_args, value) => textEnvelope("note_search", value.result) },
      isConcurrencySafe: () => true,
      async execute(args) {
        const keyword = String(args.query ?? "").toLowerCase();
        const notes = await vault.list();
        const hits = keyword
          ? notes.filter(
              (note) =>
                note.title.toLowerCase().includes(keyword)
                || note.path.toLowerCase().includes(keyword)
                || note.tags.some((tag) => tag.toLowerCase().includes(keyword))
                || note.body.toLowerCase().includes(keyword),
            )
          : notes;
        const lines = hits.slice(0, MAX_LIST).map((note) => {
          const tags = note.tags.length > 0 ? `  [${note.tags.join(", ")}]` : "";
          return `- ${note.title}${tags}\n  ${note.path}`;
        });
        return {
          result:
            lines.length > 0
              ? `${hits.length} 篇匹配（列出前 ${lines.length}）：\n${lines.join("\n")}`
              : `无匹配（库内共 ${notes.length} 篇）`,
        };
      },
    }),
  );

  // ---- note_read ----
  ctx.systemPrompt.section({ name: "tool:note_read", order: 100, text: PROMPT_SECTIONS.read });
  ctx.tools.register(
    defineSimpleTool({
      name: "note_read",
      description: "读取一篇千寻笔记的正文（frontmatter 已剥离）。路径来自 note_search 结果。",
      parameters: {
        type: "object",
        properties: { path: { type: "string", description: "笔记相对路径（正斜杠）" } },
        required: ["path"],
      },
      output: { schema: stringOut, render: (_args, value) => textEnvelope("note_read", value.result) },
      isConcurrencySafe: () => true,
      async execute(args) {
        const file = vault.resolve(args.path);
        const text = await readFile(file, "utf8");
        const body = stripFrontmatter(text);
        const bounded =
          Buffer.byteLength(body, "utf8") > MAX_READ_BYTES
            ? `${body.slice(0, MAX_READ_BYTES)}\n…（超长截断）`
            : body;
        return { result: bounded };
      },
    }),
  );

  // ---- note_write ----
  ctx.systemPrompt.section({ name: "tool:note_write", order: 100, text: PROMPT_SECTIONS.write });
  ctx.tools.register(
    defineSimpleTool({
      name: "note_write",
      description:
        "整篇原子写入千寻笔记（content 含 frontmatter，覆盖原文）。新建笔记用 `new/标题.md` 形式的相对路径。",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "笔记相对路径（正斜杠；new/ 前缀 = 新建）" },
          content: { type: "string", description: "完整 Markdown（含 frontmatter）" },
        },
        required: ["path", "content"],
      },
      output: { schema: stringOut, render: (_args, value) => textEnvelope("note_write", value.result) },
      isConcurrencySafe: () => false,
      async execute(args) {
        const file = vault.resolve(args.path);
        const { mkdir } = await import("node:fs/promises");
        await mkdir(dirname(file), { recursive: true });
        await atomicWrite(file, String(args.content ?? ""));
        return { result: `已写入 ${args.path}` };
      },
    }),
  );

  // ---- web_search / web_read ----
  const web = makeWeb(config);

  ctx.systemPrompt.section({ name: "tool:web_search", order: 90, text: PROMPT_SECTIONS.webSearch });
  ctx.tools.register(
    defineSimpleTool({
      name: "web_search",
      description: "联网搜索：按关键词搜索网页，返回标题/链接/摘要列表（Markdown）。",
      parameters: {
        type: "object",
        properties: {
          query: { type: "string", description: "搜索关键词" },
          engine: { type: "string", description: "可选引擎：bing / baidu / duckduckgo / searxng（缺省 bing）" },
          limit: { type: "number", description: "可选条数上限（1-50，缺省 10）" },
        },
        required: ["query"],
      },
      output: { schema: stringOut, render: (_args, value) => textEnvelope("web_search", value.result) },
      isConcurrencySafe: () => true,
      async execute(args) {
        const outcome = await web.search({
          query: String(args.query ?? ""),
          engine: typeof args.engine === "string" && args.engine ? args.engine : undefined,
          limit: typeof args.limit === "number" ? args.limit : undefined,
        });
        return { result: outcome.markdown };
      },
    }),
  );

  ctx.systemPrompt.section({ name: "tool:web_read", order: 90, text: PROMPT_SECTIONS.webRead });
  ctx.tools.register(
    defineSimpleTool({
      name: "web_read",
      description: "读取网页正文（Markdown）：web_search 结果或用户给的 URL；经 Moli 渲染，动态内容可读。",
      parameters: {
        type: "object",
        properties: {
          url: { type: "string", description: "http(s) 网页地址" },
        },
        required: ["url"],
      },
      output: { schema: stringOut, render: (_args, value) => textEnvelope("web_read", value.result) },
      isConcurrencySafe: () => true,
      async execute(args) {
        const markdown = await web.read(String(args.url ?? ""));
        const bounded =
          Buffer.byteLength(markdown, "utf8") > MAX_WEB_BYTES
            ? `${markdown.slice(0, MAX_WEB_BYTES)}\n…（超长截断）`
            : markdown;
        return { result: bounded };
      },
    }),
  );

  // ---- browser_*（playwright-core + moli serve --layout） ----
  const automation = makeAutomation(config);

  ctx.systemPrompt.section({
    name: "tool:browser_*",
    order: 90,
    text: "浏览器自动化：browser_open 打开页面后可用 browser_snapshot 查看可交互元素（选择器线索），browser_click / browser_fill 操作，browser_screenshot 截图（返回文件路径），browser_close 关闭。全部经 Moli 无头浏览器执行。",
  });
  const browserTools = [
    {
      name: "browser_open",
      description: "打开网页（Moli 无头浏览器，JS 渲染）。返回页面标题。",
      parameters: {
        type: "object",
        properties: { url: { type: "string", description: "http(s) 网页地址" } },
        required: ["url"],
      },
      run: (args) => automation.open(String(args.url ?? "")),
    },
    {
      name: "browser_snapshot",
      description: "列出当前页可交互元素（tag#id 文本），供 click/fill 定位。",
      parameters: { type: "object", properties: {} },
      run: () => automation.snapshot(),
    },
    {
      name: "browser_click",
      description: "点击当前页元素（CSS 选择器，来自 browser_snapshot）。",
      parameters: {
        type: "object",
        properties: { selector: { type: "string", description: "CSS 选择器" } },
        required: ["selector"],
      },
      run: (args) => automation.click(String(args.selector ?? "")),
    },
    {
      name: "browser_fill",
      description: "填充当前页输入框（CSS 选择器 + 文本）。",
      parameters: {
        type: "object",
        properties: {
          selector: { type: "string", description: "CSS 选择器" },
          text: { type: "string", description: "要填入的文本" },
        },
        required: ["selector", "text"],
      },
      run: (args) => automation.fill(String(args.selector ?? ""), String(args.text ?? "")),
    },
    {
      name: "browser_screenshot",
      description: "视口截图（PNG 落盘），返回文件路径。",
      parameters: { type: "object", properties: {} },
      run: async () =>
        "截图已保存：" +
        String(
          await automation.screenshot(
            `${tmpdir()}\\qx-bridge-shot-${Date.now()}.png`,
          ),
        ),
    },
  ];
  for (const tool of browserTools) {
    ctx.tools.register(
      defineSimpleTool({
        name: tool.name,
        description: tool.description,
        parameters: tool.parameters,
        output: { schema: stringOut, render: (_args, value) => textEnvelope(tool.name, value.result) },
        isConcurrencySafe: () => false,
        async execute(args) {
          return { result: String(await tool.run(args)) };
        },
      }),
    );
  }
  ctx.tools.register(
    defineSimpleTool({
      name: "browser_close",
      description: "关闭浏览器会话与 Moli serve 进程。",
      parameters: { type: "object", properties: {} },
      output: { schema: stringOut, render: (_args, value) => textEnvelope("browser_close", value.result) },
      isConcurrencySafe: () => false,
      async execute() {
        return { result: await automation.close() };
      },
    }),
  );

  // ---- HTTP 通道（可选服务：webServer / llm）----
  // 声明式依赖（ctx.inject）：webServer/llm 未 ACTIVE 时内层插件保持
  // PENDING，服务就绪后 cordis 自动装载并挂路由；服务缺失则永不挂载，
  // 但不阻塞上方工具注册（与 M6「任一缺失则跳过」语义一致）。
  // 修复：旧写法 ctx.get("webServer") 在 apply 瞬间同步取值，而 webServer
  // 的 [Service.init]（绑定端口）是异步的，常返回 undefined 被 if 静默
  // 跳过——路由从未挂载（/qx/notes/organize 同病）。ctx.get 无等待语义，
  // 只有 inject 声明才能让 cordis 排队等依赖 ACTIVE。
  ctx.inject(["webServer"], (http) => {
    const webServer = http.webServer;
    // 路由 effect 挂在内层 fiber：webServer 重载时自动注销再重挂。
    http.effect(() =>
      webServer.register(
        {
          kind: "exact",
          path: "/qx/web/search",
          async handler(req, res) {
            res.setHeader("Access-Control-Allow-Origin", "*");
            res.setHeader("Access-Control-Allow-Methods", "POST, OPTIONS");
            res.setHeader("Access-Control-Allow-Headers", "Content-Type");
            if (req.method === "OPTIONS") {
              res.writeHead(204);
              res.end();
              return;
            }
            if (req.method !== "POST") {
              res.writeHead(405, { "Content-Type": "application/json" });
              res.end(JSON.stringify({ ok: false, error: "method not allowed" }));
              return;
            }
            const chunks = [];
            for await (const chunk of req) chunks.push(chunk);
            let body;
            try {
              body = JSON.parse(Buffer.concat(chunks).toString("utf8"));
            } catch {
              res.writeHead(400, { "Content-Type": "application/json" });
              res.end(JSON.stringify({ ok: false, error: "invalid JSON body" }));
              return;
            }
            try {
              const outcome = await web.search({
                query: String(body.query ?? ""),
                engine: body.engine ?? undefined,
                limit: typeof body.limit === "number" ? body.limit : undefined,
              });
              res.writeHead(200, { "Content-Type": "application/json" });
              res.end(JSON.stringify({ ok: true, markdown: outcome.markdown, items: outcome.items }));
            } catch (error) {
              res.writeHead(500, { "Content-Type": "application/json" });
              res.end(JSON.stringify({ ok: false, error: error instanceof Error ? error.message : String(error) }));
            }
          },
        },
      ),
    );
    http.effect(() =>
      webServer.register(
        {
          kind: "exact",
          path: "/qx/web/read",
          async handler(req, res) {
            res.setHeader("Access-Control-Allow-Origin", "*");
            res.setHeader("Access-Control-Allow-Methods", "POST, OPTIONS");
            res.setHeader("Access-Control-Allow-Headers", "Content-Type");
            if (req.method === "OPTIONS") {
              res.writeHead(204);
              res.end();
              return;
            }
            if (req.method !== "POST") {
              res.writeHead(405, { "Content-Type": "application/json" });
              res.end(JSON.stringify({ ok: false, error: "method not allowed" }));
              return;
            }
            const chunks = [];
            for await (const chunk of req) chunks.push(chunk);
            let body;
            try {
              body = JSON.parse(Buffer.concat(chunks).toString("utf8"));
            } catch {
              res.writeHead(400, { "Content-Type": "application/json" });
              res.end(JSON.stringify({ ok: false, error: "invalid JSON body" }));
              return;
            }
            try {
              let markdown = await web.read(String(body.url ?? ""));
              let truncated = false;
              if (Buffer.byteLength(markdown, "utf8") > MAX_WEB_BYTES) {
                markdown = markdown.slice(0, MAX_WEB_BYTES);
                truncated = true;
              }
              res.writeHead(200, { "Content-Type": "application/json" });
              res.end(JSON.stringify({ ok: true, markdown, truncated }));
            } catch (error) {
              res.writeHead(500, { "Content-Type": "application/json" });
              res.end(JSON.stringify({ ok: false, error: error instanceof Error ? error.message : String(error) }));
            }
          },
        },
      ),
    );
    http.logger.info(`qx-bridge: /qx/web/* 就绪（moli：${web.hasMoli() ? moliPathLabel(config) : "未配置，降级纯文本"}）`);
  });
  ctx.inject(["webServer", "llm"], (http) => {
    const webServer = http.webServer;
    const llm = http.llm;
    http.effect(() =>
      webServer.register({
        kind: "exact",
        path: ORGANIZE_ENDPOINT,
        async handler(req, res) {
          const cors = () => {
            res.setHeader("Access-Control-Allow-Origin", "*");
            res.setHeader("Access-Control-Allow-Methods", "POST, OPTIONS");
            res.setHeader("Access-Control-Allow-Headers", "Content-Type");
          };
          cors();
          if (req.method === "OPTIONS") {
            res.writeHead(204);
            res.end();
            return;
          }
          if (req.method !== "POST") {
            res.writeHead(405, { "Content-Type": "application/json" });
            res.end(JSON.stringify({ ok: false, error: "method not allowed" }));
            return;
          }
          const chunks = [];
          for await (const chunk of req) chunks.push(chunk);
          let body;
          try {
            body = JSON.parse(Buffer.concat(chunks).toString("utf8"));
          } catch {
            res.writeHead(400, { "Content-Type": "application/json" });
            res.end(JSON.stringify({ ok: false, error: "invalid JSON body" }));
            return;
          }
          try {
            const result = await organize(llm, vault, body);
            res.writeHead(200, { "Content-Type": "application/json" });
            res.end(JSON.stringify({ ok: true, result }));
          } catch (error) {
            res.writeHead(500, { "Content-Type": "application/json" });
            res.end(
              JSON.stringify({ ok: false, error: error instanceof Error ? error.message : String(error) }),
            );
          }
        },
      }),
    );
    http.logger.info(`qx-bridge: ${ORGANIZE_ENDPOINT} 就绪（库：${vault.root}）`);
  });

  ctx.logger.info(`qx-bridge: 笔记工具就绪（库：${vault.root}）`);
}

// ---- AI 整理 ----

async function organize(llm, vault, body) {
  const instruction = String(body.instruction ?? "").trim();
  if (!instruction) throw new Error("整理指令不能为空");
  // 公开 API listProviders()（私有字段 adapters 不属于对外契约）。
  const providers = (await llm.listProviders()).map((provider) => provider.id);
  if (providers.length === 0) {
    throw new Error("DSH 未配置任何模型 API：请在 DSH 网页的设置里配置模型后重试");
  }
  const provider = typeof body.provider === "string" && body.provider ? body.provider : providers[0];
  const model = typeof body.model === "string" && body.model ? body.model : "deepseek-chat";

  const all = await vault.list();
  const wanted = Array.isArray(body.paths) ? body.paths.map(String) : null;
  const selected = wanted ? all.filter((note) => wanted.includes(note.path)) : all;
  if (selected.length === 0) throw new Error("没有可整理的笔记（库为空或所选路径无匹配）");

  const context = selected
    .map((note) => `### ${note.title}（${note.path}）\n${note.body}`)
    .join("\n\n");
  const prompt = [
    "你是个人知识库整理助手。下面是用户的笔记正文（Markdown）。",
    "请按指令整理，输出 Markdown：保留事实、去除重复、条理化分层。",
    "若结果适合归档，可给出一篇可直接保存的整理稿（带 --- title/tags frontmatter）。",
    "",
    `指令：${instruction}`,
    "",
    context,
  ].join("\n");

  const messages = [
    {
      id: randomUUID(),
      role: "user",
      content: [{ type: "text", text: prompt }],
      source: { kind: "plugin", plugin: "qx-bridge" },
    },
  ];
  let out = "";
  let finishReason;
  for await (const chunk of llm.stream({
    provider,
    model,
    messages,
    system: "你是严谨的中文个人知识库整理助手，输出简洁的 Markdown。",
    maxTokens: 4096,
  })) {
    if (chunk.type === "text-delta") out += chunk.text;
    else if (chunk.type === "finish") finishReason = chunk.reason;
  }
  if (finishReason !== undefined && finishReason?.kind === "error") {
    throw new Error(`模型调用失败：${finishReason.failure?.message ?? "未知错误"}`);
  }
  if (out.trim().length === 0) throw new Error("模型没有返回内容");
  return out;
}

/** 日志里只显示 moli 路径是否存在，不回显完整路径（可能含用户名）。 */
function moliPathLabel(config) {
  return String(config.moliPath ?? "").trim() === "" ? "未配置" : "已配置";
}
