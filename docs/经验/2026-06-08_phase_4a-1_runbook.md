# Phase 4a-1 跑通指南

**时间**: 2026-06-08 凌晨
**范围**: IPC client 骨架 + mock server + 单元测试 (chatStore 切换留 4a-2)
**前置**: 看完 `2026-06-08_desktop_mock_phase.md`

---

## 这一步做了什么

### 新增文件

| 文件 | 用途 |
|---|---|
| `qianxun-desktop/src/lib/api/client.ts` | HTTP client 包装 (fetchWithAuth + ApiError + 401) |
| `qianxun-desktop/src/lib/api/chat.ts` | Chat API (CRUD + fetchPromptStream) |
| `qianxun-desktop/src/lib/api/types.ts` | ChatSession 等 DTO |
| `qianxun-desktop/src/lib/api/mock-server.ts` | in-process mock daemon, 测用 |
| `qianxun-desktop/src/lib/api/__tests__/client.test.ts` | 11 个端到端测试 |
| `qianxun-desktop/vitest.setup.ts` | vitest setup (mock $env/$app 模块) |
| `qianxun-desktop/tests/mocks/app/environment.ts` | $app/environment mock |
| `qianxun-desktop/FEATURE-CHECKLIST.md` 更新 (新加 4a-1 节) |

### 关键决策

#### 1. 复用 daemon/ui 的 IPC client, 不重复造轮子
- `qianxun/src/daemon/ui/src/lib/api/client.ts` 已经有完整 HTTP client
- 复制到 desktop, **去掉 authStore 依赖** (desktop 当前没认证)
- 保留结构 (fetchWithAuth + ApiError + 401), 跟 daemon/ui 一致

#### 2. env 用 `import.meta.env`, 不用 `$env/dynamic/public`
- SvelteKit 推荐的 `$env/dynamic/public` 在 vitest 解析不了 (vitest 跑不到 SvelteKit runtime)
- `import.meta.env.PUBLIC_*` 是 Vite 标准, vitest 原生支持
- 用 **函数** (不是常量) 读 env, 因为 `vi.stubEnv` 后续改 env 不会更新模块级常量
- 命名 `getDaemonUrl()` 每次请求调, 拿到当前 env

#### 3. SSE 协议: 事件名在 JSON `event` 字段, 不用 W3C `event:` 行
- desktop 现有 sse/parser.ts 设计: 忽略 `event:` 行, 从 `data:` JSON 的 `event` 字段分发
- mock server 跟 desktop parser 对齐: `data: {"event": "text", "data": {...}}`
- 4a-2 接真 daemon 时, daemon 实际发 W3C `event:` 行 (axum 默认), parser 升级支持

#### 4. 路径用 v0.2 (`/v1/chat/session/*`), 不用 v1.0 (`/v1/sessions/*`)
- 真 daemon router.rs 还是 v0.2 路径
- v1.0 路径只在 `daemon-design.md` / `_shared-contract.md` 设计文档, 实际没实现
- 4a-2 统一迁到 v1.0 时改

---

## 怎么验证

### 跑测试 (in-test mock server, 不需要真 daemon)

```bash
cd E:\git\maxu\qianxun\qianxun-desktop
pnpm test:unit --run
```

应该看到:

```
✓ src/lib/api/__tests__/client.test.ts (11 tests)
✓ src/lib/sse/parser.test.ts (8 tests)
...
Test Files  8 passed | 1 failed (9)
Tests  48 passed | 2 failed (50)
```

**那 2 个 fail 是 `connection.svelte.test.ts` 的预先存在 bug** (调旧 `sessionStore.clearOfflineQueue` / `reset` 不存在的方法), 跟 4a-1 无关. 不阻塞 4a-1.

### 11 个 client.test.ts 测试覆盖

1. **env resolution**: DAEMON_URL 默认 `http://127.0.0.1:23900`
2. **CRUD**: createChatSession / listChatSessionsAll / 404 抛 ApiError
3. **SSE 流解析**: 4 events 顺序 (message_start → text → turn_finished → message_stop)
4. **message_start payload**: session_id 透传, message_id 自动生成
5. **text echo**: 默认 "收到: <text>"
6. **custom echo**: mock server 可配置 echo 函数
7. **turn_finished payload**: reason + usage
8. **404 unknown session**: 严格白名单检查
9. **sub_session 路径**: 路径拼接正确, mock 返 404 (待 4a-2 真 daemon 实现)

---

## 4a-2 待办 (明天)

### 必须改
1. `chatStore.send` — 调 `fetchPromptStream` 取代 `streamMock`, 累积 SseEvent → Message
2. `chatStore.sendToSubSession` — 同上
3. 删 `lib/utils/stream.ts` (mock 阶段 helper, 不再需要)
4. `planStore.scheduleAutoComplete` 删, 改用真 daemon `plan_update` 事件
5. `subSessionStore.messagesOf` 改 fetch + SSE append
6. session / project / experience / scheduled 持久化数据全部 fetch 真 daemon
7. env flag `PUBLIC_QIANXUN_USE_MOCK` 切真/假
8. `_shared-contract.md` 更新: 统一 v1.0 路径 + SseEvent schema (12 事件 `event` 字段)
9. `sse/parser.ts` 升级: 读 W3C `event:` 行 (跟 axum 一致)
10. mock-server.ts 也升级发 W3C `event:` 行

### 验收
- 启真 daemon (`cargo run` 编译 + 跑 qx daemon)
- `PUBLIC_QIANXUN_DAEMON_URL=http://127.0.0.1:23900 pnpm dev`
- 发消息, 看真 LLM 流式响应
- 跑 FEATURE-CHECKLIST 60 项

---

## 怎么启动 desktop 接真 daemon (4a-2 完成后)

```bash
# 1. 启 daemon
cd E:\git\maxu\qianxun\qianxun
cargo run -- daemon start --port 23900

# 2. 启 desktop, env 指向真 daemon
cd E:\git\maxu\qianxun\qianxun-desktop
PUBLIC_QIANXUN_DAEMON_URL=http://127.0.0.1:23900 pnpm dev

# 3. 浏览器开 http://localhost:5173 (SvelteKit dev 默认端口)
```

降级到 mock (daemon 没起时):
```bash
# 不设 env, 桌面端调 mock fallback (4a-2 实现)
pnpm dev
```

---

## 4a-1 留的坑

1. **SSE 协议不一致**: mock 跟 parser 跟真 daemon 都不同
   - mock: JSON `event` 字段 (跟 parser 对齐)
   - parser: 读 JSON `event` 字段
   - 真 daemon: W3C `event:` 行 + JSON `data` 字段
   - 4a-2 统一

2. **SseEvent schema 跟真 daemon 不一致**: desktop 是高层 (message_start / text / turn_finished), daemon 是低层 (text_delta / content_block_start)
   - 4a-2 在 chatStore 写 mapping (daemon v0.2 wire → desktop Message)

3. **路径 v0.2 vs v1.0**: 跟设计文档不一致
   - 4a-2 统一迁移

4. **没有 auth**: desktop 现在不带 token, daemon 也没强制
   - 后续阶段 (4b / VPS) 加 Bearer token

5. **plan / sub_session / 持久化都还是 mock**: 4a-1 只动了 chat (单 session 流式)
   - 4a-2 切换

---

## 相关文件路径

```
qianxun-desktop/
├── src/lib/api/
│   ├── client.ts                # HTTP client
│   ├── chat.ts                  # Chat API
│   ├── types.ts                 # DTO
│   ├── mock-server.ts           # in-process mock daemon
│   └── __tests__/client.test.ts # 11 个测试
├── vitest.config.ts             # 加 setupFiles + $app/environment alias
├── vitest.setup.ts              # setup hooks
├── tests/mocks/app/environment.ts  # SvelteKit 模块 mock
└── docs/经验/
    ├── 2026-06-08_desktop_mock_phase.md  # 上一阶段日记
    └── 2026-06-08_phase_4a-1_runbook.md  # 本文档
```
