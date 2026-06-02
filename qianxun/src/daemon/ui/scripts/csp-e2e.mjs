// quick e2e for CSP header — spawn daemon, curl health, kill daemon
import { spawn } from 'node:child_process';
import { promisify } from 'node:util';
import { setTimeout as sleep } from 'node:timers/promises';

const fetch = globalThis.fetch;

const daemon = spawn('E:\\git\\maxu\\qianxun\\target\\debug\\qx.exe', [
  '--daemon',
  '--port', '23920'
], {
  env: { ...process.env, QIANXUN_JWT_SECRET: 'test-jwt-secret-please-ignore-32-bytes' },
  stdio: ['ignore', 'pipe', 'pipe']
});

let daemonLog = '';
daemon.stdout.on('data', (d) => { daemonLog += d.toString(); });
daemon.stderr.on('data', (d) => { daemonLog += d.toString(); });

// wait for daemon to be ready (up to 5s)
let ready = false;
for (let i = 0; i < 50; i++) {
  await sleep(100);
  try {
    const r = await fetch('http://127.0.0.1:23920/v1/system/health');
    if (r.ok) { ready = true; break; }
  } catch { /* not yet */ }
}

if (!ready) {
  console.error('[csp-e2e] daemon not ready, log:');
  console.error(daemonLog);
  daemon.kill('SIGKILL');
  process.exit(1);
}

// ── Test 1: /v1/system/health returns CSP header ──
const r = await fetch('http://127.0.0.1:23920/v1/system/health');
const csp = r.headers.get('content-security-policy');
console.log('[csp-e2e] /v1/system/health CSP:', csp);
if (!csp) { console.error('FAIL: no CSP header'); daemon.kill(); process.exit(1); }
if (!csp.includes("default-src") || !csp.includes("'self'") || !csp.includes("script-src")) {
  console.error('FAIL: CSP missing required directives');
  daemon.kill();
  process.exit(1);
}

// ── Test 2: /  also returns CSP ──
const r2 = await fetch('http://127.0.0.1:23920/');
const csp2 = r2.headers.get('content-security-policy');
console.log('[csp-e2e] / CSP:', csp2);
if (!csp2) { console.error('FAIL: no CSP on /'); daemon.kill(); process.exit(1); }

console.log('[csp-e2e] OK: both endpoints have CSP header');
daemon.kill('SIGKILL');
process.exit(0);
