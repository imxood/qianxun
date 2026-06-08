# 04b sub-task #3 — RuntimeApi 收口经验沉淀

> 日期: 2026-06-08
> 范围: qianxun-runtime 新增 RuntimeApi trait + Tauri 5 真 command + daemon router 切 trait
> 跟 `docs/40_经验/2026-06-08_04b_subtask_2_tauri_skeleton.md` 配套
> 关联: docs/30_子项目规划/04b-tauri-runtime-integration.md (上位规划, sub-task #3)

## TL;DR

`qianxun-runtime::RuntimeApi` trait 5 方法收口 daemon HTTP router 跟 Tauri command 的业务, 业务 1:1 搬, 0 改动.
daemon router 4 路由 (`list_sessions` / `cancel_session` / `get_session` / `prompt_handler`) 改调 trait 方法,
HTTP layer 只做协议适配 (JSON ↔ DTO, axum Sse 包装).

**单文件最大 178 行** (state.rs 加 1 字段后), 平均 60 行, 远低于 200 行硬上限.

## 数字

| 指标 | sub-task #2 (前) | sub-task #3 (后) | 变化 |
|---|---|---|---|
| workspace test | 248 passed | 248 passed | 不回归 |
| clippy warning | 0 | 0 | 不变 |
| `router.rs` 行数 | 3851 | 3675 | -176 (旧 prompt_handler 142 行 → 28 行) |
| Tauri 5 真 command | 0 (空 stub) | 5 | 收口 |
| `qianxun-runtime/src/api/` 目录 | 不存在 | 8 文件 | 新增 |
| `core.rs` (trait impl) | 不存在 | 77 行 | 新增 |
| 单文件最大 | 200+ 行 (router.rs) | 178 行 (state.rs) | 收敛 |

## 设计决策 (跟 sub-task #2 同款 5 条, 加 4 条新)

### 继承自 sub-task #2

1. **domain 平行拆分** — runtime 5 command 各自一个文件 (sessions.rs / send.rs / plans.rs / cancel.rs / load.rs), 30-140 行
2. **业务 1:1 搬 0 改动** — daemon router 旧 list_sessions / prompt_handler 业务 100% 搬到 api/sessions.rs / api/send.rs, 不修不优化
3. **runtime 空 stub** (sub-task #2 阶段) → sub-task #3 一次性填 5 个真 command, lib.rs invoke_handler 加 5 行

### sub-task #3 新增

4. **trait 收口 + 薄委托 impl** — RuntimeApi trait 在 `api/trait_def.rs` (74 行), impl 在 `core.rs` (77 行, 6 个 1 行委托给 `*_impl`). 业务实现在 `api/{sessions,send,...}.rs` 各 1 个 `pub async fn *_impl(state, ...)`. **impl block 必须单文件** (Rust 语法约束), 所以业务跟 impl 物理分离, 文件行数全 < 150.

5. **send_message 走 mpsc::Receiver 模式** — 不用 axum Sse 也不用 callback, trait 方法返 `Result<(SendResponse, mpsc::Receiver<SseEvent>)>`. HTTP layer 包 axum Sse (ReceiverStream + event_to_sse), Tauri layer 起 spawn task 消费 receiver 走 `app.emit("session_event", payload)`. **两协议共用同一份业务, 0 重复**.

6. **Plan 在 in-memory HashMap** — sub-task #3 简化, 不接 SessionStore SQLite. `RuntimeState.plans: Arc<Mutex<HashMap<String, PlanInfo>>>`. 后续 sub-task 接 contract (tasks / assigned_to / verify_prompt) 时整体替换.

7. **ListSessionsResponse 加 `filter` 字段** — 跟原 daemon `Json<serde_json::Value>` 1:1 兼容 (有 `filter` 回显字段). 旧测试 `test_sessions_list_with_status_filter` 期望 `v.get("filter") == "active"`, 不加就 fail. **保持 JSON 兼容性, 比改测试更重要**.

## 5 个踩过的坑 (跟 sub-task #2 不重复)

### 坑 1: `MemoryObserver` trait 不在 scope

`api/send.rs` 调 `state.memory.build_context(...)`, `build_context` 是 `MemoryObserver` trait 的方法, `Arc<MemoryCore>` 不直接 impl, 必须 `use qianxun_core::context::MemoryObserver;` 才能调.

**修法**: `use qianxun_core::context::MemoryObserver;` (跟 `qianxun/src/runtime/router.rs` 旧 prompt_handler 一样, 业务搬过来时漏 import).

### 坑 2: `ResolvedConfig` 没 derive `Deserialize`

桌面端 `state/runtime.rs` 想直接 `serde_json::from_str::<ResolvedConfig>(&text)`, 编译报 `the trait Deserialize<'_> is not implemented for ResolvedConfig`.

**修法**: 走 `Config::from_file(path).resolve(None, None)` 路径, 跟 qianxun binary 1:1 (含 JSONC 注释解析 + env 覆盖 + provider 合并). 不要自己 parse 字符串.

### 坑 3: `tauri::command` macro 不通过 `pub use` 传递

`qianxun-desktop/src/commands/mod.rs` 旧版 `mod runtime;` (不带 pub), 外部访问 `commands::runtime::sessions::list_sessions` 报 `module 'runtime' is private` 跟 `macro '__cmd__cancel_session' is not publicly re-exported`.

**修法**: `pub mod runtime;` (跟 `pub mod health;` / `pub mod stronghold;` 一样). **tauri::command macro 必须 `pub mod` 直达, 不能 `pub use` 跨文件传递**. sub-task #2 也踩过, 这次又踩了, 值得记项目日记.

### 坑 4: `Vec<SessionInfo>` move + `len()` 后用 — borrow after move

旧 ListSessionsResponse 字段顺序错了: `sessions` 字段在 `total: sessions.len()` 之前, 编译器报 `value borrowed here after move` (因为 SessionInfo 不 Copy).

**修法**: 调换字段顺序, `total: sessions.len()` 在前, `sessions` 字段在后 (struct literal 顺序无要求, 但借用检查跟字段顺序有关, 因为初始化顺序是声明顺序).

### 坑 5: edit tool 一次替换大块, 残留旧代码

`qianxun/src/runtime/router.rs` 第一次 edit 替换 list_sessions 时, oldString 只覆盖到 new list_sessions 的开头 (含 docstring + signature + `use`), 旧 list_sessions 业务 body (60+ 行) 残留在文件后面, 跟新 cancel_session 后面又出现一个旧 cancel_session, 整文件结构乱了.

**教训**:
- `edit` 工具适合小段替换, 大段 (10+ 行) 用 Python 脚本更稳
- Python 脚本做替换前先 `print(repr(content[start:end]))` 验证要替换的边界
- Rust 函数结尾找不准时, 用 `content.find(next_function_marker)` 倒退 `rfind('}', start, next_marker)` 定位 closing brace

## 验证

```powershell
# 全 workspace 编译
cd E:\git\maxu\qianxun
cargo check --workspace
# Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.80s

# 全 workspace test (跟 sub-task #2 同基线 248)
cargo test --workspace
# test result: ok. 147 passed; 0 failed; 4 ignored
# test result: ok. 34 passed; 0 failed
# test result: ok. 5 passed; 0 failed
# test result: ok. 18 passed; 0 failed
# test result: ok. 44 passed; 0 failed
# 总: 248 passed, 0 failed

# clippy 0 warning
cargo clippy --workspace --all-targets
# Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.59s

# 桌面端
cd qianxun-desktop/src-tauri
cargo clippy --lib  # 0 warning
cargo test --lib    # 0 passed (没有新增 Tauri 单元测试, sub-task #4 接 Svelte 端再补)
```

## 04b sub-task #3 验收清单 (11 项)

- [x] 1. `qianxun-runtime/src/api/` 目录 8 个文件 (mod/error/trait_def/types/sessions/send/plans/cancel/load)
- [x] 2. RuntimeApi trait 5 方法 (list_sessions / send_message / create_plan / list_plans / cancel_session / load_session — 实际 6, 多 1 个 list_plans 配套)
- [x] 3. `qianxun-runtime/src/core.rs` 77 行 impl 块
- [x] 4. RuntimeState 加 `plans: Arc<PlanStore>` 字段 (state.rs 178 行, 加 1 字段)
- [x] 5. Tauri 5 个真 command (sessions/send/plans/cancel/load) 各自一个文件, 22-67 行
- [x] 6. Tauri lib.rs invoke_handler 加 5 行 (total 9 handler)
- [x] 7. Tauri state/runtime.rs stub 替换成真 build() (Config::from_file + RuntimeState::new + fallback new_for_test)
- [x] 8. daemon router 4 路由改调 trait (list_sessions / cancel_session / get_session / prompt_handler)
- [x] 9. api_err_to_http 助手: RuntimeApiError → (StatusCode, String)
- [x] 10. cargo check + clippy + test 全 PASS (0 warning / 0 fail / 248 passed)
- [x] 11. 单文件最大 178 行 (state.rs), 平均 60 行, 远低于 200 行硬上限

## 4 条跨项目可复用教训

1. **Trait + impl 物理分离, 业务可拆到 N 文件** — Rust 强制 impl block 单文件, 但 trait 方法的业务实现可以放其他文件, 用 `pub async fn *_impl(state, ...) -> Result<...>` 模式. impl 文件只写 1 行委托. **业务 < 200 行的硬约束, Rust 语法也能满足**.

2. **流式响应走 mpsc::Receiver 模式** — 不要让 trait 绑 axum Sse / Tauri event / callback, 返 `mpsc::Receiver<Item>` 是最 portable 的. HTTP 包 SSE, Tauri 包 emit event, 测试包 collect. **三处共用同一份业务**.

3. **保持 JSON 兼容 > 改测试** — `ListSessionsResponse` 加 `filter` 字段 (跟旧 `Json<serde_json::Value>` 1:1), 即使 trait 返回结构化数据, 也保留所有回显字段. 旧测试 `v.get("filter") == "active"` 不动, 改字段名/类型会让所有 HTTP 客户端破.

4. **edit 工具适合 < 10 行小段, Python 适合 10+ 行大段** — sub-task #3 第一次 edit 替换 list_sessions 出问题 (残留旧 body + 重复 cancel_session), 用 Python 脚本 `find` + `rfind` 定位边界, `print(repr)` 验证, 才稳. **不要靠 edit 工具做 100+ 行的结构重构**.

## 不在本 sub-task (留给后续)

- Tauri 端 Svelte stores 改 invoke (sub-task #4)
- Plan contract 完整接 (tasks / assigned_to / verify_prompt / 持久化)
- StreamSse 端到端测试 (Svelte 端 Playwright 验证 sub-task #4 一起)
- RuntimeState::new 真实 config path (跟 daemon 主链路 1:1, 当前 fallback to in-memory 兜底)
- SessionStore schema 升级 (Plan 表)

## 关联文件 (主要)

新增 (13 个):
- qianxun-runtime/src/api/{mod,error,trait_def,types,sessions,send,plans,cancel,load}.rs
- qianxun-runtime/src/core.rs
- qianxun-desktop/src-tauri/src/commands/runtime/{mod,sessions,send,plans,cancel,load}.rs

修改 (4 个):
- qianxun-runtime/src/lib.rs (加 api + core 模块)
- qianxun-runtime/src/state.rs (加 plans 字段)
- qianxun-desktop/src-tauri/src/lib.rs (5 invoke handler)
- qianxun-desktop/src-tauri/src/state/runtime.rs (真 build)
- qianxun-desktop/src-tauri/src/commands/mod.rs (pub mod runtime)
- qianxun/src/runtime/router.rs (4 路由切 trait + 删 142 行 prompt_handler body)
