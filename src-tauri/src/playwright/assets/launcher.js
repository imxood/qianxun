/**
 * qx-playwright launcher —— @playwright/mcp 的 stdio 前置外壳。
 *
 * moli 不是 Chromium 套壳（子命令式 CLI，吃不下 Chromium 启动开关，
 * executablePath 直拉必败：clap UnknownArgument，exit 2）。唯一正确
 * 姿势是 `moli serve --layout` + CDP 附着。本外壳：
 * - moli 可用（QX_MOLI_PATH 命中且文件存在）：复用/拉起 moli serve
 *   --layout，向 MCP server 传 --cdp-endpoint；
 * - 否则 Windows 传 --browser msedge --headless（系统 Edge）；
 * - 随后 spawn @playwright/mcp cli（stdio inherit 透传协议），
 *   退出时回收自己拉起的 serve 进程（复用的不回收，归其属主）。
 */
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";

const MOLI_READY_MS = 15_000;
const PORT_BASE = 19310;
const PORT_ATTEMPTS = 5;

function log(line) {
  process.stderr.write(`[qx-playwright] ${line}\n`);
}

async function cdpReady(endpoint) {
  try {
    const response = await fetch(`${endpoint}/json/version`);
    return response.ok;
  } catch {
    return false;
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** 复用既存健康 serve，否则找空闲端口拉起新的。返回 null child = 复用。 */
async function ensureMoliServe(moliPath) {
  for (let offset = 0; offset < PORT_ATTEMPTS; offset++) {
    const endpoint = `http://127.0.0.1:${PORT_BASE + offset}`;
    if (await cdpReady(endpoint)) {
      log(`复用既存 moli serve：${endpoint}`);
      return { endpoint, child: null };
    }
  }
  for (let offset = 0; offset < PORT_ATTEMPTS; offset++) {
    const port = PORT_BASE + offset;
    const endpoint = `http://127.0.0.1:${port}`;
    const child = spawn(moliPath, ["serve", "--layout", "--port", String(port)], {
      stdio: "ignore",
      windowsHide: true,
    });
    const deadline = Date.now() + MOLI_READY_MS;
    while (Date.now() < deadline) {
      if (child.exitCode !== null) break;
      if (await cdpReady(endpoint)) {
        log(`moli serve 就绪：${endpoint}`);
        return { endpoint, child };
      }
      await sleep(150);
    }
    try {
      child.kill();
    } catch {
      /* 已退出 */
    }
    if (child.exitCode !== null) {
      throw new Error(`moli serve 提前退出（code ${child.exitCode}）——检查 moli 版本是否支持 serve --layout`);
    }
  }
  throw new Error(`moli serve 端口分配失败（${PORT_BASE}-${PORT_BASE + PORT_ATTEMPTS - 1} 均不可用）`);
}

const moliPath = String(process.env.QX_MOLI_PATH ?? "").trim();
// cli.js 不在包 exports 里：解析包根再拼（bin 入口）。
const require = createRequire(import.meta.url);
const mcpRoot = dirname(require.resolve("@playwright/mcp/package.json"));
const args = [join(mcpRoot, "cli.js")];
let serve = null;
if (moliPath && existsSync(moliPath)) {
  serve = await ensureMoliServe(moliPath);
  args.push("--cdp-endpoint", serve.endpoint);
} else if (process.platform === "win32") {
  log("moli 未配置或不存在 → 系统 Edge（msedge 通道）");
  args.push("--browser", "msedge", "--headless");
  // 登录态持久化（QX_MCP_PROFILE_DIR 由千寻 patch 注入；缺省走 MCP 默认）。
  const profileDir = String(process.env.QX_MCP_PROFILE_DIR ?? "").trim();
  if (profileDir) args.push("--user-data-dir", profileDir);
} else {
  log("moli 未配置 → Playwright 默认 chromium");
}

const cli = spawn(process.execPath, args, { stdio: "inherit", windowsHide: true });

function cleanup() {
  try {
    serve?.child?.kill();
  } catch {
    /* 已退出 */
  }
}
process.on("exit", cleanup);
process.on("SIGINT", () => {
  cleanup();
  process.exit(130);
});
process.on("SIGTERM", () => {
  cleanup();
  process.exit(143);
});
cli.on("exit", (code) => {
  cleanup();
  process.exit(code ?? 0);
});
