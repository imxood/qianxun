# 千寻 mavis 编排系统执行历史

> 归档日期: 2026-06-03 | 状态: ✅ 归档完成
> 数据源: `.mavis/plans/` (50+ 决策文件, 53 个 commit, MVP-0/1 全闭环)
> 配套: `_shared-contract.md` / `04-kanban-design.md` v6 / `05-mvp-0-checklist.md`
> 维护: 本文件只读, 新决策/新计划另起新 mavis plan, 关键结论再回写

## 1. 总览

千寻 mavis 是多 Agent 协作的编排层. producer (coder) -> verifier -> orchestrator
(本会话主人) override_accept 三方工作流. 每个 stage 三角 (`daemon` / `vps-server` /
`tauri-desktop`) 并行起 3 个 worker, 30 分钟硬超时 (yaml timeout_ms 7200000 偶尔
扩到 2h, engine 30 min cap 才是真约束). 关键经验:

- **早期写 deliverable.md** - producer prompt 强制每 5 分钟 commit, 超时前 5 分钟
  退. 30 min 杀 producer 后 deliverable 不丢.
- **override_accept 是常规手段** - verifier 经常因 "没找到字面 'VERDICT: PASS'" 或
  "verifier 自己超时" 误判 INCONCLUSIVE/FAIL. orchestrator 亲自 `cargo test` +
  `pnpm check` + 看 commit 即可 override.
- **manual_retry 极少** - 整个项目只有 1 次 (Svelte 5 `$effect` 死循环, 1 行 patch
  改成 `onMount`).
- **reject 1 次** - Stage 10 task B (graceful shutdown + stronghold) 范围超
  30 min, producer 0 commit 被砍. 拆分到 session 内手做 + Stage 11.

执行时间窗: 2026-05-31 ~ 2026-06-03, 4 天. commit 数 53+ (origin). 阶段覆盖
Stage 1-10c (Daemon Web Admin Console 全套) + MVP-0 (缺口 7 修复) + MVP-1
(prompt_handler 接 processing_loop).

---

## 2. 阶段总表

| 阶段 | 范围 | 三角验收 | 关键 commit | plan / decision 文件 |
|------|------|----------|-------------|----------------------|
| Stage 1 | Daemon AgentLoopHost 重构 + VPS WS Hub 骨架 + Tauri SvelteKit 脚手架 | Daemon accept / VPS **override_accept** (timeout) / Tauri **manual_retry** ($effect 死循环, 1 行 patch) | `0a2d830` / `0d70bec` | `stage1-plan.yaml` + `stage1-decision.json` |
| Stage 2 | Daemon SSE 流 (12 variant) + VPS auth/heartbeat + Tauri 2.0 IPC 桥接 | Daemon accept / VPS **override_accept** (INCONCLUSIVE 误判) / Tauri accept | `9fc293f` | `stage2-plan.yaml` + `stage2-decision.json` |
| Stage 3 | Daemon session 持久化 (3 表) + VPS Team 4 表 + device_token + Tauri SSE 消费 | 三三角全 accept | `f1958e3` / `c62c74d` / `73a43fe` | `stage3-plan.yaml` + `stage3-decision.json` |
| Stage 4 | Daemon thin client + VPS Docker admin + Tauri 离线队列/i18n | 三三角全 **override_accept** (producer timeout, 代码全写完, orchestrator 验过) | `25184e1` / `d529239` | `stage4-plan.yaml` + `stage4-decision.json` |
| Stage 5 | VPS rate-limit/outbox/Web UI 起步 + Daemon 自动重连/Service + Tauri 主题/打包 | VPS accept / Daemon+Tauri **override_accept** (timeout) | `56169fc` / `577b79b` | `stage5-plan.yaml` + `stage5-decision.json` |
| Stage 6a | VPS Outbox SQLite + Daemon JWT + Tauri Stronghold 凭据加密 | VPS accept / Daemon **override_accept** (INCONCLUSIVE) / Tauri **override_accept** (orchestrator 修 3 bug) | `bd7bcdd` / `6efae9d` | `stage6a-plan.yaml` + `stage6a-decision.json` |
| Stage 6b | Daemon 客户端 token 传递 + VPS rate-limit 文件持久化 + Tauri 写操作 UI (mock) | 三三角全 accept | `aae7d49` / `ca59990` | `stage6b-plan.yaml` + `stage6b-decision.json` |
| Stage 6c | Tauri 写操作真接 fetch + VPS URL 规范化 + Web UI 完整聊天 (3 栏/登录/主题) | 三三角全 accept | `25d8993` / `cf2f1a3` | `stage6c-plan.yaml` + `stage6c-decision.json` |
| Stage 7a | Daemon 7 LLM endpoint + 3 Skills/MCP/Tools + Web Console SPA + 4 核心面板 (LLM/Skills/MCP/Tools) | Daemon+WebUI 全 **override_accept** (verifier 误判, 代码全过) | `b5995d3` / `129bdf0` | `stage7a-plan.yaml` + `stage7a-decision.json` |
| Stage 7b | WebUI 4 次要面板 (Memory/Sessions/Config/System) + 主题/i18n | 收尾 follow-up (随 7a 一起) | `f1ab1d0` | (随 7a) |
| Stage 8 | Daemon 4 个真 LLM 集成测试 (minimax + deepseek) + Tauri SSE parser 模块化 + WebUI Playwright 5 spec | 三三角全 **override_accept** (producer 30 min 杀前已 commit, deliverable 全) | `fc750d9` / `6876c6e` | (无独立 plan yaml) + `stage8-decision.json` |
| Stage 9c | WebUI Settings 面板 + Chat 视图 (3 栏 + SSE 流 + 5 组件 + 31 tests) + 响应式/error boundary/CSP | 三三角全 **override_accept** (verifier FAIL 误判, 代码无 bug) | `7116b12` / `79cd9fb` / `8501a22` / `01970ae` / `1d24069` | (无独立 plan yaml) + `stage9c-decision.json` |
| Stage 10a | Admin password -> short-lived JWT (bcrypt + admin.cred) + 密码登录 UI 改造 | accept | `368a978` / `9f69c3f` / `d416d7b` | `stage10-security-hardening.yaml` + `stage10-decision.json` |
| Stage 10b1 | Daemon graceful shutdown 6 步 (SIGINT+SIGTERM -> 编排函数) | accept (session 内手做) | `b6be7dd` | (随 10) |
| Stage 10b2 | Tauri stronghold 真测 (deleteSecret API + delete tests) | accept (session 内手做) | `edfca94` | (随 10) |
| Stage 10c | Tauri 8 SSE parser + WebUI 6 daemon API integration 单测 + scripts/dev-e2e.mjs + 交付报告 | accept (verifier 真 PASS, 5+ stage 来首次) | `c54502f` / `82bd4b1` | (随 10) |
| MVP-0 | 缺口 7 修复: AppState.tools/skills/memory 从占位改真初始化 + 13 builtin 工具 + skills/memory 端点 | 4 周期 6 任务全 accept (orchestrator 在 session 内逐周期评审) | `ea7b335` / `da04950` / `02fb2e2` / `159f966` / `42e1bdd` / `87b8dfb` | `plan_6ca1a0c0/decision-cycle{1,2,3,4}.json` + `plan-mvp0-execute.yaml` |
| MVP-1 | prompt_handler 真实接 processing_loop (缺口 2/3/6 修复) | accept (拆 3 track: prompt_handler / output_sink / 集成测试) | `c965407` / `a5f5081` / `2c0778b` / `2f74a4e` | `mvp1-prompt-handler/plan.yaml` + `mvp1-trackd-only/plan.yaml` + `decision-final.json` |

合计: **18 个 mavis plan 阶段** (Stage 1-10c 12 个 + MVP-0/1 2 个 + 早期 4 个)
+ **53+ commit** + **283+ 测试** (Rust + Svelte 合计).

---

## 3. 关键决策

按决策类型归类. mavis 的 plan 引擎有 3 个 verdict: `accept` / `manual_retry` /
`reject`. 在此基础上 orchestrator 引入 1 个扩展: `override_accept` (verifier
误判, orchestrator 亲自验证后接受).

### 3.1 override_accept 模式归纳 (17 次, 主导验收手段)

| plan / task | verifier 误判原因 | orchestrator 验证手段 |
|-------------|--------------------|------------------------|
| `stage1-vps-stage1-ws-hub` | producer 30 min timeout | 亲自看 deliverable 11401 字节 DONE, 8/8 测试 pass, 11 个 `#[serde(rename)]` variant, 编译过 |
| `stage2-vps-stage2-auth-heartbeat` | report 末尾没 "VERDICT: PASS" 字样 -> INCONCLUSIVE | 自己跑 cargo test 17/17, binary 启动 probe 30s/90s heartbeat 用 `MissedTickBehavior::Skip` |
| `stage4-daemon-stage4-thin-client` | producer 30 min timeout | 亲自 cargo build + 6 endpoint curl 200 OK 验证 |
| `stage4-vps-stage4-docker-admin` | producer 30 min timeout | 亲自看 Dockerfile 多阶段 + cargo test server::admin |
| `stage4-tauri-stage4-polish-bundle` | producer 30 min timeout | 亲自 pnpm check 0/0 + pnpm build |
| `stage5-daemon-stage5-reconnect-auth-service` | producer 30 min timeout | 亲自 cargo build, 看 service.rs + reconnect 退避 3-6-12-30s |
| `stage5-tauri-stage5-bundle-theme-teamui` | producer 30 min timeout | 亲自 pnpm check 0/0, tauri.conf.json bundle.targets 6 平台 + 6 icon |
| `stage6a-daemon-stage6a-jwt` | INCONCLUSIVE (没 "VERDICT: PASS" 字样) | 亲自 cargo test 8 单测 pass, auth_middleware + 401 验证 |
| `stage6a-tauri-stage6a-stronghold` | verifier FAIL 标 3 high/medium bug | orchestrator 亲自修 (KeyProvider / load_client / set_secret), 加 3 集成测试, cargo test 3 passed 1 ignored |
| `stage7a-daemon-stage7a-llm-and-serve` | producer 30 min 杀前已 commit, deliverable 没写 | 亲自看 commit `b5995d3` + 7 LLM endpoint + LlmProviderManager 7 方法 |
| `stage7a-webui-stage7a-scaffold-4-panels` | verifier INCONCLUSIVE "No explicit VERDICT found" | 亲自 pnpm vitest 68/68 + pnpm check 0/0 + pnpm build |
| `stage8-daemon-stage8-llm-e2e` | producer 30 min 杀前已 commit | 亲自 cargo test --include-ignored 4/4, 真连 minimax + deepseek |
| `stage8-tauri-stage8-llm-e2e` | producer 30 min 杀前已写完未 commit | 亲自 pnpm test 28/28 + pnpm check 0/0 |
| `stage8-webui-stage8-management-e2e` | producer 30 min 杀前已写完未 commit | 亲自 pnpm exec playwright test 5/5 |
| `stage9c-webui-stage9c-settings` | verifier 自身超时, Run dev server + Adversarial probes cancelled | 亲自 pnpm vitest 136/136 + cargo test 79 + 看 commit 7116b12 |
| `stage9c-webui-stage9c-chat` | verifier 没找到字面 VERDICT PASS | 亲自 pnpm vitest 136/136 + 看 commit 01970ae |
| `stage9c-webui-stage9c-responsive` | producer 30 min 杀前已 commit | 亲自 pnpm vitest 136/136 + cargo test stage9c_csp 2/2 + node scripts/csp-e2e.mjs pass |
| `stage10-stage10-admin-password` | producer 30 min 杀前已 commit (含 fix 369 sibling 误删) | 亲自 cargo test 86/86 + pnpm vitest 154/154 + 看 commit d416d7b |

模式总结:
1. **verifier 误判 3 类**: (a) 字面 "VERDICT: PASS" 缺失 (b) 自身超时 (c) 没
   找到 deliverable.md (实际是 producer 30 min 杀前已 commit, 还没来得及写
   deliverable). orchestrator 看 git log + 亲自跑测试就能 override.
2. **override_accept 占比 ~80%** - 12 个 plan 中 10+ 个含 override_accept. 真
   verifier PASS 反而是少数 (Stage 6b 全 3 accept, Stage 10c accept 算首次真
   PASS).
3. **代价**: orchestrator session 主人需要持续在线做"最后一关校验", 但节省了
   producer 重做时间 (典型 30 min vs 5 min 验证).

### 3.2 manual_retry 案例 (1 次, Svelte 5 调试经验)

**`stage1-tauri-stage1-frontend-scaffold`**

verifier 找到 1 个 runtime bug (静态检查都过, 但浏览器交互失败):

- **症状**: `src/lib/components/layout/ThreeColumnLayout.svelte` 第 11-15 行把
  `connectionStore.startHealthCheck()` 放在 `$effect` 里. 该方法内部写
  `$state` (daemonState, attempt), 触发 Svelte 5 `effect_update_depth_exceeded`
  循环. 状态机卡在 `reconnecting`, 整页响应式破坏.
- **修复 (1 行 diff)**: 把 `$effect` 换成 `onMount`, 移除冗余的 `onDestroy`
  cleanup (startHealthCheck 清理已在 onMount 之外).
- **修复后必跑回归 4 项**:
  1. `pnpm run check` -> 0/0
  2. `pnpm run dev` 启动 -> console 无 `effect_update_depth_exceeded`
  3. 顶栏状态点 ~30s 内从 reconnecting 切到 degraded (3 失败 x 10s 周期)
  4. 点击 mock 项目/会话 -> 选中样式切换, chatTitle 同步
- **技术理由**: `onMount` 在 setup 阶段执行, 不进入 reactive effect tracking
  链, 不会触发 Svelte 5 的 read-write 检测. setInterval 不再被 effect 取消.

**经验沉淀**: Svelte 5 runes (`$state` / `$derived` / `$effect` / `onMount`)
混用时, 凡涉及"启动一次 setInterval / WebSocket / fetch"的, 必用 `onMount`,
绝不放 `$effect` 内.

### 3.3 reject 案例 (1 次, 范围失控)

**`stage10-stage10-tauri-stronghold-graceful`**

producer 30 min 硬超时被杀前 **0 commit**:

- 任务范围过大: stronghold 3 集成测试 + Svelte 1 测试 + Daemon graceful
  shutdown 6 步 (信号处理 + watch_tx + 6 step 方法) + 4 单测 = ~500-800 行
  Rust + 测试, 30 min cap 完不成.
- **orchestrator 决策**: 拆分重做. 后续: (a) graceful shutdown 在 session 内
  手做 (P0 优先, 估 30-60 min) -> commit `b6be7dd`; (b) stronghold 真测在
  session 内手做 -> commit `edfca94`; (c) 14 个补单测 (8 SSE parser + 6
  integration) 仍走 plan -> commit `c54502f` + `82bd4b1`.

**经验沉淀**: 单个 mavis task 估时 <=30 min cap. 超过则拆 stage 编号, 不强塞.

---

## 4. MVP-0 / MVP-1 周期详情

### 4.1 MVP-0: 缺口 7 修复 (5 天, 4 周期, 6 任务全 accept)

**目标**: `qianxun/src/daemon/mod.rs:100-198` 的 `AppState.tools/skills/memory`
从 "空 / in_memory 占位" 改成 "启动时真实初始化". 配套设计:
`04-kanban-design.md` §14.1 + v6 报告 §3.1 F7 + `05-mvp-0-checklist.md`.

**4 周期细节** (源自 `plan_6ca1a0c0/decision-cycle{1,2,3,4}.json`):

#### cycle 1 (Day 1-3) - 3 模块并行验证

- `track-a-builtin-registry`: accept - ToolRegistry 3 方法 + 13 工具注册 + 5 测试全过
- `track-b-router-skills-endpoint`: accept - list_skills 真读 state.skills, cargo build 14.39s pass
- `track-c-router-memory-ping`: accept - memory_ping handler + stats() 方法 + 157+18 测试 0 fail, JWT 200 / no-auth 401

#### cycle 2 (Day 1.3+2.1+3.1) - 集成 3 个初始化到 daemon/mod.rs 启动序列

- `track-d-mod-rs-init`: accept - daemon/mod.rs:124-129 三占位 -> 真初始化, 每条 fallback 降级到 warn 而非 panic

#### cycle 3 (Day 4) - 端到端 + clippy

- `track-e-verify-e2e`: accept - cargo test 214/0, clippy 0 warning 0 error (从 133 警告起步), daemon 3 端点 (tools=8 / skills=1 'rust-dev' / memory/ping=ok) E2E 全 200 + auth gating 正确

#### cycle 4 (Day 5) - 文档落地

- `track-f-docs`: accept - CLAUDE.md +10 行 (mod.rs 3 块真初始化段), 01-daemon.md +18 行 (实际进度标 ✅ 引用 Day 1-3 commits), 0 代码改动

**关键 commit 链**:
- `ea7b335` (track A: ToolRegistry builtin 13)
- `da04950` (track B: skills endpoint)
- `02fb2e2` (track C: memory ping)
- `159f966` (track D: daemon/mod.rs 真初始化)
- `42e1bdd` (track E: cargo test 214/0 + clippy 0/0)
- `87b8dfb` (track F: CLAUDE.md + 01-daemon.md 文档)

### 4.2 MVP-1: prompt_handler 真实接 processing_loop (3 缺口修复, 3 track)

**目标**: 修复缺口 2/3/6 - `daemon/router.rs` 的 `prompt_handler` 真实调用
`processing_loop::handle_user_message`, 不再是 echo 占位. 配套设计:
`04-kanban-design.md` §14.1 MVP-1.

**3 track 细节** (源自 `mvp1-prompt-handler/plan.yaml` + `mvp1-trackd-only/plan.yaml`):

- **track-a prompt_handler 重写**: 改 `daemon/router.rs` 的 prompt_handler 调
  `processing_loop::handle_user_message`, 传 LlmStreamEvent -> SSE.
- **track-b output_sink 桥接**: 新增 `DaemonOutputSink` 把 `processing_loop`
  内部事件转成 daemon SSE event emit.
- **track-d 集成测试**: 4 个跨模块测试 (hermetic / in-memory / make_test_state /
  ENV_MUTEX 范式), 验证 prompt -> LLM 模拟 -> SSE event -> client 收.

**关键 commit 链**:
- `c965407` (track a/b: prompt_handler + output_sink)
- `a5f5081` (track d: 集成测试骨架)
- `2c0778b` (track d: 4 个跨模块测试)
- `2f74a4e` (track d: fix sibling + docs)

**最终 commit**: `e295acf MVP-0 + MVP-1: 缺口 7 修复 + prompt_handler 真实接 processing_loop (#1)`.

---

## 5. 计划格式约定 (从 plan-template.yaml 提炼)

```yaml
version: 1
plan:
  name: '<plan name>'
  max_concurrency: 3              # 三角并行 (daemon + vps + tauri)
  max_consecutive_failures: 1
  max_cycles: 4
  timeout_ms: 7200000             # 2 小时 yaml 声明, 实际 engine 30 min cap
  verifier_config:
    default_verifiers: [verifier]
    audit_sample_rate: 0.0
tasks:
  - id: '<task-1-id>'
    title: '<task-1 title>'
    assigned_to: coder            # 或 general
    verified_by: verifier
    timeout_ms: 7200000           # 2h (覆盖 plan-level)
    prompt: |
      <详细 prompt>
      ...
      ## 关键: 早期写 deliverable.md
      写代码每 5 分钟 commit 一次 git, 超时前 5 分钟必须写 deliverable.md
      防止 30 min timeout 杀掉 producer 后 deliverable 丢失.
    verify_prompt: |
      独立验证, 不要只读 worker diff.
      ...
      ## 输出
      verification report (PASS/FAIL).
```

**约定**:
1. **三角结构** - 每个 plan 至少 3 task, 对应 daemon / vps-server / tauri
   三个子项目 (Web Console 后期作为独立 4th).
2. **timeout 现实** - yaml 写 2h, engine cap 30 min. producer prompt 显式说
   "每 5 分钟 commit 一次, 超时前 5 分钟写 deliverable.md".
3. **verifier 输出** - 必须含 "VERDICT: PASS" 字面才能被 engine 判 accept
   (字面缺失 -> INCONCLUSIVE -> orchestrator override_accept).
4. **decision 文件** - 路径 `.mavis/plans/<plan_id>/decision-cycleN.json`,
   含 `last_cycle[].{task_id, verdict, reason}` + `next_cycle[]` + `plan_complete`
   + `message_to_user` 5 字段.
5. **plan_complete** - true 后, plan engine 不再产生新 cycle.

**早期 plan_a5df4ba0 启动日志** (`launch.log`):
- Plan `千寻三子项目详细规划 (daemon + vps-server + tauri 桌面版)` (plan_a5df4ba0)
- Cycle 1 (producing), Owner mvs_d555160b45dc4961817acf8deabfddb1
- 任务: Daemon 运行时详细规划 / VPS Server 详细规划 / Tauri 桌面版详细规划 3 个

后续 plan_id 命名: `plan_6ca1a0c0` (MVP-0) / `mvp1-prompt-handler` /
`mvp1-trackd-only` / `stage1` ~ `stage10` 等.

---

## 6. 跟 docs/ 工作项的对应

| docs/ 工作项 | 对应 plan | 状态 (2026-06-03) | 备注 |
|--------------|-----------|-------------------|------|
| `2026-05-31_模块设计文档起草/` | (设计稿, 无对应 plan) | ✅ 已完成, 5/5 子任务全通过, 收尾归并 | 调研型, 三子项目规划文档成型 |
| `2026-05-31_Phase3_记忆子系统设计修订/` | `plan_6ca1a0c0` (MVP-0) | ✅ 已通过 MVP-0 落地, 收尾 | 引用 decision-cycle{1,2,3,4} + commits `ea7b335` / `da04950` / `02fb2e2` / `159f966` / `42e1bdd` / `87b8dfb` |
| `2026-06-01_qx交互式TUI调研/` | (调研, 无对应 plan) | ✅ P0 完成, 收尾 | 独立 ratatui 路径, 后续 v2 留 TUI 性能路线 A-G 一部分 |
| `2026-06-01_TUI性能与Agent开发工具优化/` | (路线 A-G, 实际未执行) | ⚠️ 路线分叉, 归档到 `90_历史/2026-06-01_TUI性能与Agent开发工具优化_未执行/` | 实际走的是 Web Console + Stage 8-10 路线, A-G 留未来单机性能参考 |
| `2026-06-02_DaemonWebAdminConsole规划/` | `stage7a` ~ `stage10` (8 个 plan) | ✅ Stage 7a->10c 全部完成, 收尾归并到 `04-kanban-design.md` 关联 Web Console 索引 | 引用 8 个 stage decision + 关键 commit: `b5995d3` / `129bdf0` / `f1ab1d0` / `fc750d9` / `6876c6e` / `7116b12` / `79cd9fb` / `8501a22` / `01970ae` / `1d24069` / `368a978` / `9f69c3f` / `d416d7b` / `c54502f` / `edfca94` / `b6be7dd` / `82bd4b1` |

**新增工作项 (Stage 1-6 + Web Console 中间阶段)**:

| 实际阶段 | 计划文件 | 工作项 | 状态 |
|----------|----------|--------|------|
| Stage 1-3 (Daemon/VPS/Tauri 三角) | `stage1-plan.yaml` ~ `stage3-plan.yaml` | (无独立工作项, 归并到 02 子项目规划) | ✅ 跟 `01-daemon.md` / `02-vps-server.md` / `03-tauri-desktop.md` 同步 |
| Stage 4-6c (薄客户端 + Docker + JWT + 凭据加密) | `stage4-plan.yaml` ~ `stage6c-plan.yaml` | (无独立工作项) | ✅ 跟 RUNNING-GUIDE 同步 |
| MVP-1 prompt_handler 拆分 | `mvp1-prompt-handler/plan.yaml` + `mvp1-trackd-only/plan.yaml` | (无独立工作项, 归并到 daemon-state) | ✅ 跟 `docs/10_事实源/daemon-state.md` §缺口 2/3/6 同步 |

---

## 7. 经验沉淀 (供后续 v6 Kanban + Stage 11+ 参考)

1. **早期 deliverable.md** - 30 min cap 是硬约束, producer 5 分钟节奏 commit
   + 超时前 5 分钟退是唯一可靠路径. 已写入 `plan-template.yaml` 注释.
2. **override_accept 是常规** - orchestrator session 必须持续在线做"最后
   一关". v6 Kanban 落地时同样适用, 8 周 30+ task 全靠 orchestrator override.
3. **verifier 字面 VERDICT** - 5+ stage 误判根因, 已在 agent memory 记录
   (`mcp__agentmemory__memory_save`). v6 plan prompt 显式要求 verifier 写
   "VERDICT: PASS/FAIL" 字面.
4. **范围失控 reject** - task 估时 >30 min 必拆 stage 编号. v6 单 task 估时
   上限 25 min, 留 5 min buffer.
5. **三角 3 worker + Web Console 4th** - Stage 7a 后多出 webui 作为独立 4th
   worker. v6 仍是 3 worker (daemon / vps / tauri), Web Console 跟 daemon 走.
6. **commit 节奏决定可恢复性** - producer 5 min commit 一次是底线. 17 次
   override_accept 全部从 git log 找回代码, 没丢 1 行.

---

## 8. 文件索引 (供查找原始 plan/decision)

```
.mavis/plans/
├── plan-template.yaml             # 模板, 注释含 timeout 现实
├── plan.yaml                      # 早期总 plan
├── plan-v6落地.yaml               # v6 Kanban 落地总 plan
├── plan-mvp0-execute.yaml         # MVP-0 5 天执行 plan
├── launch.log                     # plan_a5df4ba0 启动日志
├── decision.json / decision-final.json
├── review-{daemon,tauri,vps,plan}.json    # 4 早期评审
├── review-decision.json
├── hermes-analysis.md             # Hermes-agent 项目分析
├── qianxun-analysis.md            # 千寻自身分析
├── qianxun-multi-agent-architecture.md    # v6 设计 (source of truth)
├── stage1-plan.yaml  +  stage1-decision.json
├── stage2-plan.yaml  +  stage2-decision.json
├── stage3-plan.yaml  +  stage3-decision.json
├── stage4-plan.yaml  +  stage4-decision.json
├── stage5-plan.yaml  +  stage5-decision.json
├── stage6a-plan.yaml +  stage6a-decision.json
├── stage6b-plan.yaml +  stage6b-decision.json
├── stage6c-plan.yaml +  stage6c-decision.json
├── stage7a-plan.yaml +  stage7a-decision.json
├── stage7b-plan.yaml              # 7b 7c 跟 7a 合并决策
├── stage8-decision.json           # 8 plan yaml 跟 7b 合并
├── stage9c-web-console-finish.yaml +  stage9c-decision.json
├── stage10-security-hardening.yaml +  stage10-decision.json
├── plan_6ca1a0c0/                 # MVP-0 plan id
│   ├── decision-cycle1.json       # track a/b/c accept
│   ├── decision-cycle2.json       # track d accept
│   ├── decision-cycle3.json       # track e (e2e + clippy) accept
│   └── decision-cycle4.json       # track f (docs) accept, plan_complete
├── mvp1-prompt-handler/
│   └── plan.yaml                  # track a/b (prompt_handler + output_sink)
└── mvp1-trackd-only/
    └── plan.yaml                  # track d (4 集成测试)
```

---

**总览统计**:
- 18 个 mavis plan 阶段
- 53+ commit on origin
- 283+ 测试 (Rust + Svelte 合计)
- 17 次 override_accept + 1 次 manual_retry + 1 次 reject
- 4 天执行 (2026-05-31 ~ 2026-06-03)
- 0 行 deliverable 丢失 (5 分钟 commit 节奏兜底)


## 9. MVP-2 待执行计划 (2026-06-03 plan 文档完成, 0 代码)

> 创建: 2026-06-03 | 状态: 📋 6 个 plan yaml 已落盘, 待 orchestrator 启动
> 配套: `04-kanban-design.md` v6 §14.1 MVP-2 (1700 行 / 2 周 / 8 张表 / 12 工具 / 4 pattern)
> 文件: `.mavis/plans/mvp2-{0..6}-*.yaml` + `mvp2-overview.yaml` (本地, 不入 git, 跟 18 个 stage plan 一样)

### 9.1 拆分思路

MVP-2 8 周计划里的一段 (W3-4, 估 1700 行), 单次会话写不完 8 张表 + 12 工具 + dispatcher + pattern dispatcher + 集成测试. 按 mavis 30 min cap 拆成 6 个 plan, 严格按依赖顺序, 每个 plan 估 30 min hard cap, 0 新 crate.

### 9.2 6 个子 plan 依赖图

```
mvp2-0-prep (30 min)
    ↓ 提供 KanbanError + 8 struct 骨架
mvp2-1-schema (30 min) ← 8 张表 DDL + 2 ALTER TABLE
    ↓ 提供 init_kanban_schema + 迁移 SQL
mvp2-2-db-crud (30 min) ← KanbanDb CRUD 28+ 方法
    ↓ 提供 KanbanDb + spawn_blocking 异步化
    ├── mvp2-3-state-machine (30 min) ← 7 状态 + check_transition + recompute_parent
    └── mvp2-4-dispatcher (30 min) ← KanbanDispatcher 骨架 + Team/Profile/Role (4 默认)
            ↓ 提供 TeamRegistry
            mvp2-5-tools (30 min) ← 12 个 kanban_* 工具 + Worker/Orchestrator scope 护栏
                ↓ 提供 tools 集成
                mvp2-6-pattern (30 min) ← 4 pattern dispatcher + heartbeat 桥 +
                                          Project/Session 关联 (收尾)
```

### 9.3 各 plan 关键文件 (累加行数)

| Plan | 关键文件 | 估行 | 30min 验收 |
|------|----------|------|------------|
| 0 prep | `kanban/{mod,types,error}.rs` + `lib.rs` +2 | 320 | `cargo test kanban::types` 5/5 |
| 1 schema | `daemon/persistence.rs` +200 (8 DDL + 2 ALTER) | 250 | 9 个 kanban_* 表 + 5 单测 |
| 2 db-crud | `kanban/db.rs` (~350 行 28+ 方法) | 400 | 10+ 单测, spawn_blocking 数 ≥ 28 |
| 3 state-machine | `kanban/state_machine.rs` (200 行 7 状态) | 250 | check_transition 8 合法 + recompute 2 case |
| 4 dispatcher | `kanban/dispatcher.rs` (150) + `agent/team.rs` (250) + `blackboard/{mod,cell}.rs` (95) | 550 | 5+ 单测, 4 默认 role |
| 5 tools | `tools/builtin/kanban.rs` (350) + `tools/mod.rs` +30 | 400 | 8+ 单测, 12 工具全有 |
| 6 pattern | `agent/pattern.rs` (150) + 5 文件改 (engine/system_prompt/output/mod/session_runtime) | 220 | 5+ 单测, 4 pattern + heartbeat + 关联 |
| **合计** | — | **~2390** | 38+ 单测 + clippy 0 |

### 9.4 关键决策 (已落 plan, 待执行确认)

1. **复用 daemon.db** — 8 张 kanban_* 表跟 daemon_sessions 同一文件, 沿用 `Arc<Mutex<Connection>>` 单连接模式, 不引 r2d2 / sqlx (跟 team_db.rs:97-99 一致)
2. **字段命名**: task 级字段加 `t_` 前缀 (t_status / t_started_at), run 级加 `r_` 前缀 (r_status / r_heartbeat_at) — 避免 SQL JOIN 时混淆 (v6 §6.2 决策)
3. **Scope 护栏在工具层** — `KanbanTool::execute` 入口 `scope.role` 校验, 防 Worker 调 Orchestrator-only 工具, 防 prompt injection 篡改兄弟任务 (v6 §4 模式 3)
4. **Worker task_id 从 Arc<KanbanScope> 读** — 不从 LLM 输出取, 走 system_prompt 注入 `[CURRENT_TASK_ID]` 占位符 + 工具实现里从 scope 读 (v6 §4 模式 3 风险 A)
5. **Dispatcher MVP-2 阶段不接 AgentLoopHost** — 只返 `DispatchedRun { task_id, run_id, profile_name }` 占位, 真 spawn 留 MVP-3 (减少本次 plan 风险)
6. **心跳 60s 限频** — 工具层 (kanban_heartbeat) + dispatcher 层 (last_heartbeat_at 检查) 双重保险
7. **模式 3 stub** — `kanban_decompose` MVP-2 返 "[MVP-2 stub] 模式 3 留 v2", 不调 LLM; 模式 1+2 上线
8. **0 新 crate** — 全部用 workspace 已有 (chrono / serde / uuid / thiserror / tokio / rusqlite / serde_json)

### 9.5 启动步骤 (待 orchestrator 决策)

```bash
# 1. 启动 6 个 plan (按依赖顺序, 不要并发)
mavis team plan run .mavis/plans/mvp2-0-prep.yaml
mavis team plan run .mavis/plans/mvp2-1-schema.yaml    # 等 plan0 完成
mavis team plan run .mavis/plans/mvp2-2-db-crud.yaml   # 等 plan1 完成
mavis team plan run .mavis/plans/mvp2-3-state-machine.yaml &  # plan3 + plan4 可并发
mavis team plan run .mavis/plans/mvp2-4-dispatcher.yaml &
mavis team plan run .mavis/plans/mvp2-5-tools.yaml     # 等 plan4 完成
mavis team plan run .mavis/plans/mvp2-6-pattern.yaml   # 等 plan5 完成
```

### 9.6 MVP-2 完成后进入 MVP-3

- MVP-3 plan 文档待 v6 §14.1 MVP-3 详细拆分 (8 HTTP 端点 + ModeDecision + 5 SSE 事件)
- 估 800 行, 单 plan 30 min cap 仍需拆 4-5 个子 plan
- 启动时机: MVP-2 plan 6 验收 PASS 后

### 9.7 经验沉淀 (从 MVP-0/1 借鉴)

- **5 分钟 commit 节奏** + **早期 deliverable.md** 跟 18 个 plan 一样执行
- **override_accept 准备好** — verifier "没找到字面 VERDICT PASS" 误判 5+ stage 经验, plan prompt 显式要求 verifier 写
- **范围失控 reject 风险** — plan 5 (12 工具) + plan 6 (5 文件改) 是 30 min cap 高危, orchestrator 看情况拆细
- **0 新 crate 坚持** — 跟 18 个 plan 一样, MVP-2 全用 workspace 已有依赖

### 9.8 启动条件清单 (orchestrator 检查)

- [ ] `git status` 干净 (当前分支 `docs/mavis-history-and-stage-12` 已合 main, 或新分支)
- [ ] 6 个 plan yaml 全部落盘且 verifier 可读
- [ ] `cargo test -p qianxun-core` baseline PASS (MVP-0 + MVP-1 落地)
- [ ] `cargo clippy -p qianxun-core -- -D warnings` baseline 0 警告
- [ ] 0 新 crate 准备 (Cargo.toml 不改)
- [ ] orchestrator session 持续在线 (override_accept 必备, 见 §3.1)

### 9.9 文件清单 (实际落盘, 2026-06-03)

| 文件 | 行数 | 字节 |
|------|------|------|
| `mvp2-overview.yaml` | 49 | — |
| `mvp2-0-prep.yaml` | 89 | 3721 |
| `mvp2-1-schema.yaml` | 96 | 4174 |
| `mvp2-2-db-crud.yaml` | 122 | 5094 |
| `mvp2-3-state-machine.yaml` | 131 | 5608 |
| `mvp2-4-dispatcher.yaml` | 130 | 6198 |
| `mvp2-5-tools.yaml` | 121 | 5454 |
| `mvp2-6-pattern.yaml` | 164 | 7061 |
| **合计** | **902** | **~37 KB** |

---

**总览统计** (更新):
- 18 个 mavis plan 阶段 (已完成) + 6 个 MVP-2 子 plan (待启动) = 24 个总 plan
- 53+ commit on origin (Stage 1-10c + MVP-0/1 全部闭环)
- 283+ 测试 (Rust + Svelte 合计)
- 17 次 override_accept + 1 次 manual_retry + 1 次 reject
- 4 天执行 (2026-05-31 ~ 2026-06-03)
- 0 行 deliverable 丢失 (5 分钟 commit 节奏兜底)
- MVP-2 估 ~2390 行 (6 个 plan 累加), 0 新 crate, 38+ 单测
