import { spawn } from "node:child_process";
import { describe, expect, it } from "vitest";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

/**
 * launcher 冒烟（Windows）：msedge 兜底路径真实拉起 @playwright/mcp，
 * MCP 握手 + 工具清单可用即证明 stdio 透传与参数装配正确。moli 路径
 * （QX_MOLI_PATH → serve --layout → --cdp-endpoint）需要本机 moli，
 * 由带 QX_TEST_MOLI 的用例覆盖。
 */
const here = dirname(fileURLToPath(import.meta.url));
const launcher = join(here, "launcher.js");

interface Pending {
  resolve: (msg: Record<string, unknown>) => void;
}
function makeRpc(child: ReturnType<typeof spawn>) {
  let buf = "";
  const pending = new Map<number, Pending>();
  let nextId = 1;
  child.stdout!.on("data", (chunk: Buffer) => {
    buf += chunk.toString("utf8");
    let index: number;
    while ((index = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, index).trim();
      buf = buf.slice(index + 1);
      if (!line) continue;
      try {
        const msg = JSON.parse(line) as { id?: number };
        if (typeof msg.id === "number" && pending.has(msg.id)) {
          pending.get(msg.id)!.resolve(msg);
          pending.delete(msg.id);
        }
      } catch {
        /* 忽略非 JSON 行 */
      }
    }
  });
  return (method: string, params?: unknown) => {
    const id = nextId++;
    return new Promise<Record<string, unknown>>((resolve) => {
      pending.set(id, { resolve });
      child.stdin!.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
    });
  };
}

describe.skipIf(process.platform !== "win32")("qx-playwright launcher", () => {
  it("msedge 兜底：MCP 握手成功且注册完整浏览器工具面", { timeout: 60_000 }, async () => {
    const child = spawn("node", [launcher], {
      env: { ...process.env, QX_MOLI_PATH: "" },
      stdio: ["pipe", "pipe", "pipe"],
    });
    try {
      const rpc = makeRpc(child);
      const init = (await rpc("initialize", {
        protocolVersion: "2025-06-18",
        capabilities: {},
        clientInfo: { name: "qx-test", version: "0.0.1" },
      })) as { result?: { serverInfo?: { name?: string } } };
      expect(init.result?.serverInfo?.name).toBeTruthy();
      await rpc("notifications/initialized", {});
      const tools = (await rpc("tools/list", {})) as { result?: { tools?: { name: string }[] } };
      const names = (tools.result?.tools ?? []).map((tool) => tool.name);
      expect(names).toContain("browser_navigate");
      expect(names).toContain("browser_snapshot");
      expect(names).toContain("browser_click");
      expect(names.length).toBeGreaterThan(20);
    } finally {
      child.kill();
    }
  });

  it.skipIf(!process.env.QX_TEST_MOLI)(
    "moli 路径：QX_TEST_MOLI 命中时传 --cdp-endpoint（需本机 moli）",
    { timeout: 90_000 },
    async () => {
      const child = spawn("node", [launcher], {
        env: { ...process.env, QX_MOLI_PATH: process.env.QX_TEST_MOLI! },
        stdio: ["pipe", "pipe", "pipe"],
      });
      try {
        const rpc = makeRpc(child);
        const init = (await rpc("initialize", {
          protocolVersion: "2025-06-18",
          capabilities: {},
          clientInfo: { name: "qx-test", version: "0.0.1" },
        })) as { result?: { serverInfo?: { name?: string } } };
        expect(init.result?.serverInfo?.name).toBeTruthy();
        const tools = (await rpc("tools/list", {})) as { result?: { tools?: { name: string }[] } };
        expect((tools.result?.tools ?? []).length).toBeGreaterThan(20);
      } finally {
        child.kill();
      }
    },
  );
});
