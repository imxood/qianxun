// 千寻壳 E2E 验证用 mock 网关（零依赖 node）：
//   node mock-gateway.mjs
// - 监听 127.0.0.1:17400，配合 `adb reverse tcp:17400 tcp:17400`，
//   手机上的 127.0.0.1:17400 即打到本进程——无需 EasyTier 即可全流程验证壳。
// - 提供 /qx-gate 配对（cookie）、/qx-mobile/info|version（模拟千寻网关契约）、
//   模拟工作台首页，以及 /pair-qr 全屏配对二维码（供手机相机实扫）。
import http from 'node:http';
import { createRequire } from 'node:module';

const require = createRequire('E:/develop/dsh-workspace/qianxun/package.json');
const QRCode = require('qrcode');

const PORT = 17400;
const TOKEN = 'cafebabe12345678cafebabe12345678';
const PAIR_URL = `http://127.0.0.1:${PORT}/qx-gate?token=${TOKEN}`;

const qrcodePng = await QRCode.toBuffer(PAIR_URL, { width: 720, margin: 2 });

function authorized(query, cookieHeader) {
  const cookieToken = (cookieHeader || '')
    .split(';')
    .map(part => part.trim())
    .find(part => part.startsWith('qx_token='))
    ?.slice('qx_token='.length);
  const queryToken = new URLSearchParams(query).get('token');
  return (cookieToken || queryToken) === TOKEN;
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://127.0.0.1:${PORT}`);
  const { pathname, search } = url;
  console.log('[mock]', req.method, pathname + search);

  if (pathname === '/pair-qr') {
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
    res.end(
      `<!doctype html><html><body style="margin:0;background:#fff;display:grid;place-items:center;height:100vh"><img src="/pair-qr.png" style="width:min(90vmin,720px)"></body></html>`,
    );
    return;
  }
  if (pathname === '/pair-qr.png') {
    res.writeHead(200, { 'content-type': 'image/png' });
    res.end(qrcodePng);
    return;
  }

  if (pathname === '/qx-gate') {
    if (url.searchParams.get('token') === TOKEN) {
      res.writeHead(302, {
        location: '/',
        'set-cookie': `qx_token=${TOKEN}; Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax`,
      });
      res.end();
    } else {
      res.writeHead(401, { 'content-type': 'text/plain; charset=utf-8' });
      res.end('无效或已吊销的配对 token');
    }
    return;
  }

  if (!authorized(search, req.headers.cookie)) {
    res.writeHead(401, { 'content-type': 'text/plain; charset=utf-8' });
    res.end('未配对设备');
    return;
  }

  if (pathname === '/qx-mobile/info') {
    res.writeHead(200, {
      'content-type': 'application/json; charset=utf-8',
      'access-control-allow-origin': '*',
      'cache-control': 'no-store',
    });
    res.end(
      JSON.stringify({
        app: 'qianxun',
        version: '0.1.0-test',
        hostname: 'QX-TEST-PC',
        dshReady: true,
      }),
    );
    return;
  }
  if (pathname === '/qx-mobile/version') {
    res.writeHead(200, {
      'content-type': 'application/json; charset=utf-8',
      'cache-control': 'no-store',
    });
    res.end(JSON.stringify({ v: 1, css: '0'.repeat(16), js: '1'.repeat(16) }));
    return;
  }
  if (
    pathname === '/qx-mobile/custom.css' ||
    pathname === '/qx-mobile/custom.js'
  ) {
    res.writeHead(200, { 'content-type': 'text/plain; charset=utf-8' });
    res.end('/* mock */\n');
    return;
  }

  // 模拟工作台首页：验证壳「点卡片 → 网关页加载 → 返回壳」全链路。
  if (pathname === '/') {
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
    res.end(`<!doctype html><html lang="zh-CN"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>工作台</title></head>
<body style="font-family:system-ui;background:#17171a;color:#ececf1;display:grid;place-items:center;height:100vh;margin:0">
<div style="text-align:center"><h1 style="font-size:28px">✅ 已连上 QX-TEST-PC</h1>
<p style="color:#8a8a92">mock 网关 · ${new Date().toLocaleTimeString()}</p></div></body></html>`);
    return;
  }

  res.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
  res.end('not found');
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`[mock] listening http://127.0.0.1:${PORT}`);
  console.log(`[mock] 配对链接: ${PAIR_URL}`);
  console.log(`[mock] 全屏二维码: http://127.0.0.1:${PORT}/pair-qr`);
});
