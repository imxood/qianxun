// Track E E2E — Day 4 daemon endpoint smoke test.
//
// Why Node.js?  Bash tool puts each command in a Windows Job Object;
// detached children (qx.exe) get killed when bash returns.  Node's
// child_process.spawn gives qx.exe a parent in this script's PID, so
// the daemon lives as long as the script — even if the bash call
// wrapping this node process exits while we're still waiting on curls.
//
// Run:  node scripts/e2e/track-e-daemon-e2e.mjs

import { spawn } from "node:child_process";
import { createHmac } from "node:crypto";
import { setTimeout as sleep } from "node:timers/promises";
import process from "node:process";

// Strip proxy so 127.0.0.1 doesn't go through system proxy.
delete process.env.http_proxy;
delete process.env.HTTP_PROXY;
delete process.env.https_proxy;
delete process.env.HTTPS_PROXY;

const PORT = process.env.QX_E2E_PORT || "23900";
const HOST = "127.0.0.1";
// Stage 10a 起: daemon 从 ~/.qianxun/admin.cred 读 token_secret (32-byte base64),
// QIANXUN_JWT_SECRET env var 已被 ignore. 我们直接读 cred 文件取 secret 签 JWT.
const ADMIN_CRED_PATH =
  process.env.QIANXUN_ADMIN_CRED ||
  `${process.env.USERPROFILE || process.env.HOME}\\.qianxun\\admin.cred`.replace(
    /\\\\/g,
    "\\",
  );
const QX_BIN =
  process.env.QX_BIN || "E:/git/maxu/qianxun/target/debug/qx.exe";

// ── JWT mint (HS256, no deps) ─────────────────────────────
function b64url(buf) {
  return Buffer.from(buf)
    .toString("base64")
    .replace(/=+$/, "")
    .replace(/\+/g, "-")
    .replace(/\//g, "_");
}
function mintJwt(secret) {
  const header = { alg: "HS256", typ: "JWT" };
  const payload = {
    sub: "admin",
    exp: Math.floor(Date.now() / 1000) + 3600,
    iat: Math.floor(Date.now() / 1000),
  };
  const h = b64url(JSON.stringify(header));
  const p = b64url(JSON.stringify(payload));
  const sig = b64url(
    createHmac("sha256", secret).update(`${h}.${p}`).digest(),
  );
  return `${h}.${p}.${sig}`;
}

// ── HTTP helper ───────────────────────────────────────────
async function get(path, token) {
  const url = `http://${HOST}:${PORT}${path}`;
  const res = await fetch(url, {
    headers: { Authorization: `Bearer ${token}` },
  });
  const text = await res.text();
  let json = null;
  try {
    json = JSON.parse(text);
  } catch {
    // non-json
  }
  return { status: res.status, body: json, raw: text };
}

// ── Wait for /v1/system/health ────────────────────────────
async function waitForHealth(maxSec = 15) {
  const deadline = Date.now() + maxSec * 1000;
  while (Date.now() < deadline) {
    try {
      const r = await fetch(`http://${HOST}:${PORT}/v1/system/health`, {
        signal: AbortSignal.timeout(1500),
      });
      if (r.status < 500) return true;
    } catch {
      // daemon still booting
    }
    await sleep(300);
  }
  return false;
}

// ── Daemon spawn ──────────────────────────────────────────
const child = spawn(
  QX_BIN,
  ["--daemon", "--port", PORT],
  {
    env: {
      ...process.env,
      RUST_LOG: "info,qianxun=debug",
    },
    stdio: ["ignore", "pipe", "pipe"],
  },
);

let daemonLog = "";
child.stdout.on("data", (b) => (daemonLog += b.toString()));
child.stderr.on("data", (b) => (daemonLog += b.toString()));
child.on("exit", (code, sig) => {
  console.error(`[e2e] daemon exited code=${code} sig=${sig}`);
});

const results = { health: null, tools: null, skills: null, memory: null };
let exitCode = 1;

let SECRET;
try {
  // Read admin.cred file (Stage 10a token_secret).
  const fs = await import("node:fs/promises");
  const credJson = JSON.parse(await fs.readFile(ADMIN_CRED_PATH, "utf8"));
  SECRET = credJson.token_secret;
  if (!SECRET) {
    throw new Error("token_secret not found in admin.cred");
  }
  console.log(`[e2e] loaded token_secret (${SECRET.length} chars) from ${ADMIN_CRED_PATH}`);
} catch (err) {
  console.error(`[e2e] failed to read admin.cred at ${ADMIN_CRED_PATH}: ${err.message}`);
  console.error(`[e2e] Run \`qx --daemon\` once first to bootstrap, or set QIANXUN_ADMIN_CRED.`);
  process.exit(2);
}

try {
  const ready = await waitForHealth(20);
  if (!ready) {
    console.error("[e2e] daemon failed to become healthy in 20s");
    console.error("[e2e] daemon log tail:\n" + daemonLog.slice(-2000));
    process.exit(2);
  }
  results.health = "ok";

  const jwt = mintJwt(SECRET);

  // 1) GET /v1/tools
  const tools = await get("/v1/tools", jwt);
  results.tools = {
    status: tools.status,
    count: tools.body?.tools?.length ?? null,
    sample: tools.body?.tools?.slice(0, 3) ?? null,
  };

  // 2) GET /v1/skills
  const skills = await get("/v1/skills", jwt);
  results.skills = {
    status: skills.status,
    count: skills.body?.count ?? null,
    names: skills.body?.skills ?? null,
  };

  // 3) GET /v1/memory/ping
  const mem = await get("/v1/memory/ping", jwt);
  results.memory = {
    status: mem.status,
    pingStatus: mem.body?.status ?? null,
    body: mem.body,
  };

  // ── Verdict ───────────────────────────────────────────
  const ok =
    results.tools.count !== null && results.tools.count >= 8 &&
    results.skills.count !== null && results.skills.count >= 1 &&
    results.memory.pingStatus === "ok";
  exitCode = ok ? 0 : 1;

  console.log("=== Track E Daemon E2E Results ===");
  console.log(JSON.stringify(results, null, 2));
  console.log(`\n[e2e] verdict: ${ok ? "PASS" : "FAIL"}`);
  if (!ok) {
    console.error("[e2e] failing criteria:");
    if (results.tools.count < 8) {
      console.error(`  - /v1/tools count=${results.tools.count} (expected >= 8)`);
    }
    if (results.skills.count < 1) {
      console.error(`  - /v1/skills count=${results.skills.count} (expected >= 1)`);
    }
    if (results.memory.pingStatus !== "ok") {
      console.error(
        `  - /v1/memory/ping status=${results.memory.pingStatus} (expected "ok")`,
      );
    }
  }
} catch (err) {
  console.error("[e2e] error:", err);
  console.error("[e2e] daemon log tail:\n" + daemonLog.slice(-2000));
  exitCode = 3;
} finally {
  // Always kill daemon before exiting so Job Object doesn't keep it alive.
  try {
    child.kill("SIGTERM");
    await sleep(500);
    if (!child.killed) child.kill("SIGKILL");
  } catch (e) {
    console.error("[e2e] kill failed:", e);
  }
  // Write results to a file so bash can read them
  const fs = await import("node:fs/promises");
  await fs.writeFile(
    "scripts/e2e/track-e-results.json",
    JSON.stringify({ results, exitCode, daemonLogTail: daemonLog.slice(-2000) }, null, 2),
  );
  process.exit(exitCode);
}
