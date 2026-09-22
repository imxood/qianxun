import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { apply } from "./index.js";

/**
 * apply() HTTP 通道装配回归（webServer 路由未挂载修复）：
 * - 旧写法在 apply 里 ctx.get("webServer") 同步取值，服务未 ACTIVE 时拿到
 *   undefined 被 if 静默跳过 → /qx/web/*、/qx/notes/organize 从未挂载；
 * - 新写法走 ctx.inject(deps, cb)：deps 缺失/未就绪时 cb 不执行，就绪后
 *   cordis 以内层 fiber 装载 cb，路由经 http.effect 注册（可随 fiber 注销）。
 * 这里用最小 mock 断言装配形状与 handler 的 HTTP 语义（全部离线）。
 */

interface RecordedRoute {
  kind: string;
  path: string;
  handler: (req: unknown, res: unknown) => Promise<void> | void;
}

function makeOuterCtx() {
  const tools: Array<{ name: string }> = [];
  const sections: Array<{ name: string }> = [];
  const injects: Array<{ deps: string[]; cb: (http: unknown) => void }> = [];
  const logs: string[] = [];
  const ctx = {
    systemPrompt: { section: (s: { name: string }) => sections.push(s) },
    tools: { register: (t: { name: string }) => tools.push(t) },
    logger: { info: (msg: string) => logs.push(msg) },
    inject: (deps: string[], cb: (http: unknown) => void) => injects.push({ deps, cb }),
  };
  return { ctx, tools, sections, injects, logs };
}

/** 模拟内层 fiber 的 ctx：effect 立即执行并收集 disposer。 */
function makeInnerCtx() {
  const routes: RecordedRoute[] = [];
  const http = {
    webServer: {
      register: (route: RecordedRoute) => {
        routes.push(route);
        return () => undefined;
      },
    },
    effect: (setup: () => unknown) => {
      setup();
      return () => undefined;
    },
    logger: { info: () => undefined },
    llm: {},
  };
  return { http, routes };
}

function makeVaultConfig() {
  return { vault: mkdtempSync(join(tmpdir(), "qx-apply-")), moliPath: "", playwrightCorePath: "" };
}

/** 模拟 node:http 的 req/res（够 handler 用）。rawBody 非空时按原样发送。 */
function makeReq(method: string, body?: unknown, rawBody?: Buffer) {
  const chunks = rawBody ? [rawBody] : body === undefined ? [] : [Buffer.from(JSON.stringify(body))];
  return {
    method,
    async *[Symbol.asyncIterator]() {
      for (const chunk of chunks) yield chunk;
    },
  };
}

function makeRes() {
  const headers: Record<string, string | string[]> = {};
  let status: number | undefined;
  let statusHeaders: Record<string, unknown> | undefined;
  let body = "";
  const res = {
    setHeader: (name: string, value: string | string[]) => {
      headers[name] = value;
    },
    writeHead: (code: number, head?: Record<string, unknown>) => {
      status = code;
      statusHeaders = head;
    },
    end: (text?: string) => {
      body = text ?? "";
    },
  };
  return {
    res,
    headers,
    get status() {
      return status;
    },
    get statusHeaders() {
      return statusHeaders;
    },
    get body() {
      return body;
    },
    json: () => JSON.parse(body) as unknown,
  };
}

describe("apply HTTP 通道装配", () => {
  it("webServer/llm 走 ctx.inject 声明式依赖（不再 apply 时同步取值）", () => {
    const { ctx, injects, tools } = makeOuterCtx();
    apply(ctx as never, makeVaultConfig());
    expect(injects.map((entry) => entry.deps)).toEqual([["webServer"], ["webServer", "llm"]]);
    // 工具注册不依赖可选服务：内层未装载时工具也应已就位。
    const names = tools.map((tool) => tool.name);
    expect(names).toContain("web_search");
    expect(names).toContain("web_read");
    expect(names).toContain("browser_open");
  });

  it("内层装载挂三条路由：/qx/web/search、/qx/web/read、/qx/notes/organize", () => {
    const { ctx, injects } = makeOuterCtx();
    apply(ctx as never, makeVaultConfig());
    const first = makeInnerCtx();
    injects[0].cb(first.http);
    expect(first.routes.map((route) => route.path)).toEqual(["/qx/web/search", "/qx/web/read"]);

    const second = makeInnerCtx();
    injects[1].cb(second.http);
    expect(second.routes.map((route) => route.path)).toEqual(["/qx/notes/organize"]);
    expect(second.routes[0].kind).toBe("exact");
  });

  it("handler：OPTIONS → 204；GET → 405 JSON + CORS", async () => {
    const { ctx, injects } = makeOuterCtx();
    apply(ctx as never, makeVaultConfig());
    const { http, routes } = makeInnerCtx();
    injects[0].cb(http);
    const read = routes.find((route) => route.path === "/qx/web/read")!;

    const preflight = makeRes();
    await read.handler(makeReq("OPTIONS"), preflight.res);
    expect(preflight.status).toBe(204);
    expect(preflight.headers["Access-Control-Allow-Origin"]).toBe("*");

    const wrong = makeRes();
    await read.handler(makeReq("GET"), wrong.res);
    expect(wrong.status).toBe(405);
    expect((wrong.statusHeaders as Record<string, string>)["Content-Type"]).toBe("application/json");
    expect((wrong.json() as { ok: boolean }).ok).toBe(false);
  });

  it("handler：非法 JSON → 400；空查询 → 500（离线可判定）", async () => {
    const { ctx, injects } = makeOuterCtx();
    apply(ctx as never, makeVaultConfig());
    const { http, routes } = makeInnerCtx();
    injects[0].cb(http);
    const search = routes.find((route) => route.path === "/qx/web/search")!;

    const bad = makeRes();
    await search.handler(makeReq("POST", undefined, Buffer.from("not-json")), bad.res);
    expect(bad.status).toBe(400);
    expect((bad.json() as { ok: boolean }).ok).toBe(false);

    // 空 query：web.search → buildSearchUrl 抛「搜索词不能为空」→ 500。
    const empty = makeRes();
    await search.handler(makeReq("POST", { query: "   " }), empty.res);
    expect(empty.status).toBe(500);
    expect((empty.json() as { ok: boolean; error: string }).ok).toBe(false);
    expect((empty.json() as { error: string }).error).toMatch(/搜索词/);
  });
});
