// 千寻 VPS Web UI — fetchWithAuth 单测 (vitest).
//
// Stage 6c: 验证 Authorization: Bearer <token> 头被正确添加.
// 其他模块 (loadProjects / sendMessage / SSE 解析) Stage 7+ 加.
//
// 测试策略:
//   - 在 vm context 里重新执行 app.js 源文本 (用 Function 构造器 + 注入 window/document/localStorage/fetch)
//   - 通过 window.qxvps.fetchWithAuth 触发 mock fetch, 抓取 headers
//
// vitest 跑: `pnpm test` 或 `npx vitest run` (vitest.config.js 配置 include).

import { describe, it, expect, vi } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const APP_JS_PATH = path.join(__dirname, "app.js");
const APP_JS_SRC = fs.readFileSync(APP_JS_PATH, "utf-8");

/// 把 app.js 当成脚本在 vm 上下文里跑, 暴露 window.qxvps.
/// 注入可配置的 localStorage + fetch 实现.
function loadAppWith({ token = null, fetchImpl = null } = {}) {
  const storage = {};
  if (token) storage.qxvps_token = token;

  const sandboxWindow = {};
  const sandboxDocument = {
    addEventListener: () => {},        // 不触发 DOMContentLoaded — 测试不依赖它
    getElementById: () => ({
      classList: { add: () => {}, remove: () => {}, toggle: () => {} },
      addEventListener: () => {},
      value: "",
      textContent: "",
      innerHTML: "",
      appendChild: () => {},
      querySelector: () => null,
      querySelectorAll: () => [],
    }),
  };
  const sandboxLocalStorage = {
    _data: storage,
    getItem(k) { return Object.prototype.hasOwnProperty.call(this._data, k) ? this._data[k] : null; },
    setItem(k, v) { this._data[k] = String(v); },
    removeItem(k) { delete this._data[k]; },
  };
  const sandboxFetch = fetchImpl || (() => Promise.resolve({
    ok: true, status: 200, statusText: "OK",
    text: async () => "", json: async () => ({}),
  }));

  const fn = new Function(
    "window", "document", "localStorage", "fetch", "TextDecoder", "AbortController",
    APP_JS_SRC + "\nreturn window.qxvps;"
  );
  return fn(
    sandboxWindow, sandboxDocument, sandboxLocalStorage, sandboxFetch,
    globalThis.TextDecoder, globalThis.AbortController
  );
}

describe("app.js — qxvps global API", () => {
  it("exposes the expected surface (setToken, showApp, showLogin, fetchWithAuth, getToken)", () => {
    const qxvps = loadAppWith();
    expect(qxvps.setToken).toBeTypeOf("function");
    expect(qxvps.showApp).toBeTypeOf("function");
    expect(qxvps.showLogin).toBeTypeOf("function");
    expect(qxvps.fetchWithAuth).toBeTypeOf("function");
    expect(qxvps.getToken).toBeTypeOf("function");
  });

  it("localStorage token round-trips through setToken", () => {
    const storage = { qxvps_token: "old" };
    const sandboxWindow = {};
    const sandboxDocument = { addEventListener: () => {}, getElementById: () => ({
      classList: { add: () => {}, remove: () => {}, toggle: () => {} },
      addEventListener: () => {}, value: "", textContent: "", innerHTML: "",
      appendChild: () => {}, querySelector: () => null, querySelectorAll: () => [],
    }) };
    const sandboxLocalStorage = {
      _data: storage,
      getItem(k) { return Object.prototype.hasOwnProperty.call(this._data, k) ? this._data[k] : null; },
      setItem(k, v) { this._data[k] = String(v); },
      removeItem(k) { delete this._data[k]; },
    };
    const fn = new Function(
      "window", "document", "localStorage", "fetch", "TextDecoder", "AbortController",
      APP_JS_SRC + "\nreturn window.qxvps;"
    );
    const qxvps = fn(
      sandboxWindow, sandboxDocument, sandboxLocalStorage,
      () => Promise.resolve({ ok: true, status: 200, text: async () => "", json: async () => ({}) }),
      globalThis.TextDecoder, globalThis.AbortController
    );
    qxvps.setToken("new-jwt-xyz");
    expect(storage.qxvps_token).toBe("new-jwt-xyz");
    expect(qxvps.getToken()).toBe("new-jwt-xyz");
  });
});

describe("fetchWithAuth", () => {
  it("adds Authorization Bearer header when token present", async () => {
    let captured = null;
    const mockFetch = vi.fn(async (_url, opts) => {
      captured = { url: _url, opts };
      return { ok: true, status: 200, statusText: "OK", text: async () => "", json: async () => ({}) };
    });

    const qxvps = loadAppWith({ token: "test-jwt-abc123", fetchImpl: mockFetch });
    const res = await qxvps.fetchWithAuth("/api/projects", { method: "GET" });

    expect(res).toBeDefined();
    expect(res.status).toBe(200);
    expect(captured).not.toBeNull();
    expect(captured.url).toBe("/api/projects");
    expect(captured.opts.method).toBe("GET");
    expect(captured.opts.headers["Authorization"]).toBe("Bearer test-jwt-abc123");
  });

  it("does NOT add Authorization header when no token", async () => {
    let captured = null;
    const mockFetch = vi.fn(async (_url, opts) => {
      captured = { url: _url, opts };
      return { ok: true, status: 200, statusText: "OK", text: async () => "", json: async () => ({}) };
    });

    const qxvps = loadAppWith({ token: null, fetchImpl: mockFetch });
    await qxvps.fetchWithAuth("/api/health", { method: "GET" });

    expect(captured).not.toBeNull();
    expect(captured.opts.headers["Authorization"]).toBeUndefined();
  });

  it("serializes object body to JSON with Content-Type", async () => {
    let captured = null;
    const mockFetch = vi.fn(async (_url, opts) => {
      captured = { url: _url, opts };
      return { ok: true, status: 200, statusText: "OK", text: async () => "", json: async () => ({}) };
    });

    const qxvps = loadAppWith({ token: "tok", fetchImpl: mockFetch });
    await qxvps.fetchWithAuth("/api/foo", {
      method: "POST",
      body: { hello: "world" },
    });

    expect(captured.opts.headers["Content-Type"]).toBe("application/json");
    expect(captured.opts.body).toBe('{"hello":"world"}');
  });

  it("on 401, clears token and throws", async () => {
    const mockFetch = vi.fn(async () => ({
      ok: false, status: 401, statusText: "Unauthorized",
      text: async () => "auth required", json: async () => ({}),
    }));
    const storage = { qxvps_token: "will-be-cleared" };
    const sandboxWindow = {};
    const sandboxDocument = { addEventListener: () => {}, getElementById: () => ({
      classList: { add: () => {}, remove: () => {}, toggle: () => {} },
      addEventListener: () => {}, value: "", textContent: "", innerHTML: "",
      appendChild: () => {}, querySelector: () => null, querySelectorAll: () => [],
    }) };
    const sandboxLocalStorage = {
      _data: storage,
      getItem(k) { return Object.prototype.hasOwnProperty.call(this._data, k) ? this._data[k] : null; },
      setItem(k, v) { this._data[k] = String(v); },
      removeItem(k) { delete this._data[k]; },
    };
    const fn = new Function(
      "window", "document", "localStorage", "fetch", "TextDecoder", "AbortController",
      APP_JS_SRC + "\nreturn window.qxvps;"
    );
    const qxvps = fn(
      sandboxWindow, sandboxDocument, sandboxLocalStorage, mockFetch,
      globalThis.TextDecoder, globalThis.AbortController
    );

    await expect(qxvps.fetchWithAuth("/api/x", { method: "GET" })).rejects.toThrow("401");
    expect(storage.qxvps_token).toBeUndefined();
  });
});
