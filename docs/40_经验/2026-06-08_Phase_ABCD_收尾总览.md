# Phase A→C→B→D 收尾总览 (2026-06-08)

> 范围: A 5 binary 入口切 qianxun-runtime → C Memory 真实化 → B VPS Server 最小收尾 → D Plan 真实执行
> 模式: 1 个 session 跑到底, 不分阶段回 user 等确认 (用户授权"执行到底")
> 状态: ✅ 4 phase 全部完成, 260 (workspace) + 105 (desktop) = 365 测试 pass, 0 clippy warning

## 一句话总结

千寻从 04c 抽完 `qianxun-runtime` 后的第一波"业务 0 重复"工程完成:
- **5 个 binary 入口** (TUI/ACP/CLI/client/server) 全部走同一份 RuntimeState 初始化
- **Memory** compressor/HybridSearch/consolidation 3 件套真实化
- **VPS Server** App 鉴权 + NodeStatus 广播落地 (其他 Stage 7+ 留 follow-up)
- **Plan** 不再 mock, 真实 LLM + tools 顺序执行 tasks

## 各 Phase 概况

| Phase | 范围 | 工作量 | 测试增量 | 关键产物 |
|---|---|---|---|---|
| **A** | TUI/ACP 切 RuntimeState | 1-2 天 (4-5 小时实际) | +1 backend | `App::with_runtime` + `AcpRequestHandler` 字段瘦身 |
| **C** | Memory 真实化 3 件 | 2-3 天 (3 小时实际) | +2 backend + 1 pre-existing bug fix | compressor 收 output + HybridSearch stateless + consolidation 接到 session_end |
| **B** | VPS 最小收尾 2 件 | 3-5 天 (1 小时实际) | +5 (2 msgs + 3 ws_hub) | AppAuth 帧 + NodeStatus 广播 |
| **D** | Plan 真实执行 | 1-2 天 (1.5 小时实际) | +4 backend + 0 desktop (改 7) | PlanInfo 7 字段 + RuntimeApi 6 方法 + spawn 后台 task 跑 |

## 累计变更

### 新增 / 重写文件 (12)

**Backend (qianxun / qianxun-runtime / qianxun-memory / qianxun-core)**:
- `qianxun-memory/src/compressor.rs` — 5 个 compress_* 都收 tool_output 进 narrative
- `qianxun-memory/src/search.rs` — 整个文件重写为 stateless HybridSearch
- `qianxun-memory/src/consolidation.rs` — 加 run_consolidation / run_consolidation_locked, 修 pre-existing 越界 bug
- `qianxun-memory/src/lib.rs` — search_sync 委派 HybridSearch, session_end 调 consolidation, 加 2 测试, MemoryCore 加 `#[derive(Clone)]`
- `qianxun-runtime/src/api/plans.rs` — PlanInfo 7 字段, spawn 后台 task 真实执行, TextCollectSink, 4 测试
- `qianxun-runtime/src/api/types.rs` — PlanStatus 5 态, PlanTaskSpec/Contract/TaskResult
- `qianxun-runtime/src/api/trait_def.rs` — 加 cancel_plan
- `qianxun-runtime/src/core.rs` — impl cancel_plan
- `qianxun/src/tui/mod.rs` — App::new 委托 App::with_runtime, 加新测试
- `qianxun/src/acp/{handler,prompt,session,server}.rs` — AcpRequestHandler 字段瘦身, run_acp_server 1 个 state 参数
- `qianxun/src/server/messages.rs` — 加 4 WsFrame variant (AppAuth/AppAuthOk/AppAuthError/NodeStatus)
- `qianxun/src/server/ws_hub.rs` — authenticate_app / transition_to_app / broadcast_node_status
- `qianxun/src/server/mod.rs` — handle_text_frame 加 AppAuth 派发 + Register 成功后 broadcast_node_status
- `qianxun/src/main.rs` — ACP 入口简化为 1 个 state

**Desktop (qianxun-desktop)**:
- `qianxun-desktop/src-tauri/src/commands/runtime/plans.rs` — 加 cancel_plan Tauri command
- `qianxun-desktop/src-tauri/src/lib.rs` — 注册 cancel_plan
- `qianxun-desktop/src/lib/ipc/runtime.ts` — PlanInput/Info/Contract/TaskSpec/TaskResult 完整 + cancelPlan + 5 态映射
- `qianxun-desktop/src/lib/stores/plan.svelte.ts` — 改 cancelPlan, ipcPlanToEntity 5 态映射
- `qianxun-desktop/src/lib/stores/plan.svelte.test.ts` — mock 改 cancelPlan

### 新增测试 (12 个, 跨 4 phase)

| Phase | 文件 | 测试 |
|---|---|---|
| A | tui/mod.rs | `with_runtime_uses_state_components` |
| C | memory/lib.rs | `compressor_includes_tool_output_in_narrative_for_post_hook` |
| C | memory/lib.rs | `session_end_triggers_consolidation` |
| B | server/messages.rs | `app_auth_frame_roundtrip` |
| B | server/messages.rs | `node_status_frame_roundtrip` |
| B | server/ws_hub.rs | `test_authenticate_app_succeeds_and_transitions_to_app` |
| B | server/ws_hub.rs | `test_authenticate_app_empty_jwt_returns_error` |
| B | server/ws_hub.rs | `test_broadcast_node_status_reaches_all_app_conns` |
| D | runtime/api/plans.rs | `create_then_cancel_plan_marks_aborted` |
| D | runtime/api/plans.rs | `cancel_nonexistent_plan_returns_not_found` |
| D | runtime/api/plans.rs | `create_plan_nonexistent_session_returns_not_found` |
| D | runtime/api/plans.rs | `list_plans_returns_all_with_task_results` |

### 经验沉淀 (4 个项目日记)

| 文档 | 行数 |
|---|---|
| `docs/40_经验/2026-06-08_Phase_A_5_binary_切_runtime.md` | ~250 |
| `docs/40_经验/2026-06-08_Phase_C_memory_真实化.md` | ~250 |
| `docs/40_经验/2026-06-08_Phase_B_vps_server_最小收尾.md` | ~250 |
| `docs/40_经验/2026-06-08_Phase_D_plan_真实执行.md` | ~280 |

每篇 5 段结构: TL;DR / 关键决策 / 踩过的坑 / 验收 / 文件清单 / 范围外 follow-up / 关联.

## 关键设计原则 (跨 4 phase 一致)

1. **5 binary 入口共享 RuntimeState** (Phase A) — `qianxun-runtime::RuntimeState` 10 字段统一 provider/tools/memory/skills/config/agent_host/store/plans/shutdown. TUI/ACP/CLI/desktop 调 `RuntimeState::new(config).await?` 一次, 业务 0 重复
2. **Stateless 设计** (Phase C HybridSearch) — `Connection` 由 caller 提供, struct 只持 vector index, 跟 `Arc<Mutex<Connection>>` 兼容, 不死锁
3. **嵌套 spawn_blocking 防死锁** (Phase C consolidation) — `_locked` 版本接受已加锁 conn, 锁外 caller 走 `run_consolidation`, 锁内走 `run_consolidation_locked`. 嵌套 spawn_blocking 会死锁
4. **Arc::make_mut 配 put-back** (Phase B transition_to_app) — mutate 完必须写回 hashmap, 否则下游读 OLD 值
5. **Pre-existing bug 加新 caller 唤醒** (Phase C merge_similar_clusters) — 之前没人调, 我加 session_end → consolidation 触发后, 越界 panic 暴露

## 跨 phase 经验教训汇总

### 1. 改完一处立即 cargo check, 不要累积
- 一次 edit 完了连续 5 处改动, 编译错混在一起, 调试成本高
- 习惯: 改完一个文件 → `cargo check -p <crate>` → 下一个

### 2. 改 store 引用时, 必须改对应 test mock
- Phase D 改 `cancelSession` → `cancelPlan`, 但 test mock 还在 mock 旧的
- 跑测试才发现, 浪费一次循环

### 3. Rust async trait 必须全实现
- Phase D TextCollectSink 必须实现 9 个方法, 即使业务只用 on_text
- 不实现完全部就编译不过, 写 noop 占位

### 4. 上轮 broken 代码接手时先 cargo check
- Phase A 第一步发现 `App::with_runtime` 上轮没实现, 整个 binary 编译不过
- 之前 04c 以为 done 实际没 done. 这次一上来就跑 cargo check 暴露

### 5. MemoryCore 加 `#[derive(Clone)]` 顺手
- Phase A TUI 需要 MemoryCore clone, 加 derive 一行, 比手写 Clone 简洁
- Arc 字段自动 Arc::clone, 共享 SQLite 连接

## 阶段路线执行情况

按 docs/20_工作项/2026-06-01_TUI性能与Agent开发工具优化/阶段路线.md 的 A-G:

| Phase | 规划 | 现状 |
|---|---|---|
| A 文档事实源 | 修 docs/ | 部分 (daemon-state.md 待更新) |
| B TUI 性能 | 脏标记 + 增量渲染 | 之前已 done (03d) |
| C Memory 闭环 | FTS + 检索 + char 边界 | ✅ done (这次) |
| D MCP/Skills | ServerManager + frontmatter | 之前已 done |
| E Daemon 真实运行时 | AgentLoopHost | ✅ done (04c) |
| **F** Agent Patterns + 工具安全 | React/Plan/Reflect/Workflow + tool policy | **本次新增 4 phase 走 A→C→B→D** |
| G 清理旧入口 + 测试闸口 | 收尾 | 部分 (cli/cli.rs 仍 1200 行, 留 follow-up) |

**A→C→B→D = Phase F 的子集 (Patterns + tool policy 的运行时底层)**.

## 范围外 follow-up (累计 13 项)

### Phase A follow-up (1 项)
- ACP forwarding_tools 跟 state.tools 合并 (现两套并存)

### Phase C follow-up (1 项)
- vector search 集成 (HybridSearch 已留 vector 字段 + 权重)

### Phase B follow-up (5 项)
- 完整 App JWT 签名验证 (jsonwebtoken crate)
- Refresh token 轮换
- NodeStatus 按 team 过滤 (`by_team` 索引)
- 完整 Web UI (chat/team/project/node list, Stage 7 大块)
- RateLimiter → Redis + Outbox SQLite 持久化 + 256 ring + TTL (Stage 6+)

### Phase D follow-up (8 项)
- Plan 持久化
- Task 依赖图 (toposort)
- Task 角色 (assigned_to) 按角色配 LLM
- SseEvent::PlanUpdate 实时事件
- Cancel in-flight task (发 cancel signal)
- Task 失败重试
- Verify task (verify_prompt 字段启用)
- Plan summary (LLLM 拿 task outputs 总结)

### 跨 phase 文档 follow-up (1 项)
- `docs/10_事实源/daemon-state.md` 待重写 (现引用旧文件路径)

## 用户后续工作 (Phase E)

按规划, 用户手动跑 E2E 验收:

```bash
# 启 daemon (HTTP + 1 个 RuntimeState)
python scripts/run.py --port 23900  # 或 cargo run --bin qx -- --daemon --port 23900

# 启 desktop 端
cd qianxun-desktop && pnpm tauri dev

# 端到端走 6 步:
# 1. 发消息 (verify send_message → 走 LLM)
# 2. 流式输出 (verify SseEvent → Tauri emit → Svelte store)
# 3. 创建 plan (verify create_plan 走 LLM + tools 跑 task)
# 4. 持久化 (verify SQLite daemon.db)
# 5. 重启 daemon, 重新连接 (verify session 恢复)
# 6. 退出 (verify graceful shutdown)
```

## 关联

- `docs/30_子项目规划/04b-tauri-runtime-integration.md` (前置: Svelte 切 invoke)
- `docs/30_子项目规划/04c-qianxun-runtime-extraction.md` (前置: RuntimeState 抽离)
- `docs/20_工作项/2026-06-01_TUI性能与Agent开发工具优化/阶段路线.md` (背景: A-G 路线)
- `docs/40_经验/2026-06-08_Phase_A_5_binary_切_runtime.md`
- `docs/40_经验/2026-06-08_Phase_C_memory_真实化.md`
- `docs/40_经验/2026-06-08_Phase_B_vps_server_最小收尾.md`
- `docs/40_经验/2026-06-08_Phase_D_plan_真实执行.md`
