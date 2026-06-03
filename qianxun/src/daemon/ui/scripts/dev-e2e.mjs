// quick e2e: vite dev server + curl main page, check 404 fallback returns 200
// (SPA fallback in serve)
import { spawn } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';

const proc = spawn('E:\\git\\maxu\\qianxun\\qianxun\\src\\daemon\\ui\\node_modules\\.bin\\vite.cmd', [
  'dev',
  '--port', '5174',
  '--host', '127.0.0.1'
], {
  cwd: 'E:\\git\\maxu\\qianxun\\qianxun\\src\\daemon\\ui',
  env: { ...process.env, http_proxy: '', HTTP_PROXY: '' },
  stdio: ['ignore', 'pipe', 'pipe'],
  shell: true
});

let devLog = '';
proc.stdout.on('data', (d) => { devLog += d.toString(); });
proc.stderr.on('data', (d) => { devLog += d.toString(); });

let ready = false;
for (let i = 0; i < 100; i++) {
  await sleep(150);
  try {
    const r = await fetch('http://127.0.0.1:5174/');
    if (r.status < 500) { ready = true; break; }
  } catch { /* not yet */ }
}

if (!ready) {
  console.error('[dev-e2e] vite not ready, log tail:');
  console.error(devLog.split('\n').slice(-15).join('\n'));
  proc.kill('SIGKILL');
  process.exit(1);
}

console.log('[dev-e2e] vite dev ready');
const r = await fetch('http://127.0.0.1:5174/');
const body = await r.text();
console.log(`[dev-e2e] GET / status=${r.status}, body length=${body.length}`);

// check 404 fallback → should still 200 (SPA)
const r2 = await fetch('http://127.0.0.1:5174/some/random/path');
console.log(`[dev-e2e] GET /some/random/path status=${r2.status}, body length=${(await r2.text()).length}`);

// check CSS
const r3 = await fetch('http://127.0.0.1:5174/@id/__x00__virtual:%5C%2F.svelte-kit%2Ftypes%2Froutes%2F%2Blayout.svelte');
console.log(`[dev-e2e] layout.svelte route types status=${r3.status}`);

proc.kill('SIGKILL');
process.exit(0);
