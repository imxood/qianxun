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
import { randomUUID } from "node:crypto";
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
};

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

  // ---- AI 整理 HTTP 通道（可选服务：webServer / llm 任一缺失则跳过） ----
  const webServer = ctx.get("webServer");
  const llm = ctx.get("llm");
  if (webServer !== undefined && llm !== undefined) {
    ctx.effect(() =>
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
    ctx.logger.info(`qx-bridge: ${ORGANIZE_ENDPOINT} 就绪（库：${vault.root}）`);
  }

  ctx.logger.info(`qx-bridge: 笔记工具就绪（库：${vault.root}）`);
}

// ---- AI 整理 ----

async function organize(llm, vault, body) {
  const instruction = String(body.instruction ?? "").trim();
  if (!instruction) throw new Error("整理指令不能为空");
  const providers = [...llm.adapters.keys()];
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
    purpose: "qx-notes-organize",
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
