// 简单的 SPA-fallback HTTP server, 用于 dev 截图
// Stage 7a 临时工具 — 走 Python simple server 不支持 fallback, 写一个 Node 的
// 用法: node serve.mjs <port> <root>

import http from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { join, extname } from 'node:path';

const port = parseInt(process.argv[2] ?? '5174', 10);
const root = process.argv[3] ?? '.';

const mime = {
	'.html': 'text/html; charset=utf-8',
	'.js': 'text/javascript; charset=utf-8',
	'.mjs': 'text/javascript; charset=utf-8',
	'.css': 'text/css; charset=utf-8',
	'.json': 'application/json',
	'.svg': 'image/svg+xml',
	'.ico': 'image/x-icon',
	'.png': 'image/png',
	'.jpg': 'image/jpeg'
};

const server = http.createServer(async (req, res) => {
	const url = (req.url ?? '/').split('?')[0];
	let path = join(root, url);

	// 安全: 防止 path traversal
	if (!path.startsWith(root)) {
		res.statusCode = 403;
		res.end('Forbidden');
		return;
	}

	try {
		const s = await stat(path);
		if (s.isDirectory()) {
			path = join(path, 'index.html');
		}
		const buf = await readFile(path);
		res.setHeader('Content-Type', mime[extname(path)] ?? 'application/octet-stream');
		res.end(buf);
	} catch {
		// SPA fallback → index.html
		try {
			const buf = await readFile(join(root, 'index.html'));
			res.setHeader('Content-Type', 'text/html; charset=utf-8');
			res.end(buf);
		} catch {
			res.statusCode = 404;
			res.end('Not Found');
		}
	}
});

server.listen(port, '127.0.0.1', () => {
	console.log(`SPA server: http://127.0.0.1:${port}/ serving from ${root}`);
});
