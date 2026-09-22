import http from "node:http";
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { createBrowserAutomation } from "./browsersession.js";

/**
 * 真实集成测试（R001 D11）：playwright-core + moli serve --layout 全链路。
 *
 * 门控：设置 QX_MOLI_PATH 指向 moli 可执行文件才运行（CI 无 moli 时
 * 自然跳过，不算假测试——状态机会被 browsersession.test.ts 的假件覆盖）。
 *   QX_TEST_MOLI=D:\programs\moli-...\moli.exe pnpm vitest run browsersession.integration
 */

const moliPath = process.env.QX_TEST_MOLI?.trim() ?? "";

/**
 * serve 就绪探测与 createBrowserAutomation 的 startServer 同款超时语义：
 * 这里不复用桥内实现，避免测试依赖 index.js（装配层）。
 */
function startServe(moli: string) {
  return async () => {
    const port = 19490 + Math.floor(Math.random() * 100);
    const child = spawn(moli, ["serve", "--layout", "--port", String(port)], {
      stdio: "ignore",
      windowsHide: true,
    });
    const endpoint = `http://127.0.0.1:${port}`;
    const deadline = Date.now() + 15_000;
    for (;;) {
      if (child.exitCode !== null) throw new Error(`moli serve 提前退出（code ${child.exitCode}）`);
      try {
        const response = await fetch(`${endpoint}/json/version`);
        if (response.ok) break;
      } catch {
        // 未就绪继续等。
      }
      if (Date.now() > deadline) {
        child.kill();
        throw new Error("moli serve 15s 未就绪");
      }
      await new Promise((resolve) => setTimeout(resolve, 150));
    }
    return { endpoint, stop: () => child.kill() };
  };
}

/** 一次性本地页：比 data: URL 更真实，且走 http(s) 守卫。 */
async function withLocalPage(html, run) {
  const server = http.createServer((_req, res) => {
    res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    res.end(html);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  const url = `http://127.0.0.1:${address.port}/`;
  try {
    await run(url);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}

describe.skipIf(moliPath === "")("playwright + moli 真实链路", () => {
  it("open → snapshot → click → fill → screenshot 全链路", { timeout: 60_000 }, async () => {
    const require = createRequire(import.meta.url);
    const mod = require("playwright-core");
    const chromium = mod.chromium ?? mod.default?.chromium ?? mod;
    const automation = createBrowserAutomation({
      loadChromium: async () => chromium,
      startServer: startServe(moliPath),
      idleMs: 30_000,
    });
    try {
      // 打开带交互的本地页（离线、无外网依赖，且过 http 守卫）。
      const html =
        "<title>moli-it</title><input id=kw><button id=go>go</button>";
      await withLocalPage(html, async (url) => {
        const title = await automation.open(url);
        expect(title).toBe("moli-it");

        // snapshot 列出可交互元素。
        const list = await automation.snapshot();
        expect(list).toContain("#kw");
        expect(list).toContain("#go");

        // fill + click 真实坐标输入（--layout 几何）。
        await automation.fill("#kw", "rust 入门指南");
        await automation.click("#go");
      });

      // 4. screenshot 落盘（--layout 软件渲染）。
      const shot = path.join(process.env.TEMP ?? ".", `qx-moli-it-${Date.now()}.png`);
      const saved = await automation.screenshot(shot);
      const { statSync, rmSync } = await import("node:fs");
      expect(statSync(saved).size).toBeGreaterThan(0);
      rmSync(saved, { force: true });
    } finally {
      await automation.close();
    }
  });

  it("close 后 serve 进程退出", { timeout: 30_000 }, async () => {
    const require = createRequire(import.meta.url);
    const mod = require("playwright-core");
    const chromium = mod.chromium ?? mod.default?.chromium ?? mod;
    const automation = createBrowserAutomation({
      loadChromium: async () => chromium,
      startServer: startServe(moliPath),
    });
    await withLocalPage("<title>x</title>", async (url) => {
      await automation.open(url);
    });
    expect(automation.is_open()).toBe(true);
    await automation.close();
    expect(automation.is_open()).toBe(false);
  });
});
