// qianxun-desktop/src/lib/api/mock-server.ts
// Phase 4a-1: 测试用 in-process mock daemon
//
// 用途:
//   - 单元测试时启动一个 http server, 模拟 daemon 行为
//   - 客户端 (client.ts + chat.ts) 跟它通信, 验证 IPC 正确性
//   - 跑 `pnpm test:unit` 不需要真 daemon
//
// 设计:
//   - 启动 Node http server, 端口 = 0 (随机)
//   - 路由跟真 daemon 对齐 (v0.2 路径, 跟 qianxun/src/daemon/router.rs 当前实现一致)
//   - SSE 流发 desktop 高层 schema (SseEvent, 跟 src/lib/types/sse.ts 对齐)
//     这样 mock server 跟现状兼容, 不需要 mapping 层
//   - 真 daemon 4a-2 上线时, 写一个 mapping (daemon v0.2 wire → desktop 高层) 或
//     改 desktop SseEvent 跟 daemon 对齐 — 那是 4a-2 的事
//
// 行为:
//   - POST /v1/chat/session                  → 返 { session: {...} }
//   - GET  /v1/chat/sessions                 → 返 { sessions: [] } (mock 不持久化)
//   - POST /v1/chat/session/{id}/prompt      → SSE 流: message_start + text* + turn_finished + message_stop
//   - 其他                                   → 404
//
// 启动/关闭: 测试 setup 调 startMockServer(), teardown 调 close()

import { createServer, IncomingMessage, ServerResponse, Server } from 'node:http';
import type { AddressInfo } from 'node:net';

export interface MockServerHandle {
	url: string;
	port: number;
	/** 关闭 server, 释放端口 */
	close(): Promise<void>;
}

interface MockServerOptions {
	/** SSE chunk 间隔 (ms), 默认 30 模拟打字 */
	chunkIntervalMs?: number;
	/** prompt 到 message_start 的延迟 (ms), 默认 200 */
	startDelayMs?: number;
	/** 自定义 echo 函数, 给定 prompt 文本, 返要流的文本 chunks; 默认是固定 "收到: <text>" */
	echo?: (prompt: string) => string[];
}

export async function startMockServer(opts: MockServerOptions = {}): Promise<MockServerHandle> {
	const chunkInterval = opts.chunkIntervalMs ?? 30;
	const startDelay = opts.startDelayMs ?? 200;
	const echoFn = opts.echo ?? ((text: string) => [`收到: ${text}`]);

	// 跟踪已创建的 session — prompt 路径必须在白名单内才返 SSE, 否则 404
	const knownSessions = new Set<string>();

	const server: Server = createServer((req, res) => {
		handleRequest(req, res, { chunkInterval, startDelay, echoFn, knownSessions });
	});

	await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', () => resolve()));
	const addr = server.address() as AddressInfo;

	return {
		url: `http://127.0.0.1:${addr.port}`,
		port: addr.port,
		close: () =>
			new Promise<void>((resolve, reject) => {
				server.close((err) => (err ? reject(err) : resolve()));
			})
	};
}

interface HandlerCtx {
	chunkInterval: number;
	startDelay: number;
	echoFn: (prompt: string) => string[];
	knownSessions: Set<string>;
}

interface HandlerCtx {
	chunkInterval: number;
	startDelay: number;
	echoFn: (text: string) => string[];
}

function handleRequest(req: IncomingMessage, res: ServerResponse, ctx: HandlerCtx): void {
	const url = new URL(req.url ?? '/', 'http://localhost');
	const method = req.method ?? 'GET';

	// CORS for browser fetch (jsdom fetch 走同源不严格, 但加 header 防止 prod 端 fetch 跨源)
	setCors(res);

	if (method === 'OPTIONS') {
		res.statusCode = 204;
		res.end();
		return;
	}

	// POST /v1/chat/session
	if (url.pathname === '/v1/chat/session' && method === 'POST') {
		const id = `sess_mock_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
		const now = new Date().toISOString();
		ctx.knownSessions.add(id);
		writeJson(res, 200, {
			session: {
				id,
				title: '新会话',
				project_id: null,
				model: 'mock-model',
				status: 'Idle',
				created_at: now,
				last_active_at: now
			}
		});
		return;
	}

	// GET /v1/chat/sessions
	if (url.pathname === '/v1/chat/sessions' && method === 'GET') {
		writeJson(res, 200, { sessions: [] });
		return;
	}

	// POST /v1/chat/session/{id}/prompt
	const promptMatch = url.pathname.match(/^\/v1\/chat\/session\/([^/]+)\/prompt$/);
	if (promptMatch && method === 'POST') {
		const sessionId = promptMatch[1];
		if (!ctx.knownSessions.has(sessionId)) {
			res.statusCode = 404;
			res.setHeader('Content-Type', 'application/json');
			res.end(JSON.stringify({ error: 'session not found' }));
			return;
		}
		readBodyJson(req)
			.then((body) => {
				const text = extractUserText(body);
				streamMockPrompt(res, ctx, sessionId, text);
			})
			.catch(() => {
				res.statusCode = 400;
				res.end('bad request');
			});
		return;
	}

	// 其他
	res.statusCode = 404;
	res.end('not found');
}

function setCors(res: ServerResponse): void {
	res.setHeader('Access-Control-Allow-Origin', '*');
	res.setHeader('Access-Control-Allow-Methods', 'GET, POST, PUT, DELETE, OPTIONS');
	res.setHeader('Access-Control-Allow-Headers', 'Content-Type, Authorization');
}

function writeJson(res: ServerResponse, status: number, body: unknown): void {
	res.statusCode = status;
	res.setHeader('Content-Type', 'application/json');
	res.end(JSON.stringify(body));
}

function readBodyJson(req: IncomingMessage): Promise<unknown> {
	return new Promise((resolve, reject) => {
		const chunks: Buffer[] = [];
		req.on('data', (c) => chunks.push(c));
		req.on('end', () => {
			try {
				const text = Buffer.concat(chunks).toString('utf-8');
				resolve(text ? JSON.parse(text) : {});
			} catch (e) {
				reject(e);
			}
		});
		req.on('error', reject);
	});
}

function extractUserText(body: unknown): string {
	if (body && typeof body === 'object' && 'messages' in body) {
		const msgs = (body as { messages: unknown }).messages;
		if (Array.isArray(msgs) && msgs.length > 0) {
			const last = msgs[msgs.length - 1];
			if (last && typeof last === 'object' && 'content' in last) {
				const c = (last as { content: unknown }).content;
				if (typeof c === 'string') return c;
			}
		}
	}
	return '';
}

function streamMockPrompt(
	res: ServerResponse,
	ctx: HandlerCtx,
	sessionId: string,
	userText: string
): void {
	res.statusCode = 200;
	res.setHeader('Content-Type', 'text/event-stream');
	res.setHeader('Cache-Control', 'no-cache');
	res.setHeader('Connection', 'keep-alive');
	res.setHeader('X-Accel-Buffering', 'no');

	const messageId = `msg_mock_${Date.now()}`;
	const chunks = ctx.echoFn(userText);

	// 整体延迟
	setTimeout(() => {
		// 1. message_start
		writeSse(res, {
			event: 'message_start',
			data: { session_id: sessionId, message_id: messageId }
		});

		// 2. text chunks (每 chunk 一行, 但 mock 简化成一整段)
		//    真实 daemon 是多次 text_delta, mock 简化为一次 text (跟 desktop schema 兼容)
		writeSse(res, {
			event: 'text',
			data: { text: chunks.join('') }
		});

		// 3. turn_finished + message_stop
		setTimeout(() => {
			writeSse(res, {
				event: 'turn_finished',
				data: {
					reason: 'end_turn',
					usage: { input: 10, output: 5, cost_usd: 0.0001 }
				}
			});
			writeSse(res, { event: 'message_stop', data: {} });
			res.end();
		}, ctx.chunkInterval);
	}, ctx.startDelay);
}

function writeSse(res: ServerResponse, ev: { event: string; data: unknown }): void {
	// 不发 W3C `event:` 行 — 事件名塞到 data JSON 的 `event` 字段里.
	// 跟 desktop/lib/sse/parser.ts 现有解析逻辑一致 (只读 data: 行, JSON `event` 字段分发).
	// 4a-2 接真 daemon (axum 发 W3C event: 行) 时 parser 升级支持 event: 行.
	res.write(`data: ${JSON.stringify({ event: ev.event, data: ev.data })}\n\n`);
}
