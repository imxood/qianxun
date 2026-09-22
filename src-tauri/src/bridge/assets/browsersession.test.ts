import { describe, expect, it, vi } from "vitest";
import { createBrowserAutomation } from "./browsersession.js";

/**
 * 浏览器自动化状态机真测（R001 D11）：connector 与 serve 全部注入假件，
 * 验证会话复用 / 空闲关停 / URL 校验 / 每个操作的调用链。真实 moli +
 * playwright-core 链路由带 QX_TEST_MOLI 的集成测试覆盖（本机跑）。
 */

function makeFakePage() {
  return {
    title: vi.fn().mockResolvedValue("示例页"),
    goto: vi.fn().mockResolvedValue(undefined),
    click: vi.fn().mockResolvedValue(undefined),
    fill: vi.fn().mockResolvedValue(undefined),
    screenshot: vi.fn().mockResolvedValue(undefined),
    $$eval: vi.fn().mockResolvedValue(["a#link 首页", "button#submit 提交"]),
  };
}

function makeDeps(overrides?: Record<string, unknown>) {
  const page = makeFakePage();
  const browser = {
    contexts: () => [{ pages: () => [page] }],
    close: vi.fn().mockResolvedValue(undefined),
  };
  const stop = vi.fn();
  return {
    page,
    browser,
    stop,
    loadChromium: vi.fn().mockResolvedValue({
      connectOverCDP: vi.fn().mockResolvedValue(browser),
    }),
    startServer: vi.fn().mockResolvedValue({ endpoint: "http://127.0.0.1:19999", stop }),
    idleMs: 0, // 默认禁用空闲关停；关停用例单独开
    ...overrides,
  };
}

describe("createBrowserAutomation", () => {
  it("open 拉起 serve 并连接，URL 校验拒绝非 http(s)", async () => {
    const deps = makeDeps();
    const automation = createBrowserAutomation(deps);
    await expect(automation.open("file:///etc/passwd")).rejects.toThrow(/http/);
    expect(deps.startServer).not.toHaveBeenCalled();

    const title = await automation.open("https://example.com/");
    expect(title).toBe("示例页");
    expect(deps.startServer).toHaveBeenCalledTimes(1);
    expect(deps.page.goto).toHaveBeenCalledWith("https://example.com/", {
      waitUntil: "domcontentloaded",
      timeout: 20_000,
    });
  });

  it("连续操作复用同一会话，close 后再次 open 重建", async () => {
    const deps = makeDeps();
    const automation = createBrowserAutomation(deps);
    await automation.open("https://example.com/");
    await automation.snapshot();
    await automation.click("#submit");
    expect(deps.startServer).toHaveBeenCalledTimes(1);
    expect(deps.page.click).toHaveBeenCalledWith("#submit", { timeout: 10_000 });

    await automation.close();
    expect(deps.browser.close).toHaveBeenCalled();
    expect(deps.stop).toHaveBeenCalled();

    await automation.open("https://example.com/again");
    expect(deps.startServer).toHaveBeenCalledTimes(2);
  });

  it("snapshot 返回元素清单，fill 带超时传参", async () => {
    const deps = makeDeps();
    const automation = createBrowserAutomation(deps);
    const list = await automation.snapshot();
    expect(list).toContain("a#link 首页");
    await automation.fill("#kw", "rust 入门指南");
    expect(deps.page.fill).toHaveBeenCalledWith("#kw", "rust 入门指南", {
      timeout: 10_000,
    });
  });

  it("screenshot 传路径给 page.screenshot", async () => {
    const deps = makeDeps();
    const automation = createBrowserAutomation(deps);
    const saved = await automation.screenshot("C:/tmp/shot.png");
    expect(saved).toBe("C:/tmp/shot.png");
    expect(deps.page.screenshot).toHaveBeenCalledWith({
      path: "C:/tmp/shot.png",
      timeout: 10_000,
    });
  });

  it("空闲超时自动关停会话与 serve", async () => {
    vi.useFakeTimers();
    try {
      const deps = makeDeps({ idleMs: 1_000 });
      const automation = createBrowserAutomation(deps);
      await automation.open("https://example.com/");
      await vi.advanceTimersByTimeAsync(1_500);
      expect(deps.browser.close).toHaveBeenCalled();
      expect(deps.stop).toHaveBeenCalled();
      expect(automation.is_open()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("连接失败时回收 serve 进程", async () => {
    const deps = makeDeps();
    deps.loadChromium = vi.fn().mockRejectedValue(new Error("cdp down"));
    const automation = createBrowserAutomation(deps);
    await expect(automation.open("https://example.com/")).rejects.toThrow("cdp down");
    expect(deps.stop).toHaveBeenCalled();
    expect(automation.is_open()).toBe(false);
  });
});
