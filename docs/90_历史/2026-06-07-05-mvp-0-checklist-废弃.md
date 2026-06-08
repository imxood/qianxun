# 千寻 MVP-0 详细任务清单 (修复缺口 7)

> 配套设计: `04-kanban-design.md` §14.1 MVP-0 + v6 报告 §3.1 F7 + §3.2 缺口 7
> 源报告: `E:/git/maxu/qianxun/.mavis/plans/qianxun-multi-agent-architecture.md` (v6)
> 同步日期: 2026-06-03
> 维护: 本文件跟 v6 设计同步, 改 v6 时也要看本文件是否要改

## 执行摘要 (5 行)

- **目标**: 修复缺口 7 — `qianxun/src/daemon/mod.rs:100-198` 的 `AppState.tools/skills/memory` 从"空/in_memory 占位"改成"启动时真实初始化"
- **预期改动**: 新增 ~280 行, 改 ~60 行 = **~340 行**, **0 个新 crate**, 0 个新 SQL 表
- **验收**: `cargo run --bin qx -- daemon` 启动后, `curl /v1/tools` 返 ≥8 个 builtin, `curl /v1/skills` 返 ≥1 个, `curl /v1/memory/ping` 返 `{"status":"ok"}`
- **时长**: 1 周 (5 工作日), 单人可干
- **关键风险**: DB 启动慢 / skill 加载失败 / 启动序列不对 → graceful fallback, 不让 daemon 起不来

---

## 关键决策表 (6 个)

| # | 决策点 | 选项 | 推荐 | 理由 |
|---|---|---|---|---|
| D1 | MemoryCore 启动路径? | (a) `MemoryCore::open(&path)` 真 SQLite (b) `open_in_memory()` (c) 异步预热 | **(a) `open("~/.qianxun/mem.db")`** | 千寻已有 8 表 + 18 集成测试 pass, 直接用. (b) 是当前占位实现, 缺持久化 |
| D2 | ToolRegistry 启动序列? | (a) eager 全部注册 (b) lazy 按需注册 (c) 两阶段 | **(a) eager 全部** | builtin 只有 8 个, 启动 < 50ms, 不需要 lazy |
| D3 | SkillManager 加载时机? | (a) 启动时同步 (b) 启动时异步 (c) 首次访问时 | **(a) 启动时同步** | 千寻 daemon 是常驻服务, 启动一次 OK; 异步增加复杂度, 没收益 |
| D4 | hot-reload MVP-0 要不要做? | (a) 做完 (b) 留 v2 | **(b) 留 v2** | MVP-0 目标只是"启动时真实初始化", 启动后改 TOML 不需要立刻生效. 加 SkillWatcher 但**只 watcher, 不 reload** |
| D5 | 启动失败 fallback 策略? | (a) 任一组件失败都 panic (b) 失败转 in_memory 占位 + warn 日志 (c) daemon 拒启 | **(b) 失败转 in_memory + warn** | 千寻是单 daemon 部署, daemon 拒启 = 用户没法用. fallback 到 in_memory (当前行为) 不破坏功能, 加日志可观察 |
| D6 | SkillWatcher MVP-0 装不装? | (a) 装, 后台 tick (b) 不装 | **(b) 不装, 留 v2** | watcher 需要 tokio task + mpsc 通道, MVP-0 范围已大. 注释清楚 v2 接上即可 |

---

## 天级任务清单

### Day 1 (周一) — 缺口 7 第 1 步: ToolRegistry 真实初始化

#### 1.1 抽 `register_all_builtin_tools()` 函数 (2-3h)

**文件**: `qianxun-core/src/tools/mod.rs` (改, +60 行)

**改动类型**: 改 (新加 1 个函数)

**代码骨架**:
```rust
// qianxun-core/src/tools/mod.rs (在 ToolRegistry impl 块内, L170 附近)

impl ToolRegistry {
    /// MVP-0: 注册所有 8 个 builtin 工具, 千寻启动时一次性调用
    pub fn register_all_builtin(&mut self) -> Result<usize, ToolError> {
        let mut count = 0;
        // 8 个 builtin 工具: read_text_file, write_text_file, search, grep,
        // list_directory, execute_command, edit_file, skill_read
        // 现有实现: 每个工具的 Arc<dyn AgentTool> 通过 ToolBuilder 构造
        for tool in builtin::all_builtin_tools() {
            self.register_builtin(tool);
            count += 1;
        }
        Ok(count)
    }
}
```

**验收命令**: `cargo build -p qianxun-core` (无编译错), `cargo test -p qianxun-core tools::` (现有测试 pass)

**预期产物**: `qianxun-core/src/tools/mod.rs` 多 1 个 `register_all_builtin()` 函数 + 1 个 `builtin` 子模块声明

**预计耗时**: 2-3h

**风险**: builtin::all_builtin_tools() 函数可能不存在, 需新建 `qianxun-core/src/tools/builtin/mod.rs` 把 8 个工具集中暴露. [A] 风险中等, 1h 内可解决

#### 1.2 改 AppState.tools 字段类型 (1-2h)

**文件**: `qianxun/src/daemon/mod.rs:43-65` (改, +20 行)

**改动类型**: 改 (字段初始化方式)

**代码骨架**:
```rust
// qianxun/src/daemon/mod.rs:43-49

pub struct AppState {
    pub agent_host: Arc<AgentLoopHost>,
    pub config: Arc<ResolvedConfig>,
    pub provider: Arc<dyn LlmProvider>,
    // MVP-0: 改 None 占位为初始化后的 Arc
    pub tools: Arc<ToolRegistry>,           // 改, 之前是 Option 或 in_memory
    pub skills: Arc<SkillManager>,         // 改 (Day 2 加)
    pub memory: Arc<MemoryCore>,           // 改 (Day 3 加)
    // ... 其他字段不变 ...
}
```

**验收命令**: `cargo build` 仍 pass, 改的字段类型不破坏调用方

**预计耗时**: 1-2h

**风险**: 调用方可能假设 `tools` 是 Option, 需 grep 找全改完. [A] 低风险

#### 1.3 启动序列调 register_all_builtin (1h)

**文件**: `qianxun/src/daemon/mod.rs` (在 `run()` 函数启动处, +30 行)

**改动类型**: 改 (启动序列)

**代码骨架**:
```rust
// qianxun/src/daemon/mod.rs:run() 函数, 在创建 AppState 之前

let mut tools = ToolRegistry::new();
match tools.register_all_builtin() {
    Ok(n) => tracing::info!(registered = n, "builtin tools registered"),
    Err(e) => {
        tracing::error!(error = ?e, "register_all_builtin failed, fallback to empty");
        // 决策 D5: fallback 到空 registry + warn, 不 panic
    }
}

let state = AppState {
    tools: Arc::new(tools),
    // ... 其他字段 ...
};
```

**验收命令**: `cargo run --bin qx -- daemon --port 23900 &`, 启动后 `curl -s http://127.0.0.1:23900/v1/tools | jq '.tools | length'`, 期望 `>= 8`

**预计耗时**: 1h

**风险**: 启动序列可能改 state 字段类型, 跟其他初始化逻辑有依赖. 需先看 `daemon/mod.rs:100-198` 完整代码. [A] 中等

#### 1.4 写集成测试 (1-2h)

**文件**: `qianxun-core/tests/builtin_init.rs` (新, +60 行)

**改动类型**: 新 (集成测试)

**代码骨架**:
```rust
// qianxun-core/tests/builtin_init.rs

use qianxun_core::tools::ToolRegistry;

#[test]
fn builtin_registry_loads_eight_tools() {
    let mut registry = ToolRegistry::new();
    let n = registry.register_all_builtin().expect("register");
    assert!(n >= 8, "expected >= 8 builtin tools, got {}", n);
}

#[test]
fn builtin_tools_have_unique_names() {
    let mut registry = ToolRegistry::new();
    registry.register_all_builtin().unwrap();
    let names: Vec<String> = registry.list_names();
    let unique: std::collections::HashSet<_> = names.iter().cloned().collect();
    assert_eq!(names.len(), unique.len(), "duplicate tool names");
}
```

**验收命令**: `cargo test -p qianxun-core --test builtin_init`

**预计耗时**: 1-2h

**风险**: 现有 test infrastructure 可能假设其他初始化. [A] 低

#### 1.5 commit + 自我 review (1h)

**提交信息模板**:
```
[Day 1/5] [MVP-0] 修复缺口 7: ToolRegistry 真实初始化

- 加 ToolRegistry::register_all_builtin() 函数
- 改 AppState.tools 字段为 Arc<ToolRegistry>
- 启动序列调 register_all_builtin, 失败 fallback
- 加 builtin_init 集成测试 (2 个 case)
- 验收: cargo test pass, daemon 启动后 /v1/tools 返 >= 8
```

**预计耗时**: 1h

---

### Day 2 (周二) — 缺口 7 第 2 步: SkillManager 真实加载

#### 2.1 改 AppState.skills 字段 (1-2h)

**文件**: `qianxun/src/daemon/mod.rs:43-65` (改, +15 行)

**代码骨架**:
```rust
// 启动序列 (在 Day 1 1.3 之后)

let skills = match SkillManager::load_all(None) {
    Ok(mgr) => {
        tracing::info!(count = mgr.skill_count(), "skills loaded");
        Arc::new(mgr)
    }
    Err(e) => {
        tracing::warn!(error = ?e, "load_all skills failed, fallback to empty");
        // D5: fallback 不 panic
        Arc::new(SkillManager::new())
    }
};

let state = AppState {
    // ...
    skills,  // Arc<SkillManager>
    // ...
};
```

**验收命令**: `cargo build`, `cargo test -p qianxun-core skills::`

**预计耗时**: 1-2h

**风险**: `SkillManager::load_all` 当前签名是 `Option<&Path>`, 跟 v1 single-project 兼容 (None = 走全局). [A] 低

#### 2.2 集成测试: 验证 skill 加载 (1-2h)

**文件**: `qianxun-core/tests/skill_init.rs` (新, +50 行)

**代码骨架**:
```rust
// qianxun-core/tests/skill_init.rs

use qianxun_core::skills::SkillManager;

#[test]
fn skill_manager_loads_from_global_dir() {
    let mgr = SkillManager::load_all(None).expect("load_all");
    let count = mgr.skill_count();
    // 千寻默认应该有至少 1 个内置 skill
    assert!(count >= 1, "expected >= 1 skill, got {}", count);
}
```

**验收命令**: `cargo test -p qianxun-core --test skill_init`

**预计耗时**: 1-2h

**风险**: 测试环境 `~/.qianxun/skills/` 可能不存在, 需要在测试 setup 里建临时目录 + 1 个 fake skill file. [A] 低-中

#### 2.3 加 1 个端点 /v1/skills 验证 (2-3h)

**文件**: `qianxun/src/daemon/router.rs` (改, +40 行)

**代码骨架**:
```rust
// 沿用 §8.5 契约, 在现有 router 里加

async fn list_skills(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let names = state.skills.available_skills();
    Ok(Json(json!({ "skills": names, "count": names.len() })))
}

// 路由注册:
.route("/v1/skills", get(list_skills))
```

**验收命令**: 启动 daemon, `curl -s http://127.0.0.1:23900/v1/skills | jq '.count'`, 期望 `>= 1`

**预计耗时**: 2-3h

**风险**: 现有 router 的鉴权中间件可能拦住, 需看 `router.rs:177-284` Claims/auth_middleware. [A] 中等

#### 2.4 commit (30 min)

**预计耗时**: 30 min

---

### Day 3 (周三) — 缺口 7 第 3 步: MemoryCore 真路径

#### 3.1 改 AppState.memory 字段 (2h)

**文件**: `qianxun/src/daemon/mod.rs` (改, +25 行)

**代码骨架**:
```rust
// 启动序列

let mem_path = dirs::home_dir()
    .unwrap_or_else(|| PathBuf::from("."))
    .join(".qianxun")
    .join("mem.db");

let memory = match MemoryCore::open(&mem_path) {
    Ok(core) => {
        tracing::info!(path = ?mem_path, "memory opened");
        Arc::new(core)
    }
    Err(e) => {
        tracing::warn!(error = ?e, path = ?mem_path, "memory open failed, fallback to in_memory");
        // D5: fallback 不 panic
        MemoryCore::open_in_memory().unwrap_or_else(|_| /* 极端 fallback: 空 */)
    }
};
```

**验收命令**: `cargo build`

**预计耗时**: 2h

**风险**: `dirs` crate 可能没引, 需 `use std::path::PathBuf` 替代 + `std::env::var("HOME")`. [A] 低

#### 3.2 加 /v1/memory/ping 端点 (2h)

**文件**: `qianxun/src/daemon/router.rs` (改, +30 行)

**代码骨架**:
```rust
async fn memory_ping(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    // 简单 ping: 调一个 read-only 内存查询, 验证可访问
    let stats = state.memory.stats();
    Ok(Json(json!({
        "status": "ok",
        "observations": stats.observation_count,
        "memories": stats.memory_count,
    })))
}
```

**验收命令**: `curl -s http://127.0.0.1:23900/v1/memory/ping | jq .status`, 期望 `"ok"`

**预计耗时**: 2h

#### 3.3 集成测试: Memory 真路径持久化 (1-2h)

**文件**: `qianxun-core/tests/memory_init.rs` (新, +60 行)

**代码骨架**:
```rust
// 用 tempfile crate 创临时 db, 写一条 record, 重新 open, 验证还在
```

**预计耗时**: 1-2h

#### 3.4 commit (30 min)

**预计耗时**: 30 min

---

### Day 4 (周四) — 端到端验证 + clippy

#### 4.1 跑 E2E 验收脚本 (2-3h)

**执行** (在临时跑 daemon 的环境):
```bash
# 1. 启动 daemon
cargo build --release
./target/release/qx daemon --port 23900 &
DAEMON_PID=$!
sleep 3

# 2. 验证 3 个核心端点
echo "=== /v1/tools ==="
curl -s http://127.0.0.1:23900/v1/tools | jq '.tools | length'
# 期望: >= 8

echo "=== /v1/skills ==="
curl -s http://127.0.0.1:23900/v1/skills | jq '.count'
# 期望: >= 1

echo "=== /v1/memory/ping ==="
curl -s http://127.0.0.1:23900/v1/memory/ping | jq '.status'
# 期望: "ok"

# 3. 跑完整 cargo test
cargo test --workspace

kill $DAEMON_PID
```

**期望**: 3 个端点都返合理值, cargo test 全 pass

**预计耗时**: 2-3h (含调试)

**风险**: 端点路径 / 鉴权可能跟我们 v6 文档不一致, 跑起来发现差再调. [A] 中等

#### 4.2 跑 clippy (1h)

**执行**:
```bash
cargo clippy --all -- -D warnings 2>&1 | tee /tmp/clippy.log
```

**期望**: 0 warnings, 0 errors

**修复**: clippy 报的 lints, 逐个修

**预计耗时**: 1h

**风险**: clippy 可能会报一些"风格上对但功能上不影响"的 lints, 比如 `needless_lifetimes` / `redundant_clone`, 都是机械修复. [A] 低

#### 4.3 修测试覆盖空洞 (1-2h)

- 跑 `cargo test --workspace`, 看有没有失败/忽略
- 修明显的测试缺漏 (如 fallback 路径的测试)
- 文档化已知跳过的测试

**预计耗时**: 1-2h

#### 4.4 commit (30 min)

**预计耗时**: 30 min

---

### Day 5 (周五) — 文档 + 收尾

#### 5.1 更新 CLAUDE.md (1h)

**改动**: 在 "## 模块结构" 加一节说明 `daemon/mod.rs` AppState 14 字段现在全部是真实初始化, 不再有 None 占位

#### 5.2 更新 docs/30_子项目规划/01-daemon.md (1h)

**改动**: 在"实际进度"部分标 ✅ "缺口 7 修复 (MVP-0 落地)", 引用 PR 编号

#### 5.3 写 PR description (1h)

**模板**:
```markdown
## [MVP-0] 修复缺口 7: AppState 真实初始化

### 背景
- v6 报告 §3.2 缺口 7: AppState.tools/skills/memory 是空/in_memory 占位
- 见 04-kanban-design.md §14.1 MVP-0

### 改动
- 加 ToolRegistry::register_all_builtin() (~60 行)
- 加 SkillManager 启动加载 (~30 行)
- 加 MemoryCore::open 真路径 (~30 行)
- 加 2 个新端点 /v1/skills /v1/memory/ping (~70 行)
- 改 AppState 字段类型, 加 fallback (~60 行)
- 加 3 个集成测试 builtin_init / skill_init / memory_init (~170 行)
- 总计: ~340 行, 0 新 crate, 0 新 SQL 表

### 验收
- cargo test --workspace pass
- cargo clippy --all -- -D warnings 0 警告
- 启动 daemon, /v1/tools >= 8, /v1/skills >= 1, /v1/memory/ping status=ok
```

#### 5.4 跑完整 E2E 一遍 (1h)

**重跑** Day 4 4.1 脚本, 录屏, 写进 PR

#### 5.5 求 review + merge (1h)

- 提交 PR, 自己 review 一遍
- 找一个 reviewer (找 worker 同事) 签核
- merge, 关 issue

**预计耗时**: 1h

---

## 风险与缓解 (5 个)

| # | 风险 | 触发条件 | 缓解 |
|---|---|---|---|
| K1 | DB 启动超时 (`MemoryCore::open` 卡 30s+) | 磁盘满 / DB 锁 / 文件权限 | D5 已选 fallback 到 `open_in_memory`, 加 5s timeout, warn 日志明确告知用户 |
| K2 | SkillManager::load_all panic | `~/.qianxun/skills/` 目录权限错 / TOML 解析错 | D5 fallback 到 `SkillManager::new()` (空), warn 日志. 后续启动期重试 |
| K3 | register_all_builtin panic | 某个 builtin 工具构造期 panic | try/catch 包住, 跳过出错的工具, warn 继续 |
| K4 | 端点鉴权拦 | router 鉴权中间件配置错 | 跟现有 /v1/tools 端点对齐 (已是允许状态), 复制其 pattern |
| K5 | clippy 报"看似对"的风格错 | 比如 `redundant_clone`, 改完测试要能跑 | 改完跑一遍 cargo test 确认行为不变 |

---

## 端到端验收脚本 (一键跑)

```bash
#!/usr/bin/env bash
# mvp-0-e2e-verify.sh
# 在 git 仓库根目录跑
set -euo pipefail

echo "=== MVP-0 端到端验收 ==="

# 1. Build
echo "[1/6] cargo build --release"
cargo build --release 2>&1 | tail -5

# 2. 启动 daemon
echo "[2/6] 启动 daemon (后台)"
./target/release/qx daemon --port 23900 &
DAEMON_PID=$!
sleep 3

# 3. /v1/tools 返 >= 8 builtin
echo "[3/6] /v1/tools 验证"
TOOLS_COUNT=$(curl -s http://127.0.0.1:23900/v1/tools | jq '.tools | length')
[ "$TOOLS_COUNT" -ge 8 ] || { echo "FAIL: tools=$TOOLS_COUNT < 8"; kill $DAEMON_PID; exit 1; }
echo "    OK: $TOOLS_COUNT tools"

# 4. /v1/skills 返 >= 1
echo "[4/6] /v1/skills 验证"
SKILLS_COUNT=$(curl -s http://127.0.0.1:23900/v1/skills | jq '.count')
[ "$SKILLS_COUNT" -ge 1 ] || { echo "FAIL: skills=$SKILLS_COUNT < 1"; kill $DAEMON_PID; exit 1; }
echo "    OK: $SKILLS_COUNT skills"

# 5. /v1/memory/ping 返 status=ok
echo "[5/6] /v1/memory/ping 验证"
MEM_STATUS=$(curl -s http://127.0.0.1:23900/v1/memory/ping | jq -r '.status')
[ "$MEM_STATUS" = "ok" ] || { echo "FAIL: memory.status=$MEM_STATUS != ok"; kill $DAEMON_PID; exit 1; }
echo "    OK: status=$MEM_STATUS"

# 6. cargo test --workspace
echo "[6/6] cargo test --workspace"
cargo test --workspace 2>&1 | tail -10

kill $DAEMON_PID 2>/dev/null || true

echo "=== 全部通过, MVP-0 验收完成 ==="
```

**执行**: `chmod +x mvp-0-e2e-verify.sh && ./mvp-0-e2e-verify.sh`

---

## 交付物清单

- [ ] **1 个新 PR**: 标题 `[MVP-0] 修复缺口 7: AppState 真实初始化`, 描述按 §5.3
- [ ] **6 个 commit** (按天划分, 1.5/2.4/3.4/4.4/5.1-5.4):
  1. Day 1: ToolRegistry 真实初始化
  2. Day 2: SkillManager 真实加载
  3. Day 3: MemoryCore 真路径
  4. Day 4: E2E 验证 + clippy 0 警告
  5. Day 5a: CLAUDE.md 更新
  6. Day 5b-c: docs 更新 + PR description
- [ ] **1 个新文件**: `docs/30_子项目规划/01-daemon.md` 加"缺口 7 修复 ✅"段
- [ ] **1 个完成标志**: PR merge + E2E 脚本跑通 + clippy 0 警告

---

## 附录 A: 卡住怎么办

| 现象 | 排查方向 | 联系 |
|---|---|---|
| cargo build 失败, 编译错 | 看错信息的 file:line, 优先看是不是字段类型不匹配 | — |
| 启动 daemon 后立刻崩 | 看 `~/.qianxun/daemon.log` 的 panic 信息 | — |
| /v1/tools 返 0 个 | 是不是 `register_all_builtin` 静默失败? 加 `eprintln!` 看 | — |
| /v1/memory/ping 返 timeout | 是不是 `open(&path)` 卡 30s+? 加 timeout 包装 | — |
| 鉴权拦 | 跟现有 /v1/tools 对比, 鉴权配置应该一致 | — |

## 附录 B: 启动时序图 (ASCII)

```
qx daemon 启动
  │
  ├─ 加载配置 (config.toml)
  │
  ├─ 初始化 LLM provider
  │
  ├─ [Day 1] ToolRegistry::register_all_builtin()  ← eager 8 个工具
  │     └─ 失败? fallback 到空 + warn
  │
  ├─ [Day 2] SkillManager::load_all(None)         ← 同步加载全局 skill
  │     └─ 失败? fallback 到 SkillManager::new() + warn
  │
  ├─ [Day 3] MemoryCore::open("~/.qianxun/mem.db")  ← 真 SQLite 路径
  │     └─ 失败? fallback 到 open_in_memory() + warn
  │
  ├─ 创建 AppState { tools, skills, memory, ... }
  │
  ├─ 启动 router (25+ 路由)
  │
  ├─ 启动 TUI/ACP client handlers
  │
  ├─ ready: 接受 HTTP 请求
  │
  └─ END
```

---

**总耗时估算**: 5 工作日 (40 小时), 单个开发者可完成. 关键依赖: 编译过 (5 min/次), 启动 daemon 验证 (3 min/次).
