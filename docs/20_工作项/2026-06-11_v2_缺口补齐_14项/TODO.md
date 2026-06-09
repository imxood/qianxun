# TODO.md

> 状态: 启动中 | 最后更新: 2026-06-11

## 排期 (按 ROI + 依赖)

每个子任务独立可执行, 完成后:
- 跑 `cargo test` 248+ 不退步
- 同步更新 [10_事实源/runtime-state.md](../../10_事实源/runtime-state.md) 公开 API 表
- 同步更新 [10_事实源/desktop-state.md](../../10_事实源/desktop-state.md) Tauri command 表
- 在本目录 [验收.md](./验收.md) 追加一行 ✅

---

## Phase 1: P0 缺口 (5 项, ~830 行, v2 必备)

### [ ] 缺口 01 — Hook 退出码 + 熔断

**借鉴**: [octos](E:\git\ai\octos) 退出码语义 + 3 连败熔断
**详细**: [设计/01_Hook退出码与熔断.md](../../设计/01_Hook退出码与熔断.md)
**行数**: ~105
**依赖**: v2 E1 (HookRegistry) 完成
**步骤**:
1. `qianxun-core/src/hooks/handler.rs` HookResult 加 `Error(String)` 变体
2. `qianxun-core/src/hooks/registry.rs` 加 HookStats (DashMap) + 熔断逻辑
3. `qianxun-runtime/src/sse.rs` 加 HookDisabled/HookRecovered 2 变体
4. 测试: 连续 3 次 Error → 第 4 次 skip; 60s 后自动 re-enable
**完成标准**:
- [ ] `cargo test -p qianxun-core -- hooks` 全过
- [ ] `cargo test -p qianxun-runtime -- sse` 全过
- [ ] 248 基线不退

### [ ] 缺口 02 — LLM 错误分类

**借鉴**: [hermes-agent](E:\git\ai\hermes-agent) `error_classifier.py` 22 种 FailoverReason
**详细**: [设计/02_LLM错误分类与恢复.md](../../设计/02_LLM错误分类与恢复.md)
**行数**: ~320
**依赖**: 无
**步骤**:
1. `qianxun-core/src/provider/error.rs` (新) LlmErrorKind 15 变体 + Classifier
2. `qianxun-runtime/src/provider/recovery.rs` (新) decide_recovery
3. `qianxun-runtime/src/api/send.rs` 接入 retry 循环
4. 测试: 22 种分类各 1 case, 决策树 4 种 action 各 1 case
**完成标准**:
- [ ] `cargo test -p qianxun-core -- provider::error` 全过
- [ ] 401/429/500/503/timeout/context_overflow 各 1 集成测试

### [ ] 缺口 03 — SubAgent 工具白名单

**借鉴**: [microclaw](E:\git\ai\microclaw) 9-tool 白名单
**详细**: [设计/03_SubAgent工具白名单.md](../../设计/03_SubAgent工具白名单.md)
**行数**: ~70
**依赖**: v2 E4 (SubAgentSpec) 完成
**步骤**:
1. `qianxun-core/src/subagent/mod.rs` `tool_filter: Option<Vec<String>>` + `DEFAULT_SUBAGENT_TOOLS` 常量
2. `qianxun-core/src/processing_loop/v2.rs` 加白名单校验
3. `qianxun-runtime/src/sse.rs` 加 ToolDenied 变体
4. 测试: 12-tool 拒绝 + 9-tool 允许
**完成标准**:
- [ ] `cargo test -p qianxun-core -- subagent` 全过
- [ ] sub-agent 调 write_file → 拒绝 + 收到 ToolDenied

### [ ] 缺口 04 — Skill 生命周期自动化 (后置, 不阻塞 P0)

**借鉴**: [opencrust](E:\git\ai\opencrust) self-learning skill lifecycle
**详细**: [设计/04_Skill生命周期自动化.md](../../设计/04_Skill生命周期自动化.md)
**行数**: ~330
**依赖**: v2 E6 (Checkpoint 持久化) 完成
**优先级**: P0 但**可独立后置**, 跟其他 P0 解耦
**步骤**:
1. `qianxun-core/src/skills/lifecycle.rs` (新) SkillLifecycle + 4 状态
2. `qianxun-runtime/src/persistence.rs` 加 2 张表 (skill_lifecycle / skill_changelog)
3. skill 格式升级 (向前兼容)
4. 测试: 5+ 次 → Candidate / 30 天 → Archived / confidence gate
**完成标准**:
- [ ] `cargo test -p qianxun-core -- skills::lifecycle` 全过
- [ ] tick() 不阻塞 boot (async 后台)

### [ ] 缺口 05 — 后台异步任务

**借鉴**: [oh-my-opencode](E:\git\ai\oh-my-opencode) `background-task` + `background-continuation`
**详细**: [设计/05_后台异步任务.md](../../设计/05_后台异步任务.md)
**行数**: ~690 (含 Tauri 4 command + Svelte UI)
**依赖**: v2 E4 (SubAgent) 完成
**步骤**:
1. `qianxun-runtime/src/background_task.rs` (新) BackgroundTaskManager
2. `qianxun-runtime/src/api/trait_def.rs` 加 4 方法
3. `qianxun-runtime/src/sse.rs` 加 4 变体
4. `qianxun-desktop/src-tauri/src/commands/runtime/tasks.rs` (新) 4 command
5. `qianxun-desktop/src/lib/stores/task.svelte.ts` (新) store
6. `qianxun-desktop/src/lib/components/col3/TaskListPanel.svelte` (新) UI
7. 测试: 5 并发 + FIFO + cancel + resume
**完成标准**:
- [ ] `cargo test -p qianxun-runtime -- background_task` 全过
- [ ] 用户发 6 个长任务 → 5 Running + 1 Queued
- [ ] Tauri UI 切走 5 分钟, 任务进度正常

---

## Phase 2: P1 缺口 (9 项, ~1330 行, 按 ROI 选做)

### [ ] 缺口 06 — 压缩前 Memory Flush (低垂果实, 优先)

**借鉴**: [openclaw-mini](E:\git\ai\openclaw-mini) `softThreshold` 提前 flush
**详细**: [设计/06_压缩前MemoryFlush.md](../../设计/06_压缩前MemoryFlush.md)
**行数**: ~160
**依赖**: v2 E2 (processing_loop_v2) 完成
**步骤**:
1. `qianxun-core/src/agent/context/compact.rs` 加 `flush_durable_to_memory`
2. `qianxun-core/src/config.rs` 加 3 配置字段
3. `qianxun-memory` (已存在) save 新接口
4. 测试: soft/hard 阈值触发

### [ ] 缺口 07 — 双层循环 + 20 EventStream (重构, 后置)

**借鉴**: [openclaw-mini](E:\git\ai\openclaw-mini) outer/inner loop
**详细**: [设计/07_双层循环与EventStream.md](../../设计/07_双层循环与EventStream.md)
**行数**: ~365
**依赖**: v2 E2 完成
**步骤**:
1. `qianxun-core/src/processing_loop/dual.rs` (新) DualLoop
2. `qianxun-core/src/processing_loop/event.rs` (新) 20 AgentEvent
3. 接入 v2 loop
4. 测试: steer 立即应用 + user input 等 inner 完成

### [ ] 缺口 08 — Provider 三层 Failover (投资大, 最后)

**借鉴**: [octos](E:\git\ai\octos) Retry/Chain/AdaptiveRouter
**详细**: [设计/08_Provider三层Failover.md](../../设计/08_Provider三层Failover.md)
**行数**: ~510
**依赖**: 缺口 02 (LLM 错误分类) 完成
**步骤**:
1. `qianxun-core/src/provider/failover.rs` (新) 三层抽象
2. `qianxun-core/src/provider/scoreboard.rs` (新) 评分
3. `qianxun-runtime/src/api/send.rs` 接入 stack
4. 测试: 三层 + 评分 + 熔断

### [ ] 缺口 09 — Hook 五层 Tier (低垂果实, 优先)

**借鉴**: [oh-my-opencode](E:\git\ai\oh-my-opencode) Session/ToolGuard/Transform/Continuation/Skill
**详细**: [设计/09_Hook五层Tier.md](../../设计/09_Hook五层Tier.md)
**行数**: ~150
**依赖**: v2 E1 + 缺口 01 完成
**步骤**:
1. `qianxun-core/src/hooks/tier.rs` (新) HookTier 5 变体
2. `qianxun-core/src/hooks/registry.rs` 改 HashMap<HookTier, Vec<...>>
3. builtin/ 6 文件加 tier 标注
4. 测试: 5 tier 独立调度

### [ ] 缺口 10 — Hashline Edit 防 Stale

**借鉴**: [oh-my-opencode](E:\git\ai\oh-my-opencode) `hashline-edit`
**详细**: [设计/10_HashlineEdit防Stale.md](../../设计/10_HashlineEdit防Stale.md)
**行数**: ~270
**依赖**: 无
**步骤**:
1. `qianxun-core/src/tools/builtin/hashline.rs` (新) read_with_hashline + hashline_edit
2. builtin/read_file.rs + write_file.rs 改造
3. 测试: stale 检测 + 正常 edit

### [ ] 缺口 11 — Verdict 四态 + BDD 验收 (跟反射联动)

**借鉴**: [agent-spec](E:\git\ai\agent-spec) Verdict 4 态
**详细**: [设计/11_Verdict四态与BDD验收.md](../../设计/11_Verdict四态与BDD验收.md)
**行数**: ~345
**依赖**: v2 E3 (reflect 迁移) 完成
**步骤**:
1. `qianxun-core/src/verify/mod.rs` (新) Verdict 4 态 + Verifier
2. `qianxun-core/src/verify/bdd.rs` (新) BddSpec
3. `qianxun-core/src/hooks/builtin/reflect.rs` 接入 verifier
4. 测试: 4 态各 1 case

### [ ] 缺口 12 — Context Window 五层优先

**借鉴**: [moltis](E:\git\ai\moltis) 5 层 precedence
**详细**: [设计/12_ContextWindow五层优先.md](../../设计/12_ContextWindow五层优先.md)
**行数**: ~305
**依赖**: 无
**步骤**:
1. `qianxun-core/src/provider/capabilities.rs` (新) 5 层 chain
2. `qianxun-core/src/config.rs` 加 ModelCapabilitiesConfig
3. `qianxun-core/src/agent/context/compact.rs` 接入
4. 测试: 5 层覆盖 + 启发式

### [ ] 缺口 13 — Knowledge 五状态 + Gate (跟 Memory Flush 联动)

**借鉴**: [mempal](E:\git\ai\mempal) 5 状态 + Gate
**详细**: [设计/13_Knowledge五状态与Gate.md](../../设计/13_Knowledge五状态与Gate.md)
**行数**: ~420
**依赖**: 缺口 06 (Memory Flush) 完成
**步骤**:
1. `qianxun-memory/src/knowledge.rs` (新) 5 状态 + promote/demote
2. `qianxun-memory/src/gate.rs` (新) KnowledgeGate
3. `qianxun-core/src/hooks/builtin/reflect.rs` 联动 promote
4. 测试: 5 状态 + gate 拒绝

### [ ] 缺口 14 — Session Queue 五种模式

**借鉴**: [octos](E:\git\ai\octos) Followup/Collect/Steer/Interrupt/Speculative
**详细**: [设计/14_SessionQueue五种模式.md](../../设计/14_SessionQueue五种模式.md)
**行数**: ~415
**依赖**: v2 E2 完成
**步骤**:
1. `qianxun-runtime/src/queue.rs` (新) SessionQueue
2. `qianxun-core/src/processing_loop/v2.rs` 接入
3. `qianxun-desktop/src/.../QueueModeSelect.svelte` UI
4. 测试: 5 mode 行为

---

## Phase 3: 收尾 (3 项)

### [ ] 文档同步

- [ ] [10_事实源/runtime-state.md](../../10_事实源/runtime-state.md) 公开 API 表更新
- [ ] [10_事实源/desktop-state.md](../../10_事实源/desktop-state.md) Tauri command 表更新
- [ ] [agent_loop_v2.md](../../10_事实源/架构/agent_loop_v2.md) §7 实施阶段加 14 缺口

### [ ] E2E 验收

- [ ] `pnpm tauri dev` 启动, 跑每个缺口的验收 checklist
- [ ] 桌面端 UI 演示: 后台任务 / 计划模式 / Bypass 模式
- [ ] 6 步 E2E 不退步

### [ ] 工作项收尾归并

按 [document_rules.md §9.3](../../README.md):
- 稳定架构/API/配置/功能结论 → `10_事实源/`
- 长期验收证据 → `40_经验/`
- 已废弃 → `90_历史/`
- 无价值临时记录 → 删除

---

## 进度跟踪

| 缺口 | 状态 | 启动日期 | 完成日期 | 实际行数 | 备注 |
|---|---|---|---|---|---|
| 01 Hook 退出码 | ⏳ |  |  |  |  |
| 02 LLM 错误分类 | ⏳ |  |  |  |  |
| 03 SubAgent 白名单 | ⏳ |  |  |  |  |
| 04 Skill 自学习 | ⏳ |  |  |  | 后置 |
| 05 后台异步任务 | ⏳ |  |  |  |  |
| 06 Memory Flush | ⏳ |  |  |  |  |
| 07 双层循环 | ⏳ |  |  |  |  |
| 08 Provider Failover | ⏳ |  |  |  | 投资大 |
| 09 Hook 5 Tier | ⏳ |  |  |  |  |
| 10 Hashline Edit | ⏳ |  |  |  |  |
| 11 Verdict + BDD | ⏳ |  |  |  |  |
| 12 Context Window | ⏳ |  |  |  |  |
| 13 Knowledge Gate | ⏳ |  |  |  |  |
| 14 Queue 5 mode | ⏳ |  |  |  |  |
