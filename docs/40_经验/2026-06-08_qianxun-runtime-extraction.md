# qianxun-runtime crate 抽取 项目日记

**时间**: 2026-06-08 凌晨 + 中午 (跨 daemon 重启)
**目标**: 把 5 核心从 qianxun binary 抽成独立 crate, 给 4a-2 Tauri desktop 集成铺路
**作者**: Mavis (按 maxu 要求, 项目日记风格, 不写成正式 ADR)
**前置 commit**: `ef48ea1` (rename daemon → runtime)
**本 commit**: `456d12a` (抽 qianxun-runtime)

---

## 背景

千寻 `qianxun/src/runtime/` 里有 5 个核心模块 (agent_host / output_sink / persistence /
session_runtime / sse), 它们跟业务 (HTTP routing / admin / Web UI serving) 紧耦合在
`qianxun` binary 里. 接下来要做 4a-2 Tauri desktop 集成 (见
`docs/30_子项目规划/04b-tauri-runtime-integration.md`), 桌面端要复用这 5 个核心,
但 desktop 是独立 binary, 不能直接 `mod` 进 qianxun binary.

唯一解法: 抽独立 crate `qianxun-runtime`, 跟 `qianxun-core` / `qianxun-memory` 平级,
qianxun binary + qianxun-desktop 各自 path dep 它, **业务 0 重复**.

规划文档 `docs/30_子项目规划/04c-qianxun-runtime-extraction.md` (654 行) 写的是
"应该怎么做", 这篇日记写 "实际踩了什么坑, 走完一遍回头看哪些设计是对的哪些可以再优化".

---

## 时间线

### 2026-06-08 凌晨 — 完成代码改造 (在 daemon 重启前)

按 04c 9 步走, 一气呵成:

1. 建 `qianxun-runtime/Cargo.toml` 空壳, dep `qianxun-core` + `qianxun-memory` + tokio 等
2. 5 核心 1:1 搬到 `qianxun-runtime/src/{agent_host,output_sink,persistence,session_runtime,sse}.rs`
3. 新增 `qianxun-runtime/src/state.rs` — `RuntimeState` 9 字段, 从原 `qianxun/src/runtime/mod.rs::run()`
   启动逻辑 1:1 抽出, 包含 `new_for_test()` + `new_in_memory_with_config()` 两个 helper
4. 拆 `qianxun/src/runtime/sse_axum.rs` (25 行) — axum 包装层, 业务 SseEvent 在 qianxun-runtime,
   axum glue 留 qianxun binary
5. `qianxun/Cargo.toml` 加 `qianxun-runtime = { path = "../qianxun-runtime" }`
6. `qianxun/src/runtime/{mod.rs, router.rs, llm_integration_tests.rs, mvp1_integration_tests.rs}`
   跟着改: use 路径 + 测试用 `RuntimeState::new_for_test()` 替换手搓 state
7. 跑 `cargo test --workspace` → 248 passed

**没在 commit 时做的事** (当时留下了):
- 注释/日志里 36 处 `[daemon]` tag (4 个 .rs)
- 2 个 unused import warning (`ResolvedConfig` / `SkillManager` / `ToolRegistry` / `MemoryCore` /
  `create_provider` / `watch` / `LogRing`)

### 2026-06-08 中午 — daemon 重启, session 被打断

daemon 进程挂了重启, session 断在 commit 之前. 我接手时 working tree 是干净的 (uncommitted),
所有改动 cargo check 通过, 248 个测试全绿. 但有上面那 10 个 warning + 1 个 clippy warning.

### 接手后做的事 (清理 + commit)

1. **清 unused import**:
   - `qianxun/src/runtime/mod.rs` 测试模块: 6 个 import 不再需要 (state 构造走 `RuntimeState::new_for_test()`)
   - `qianxun/src/runtime/router.rs` 测试模块: 4 个 import (同上原因, + `LogRing` 漏掉)
2. **修 clippy warning**: `MemoryCore::open(&PathBuf::from(mem_path))` → `MemoryCore::open(PathBuf::from(mem_path))`
   (`&PathBuf::from(...)` 是 borrow expression, clippy 抱怨)
3. **清 36 处 `[daemon]`** → `[runtime]` (4 个 .rs, 纯 sed 替换, 30 秒)
4. **写 commit message + 提交**: 18 files, 6410/-5787

最终: cargo test 248/0, clippy 0/0, release build 17.6s.

---

## 关键设计决策

### 1. RuntimeState 9 字段, 业务 0 重复

按 04c §1.2 设计, RuntimeState 只放"跨 binary 通用"9 字段, 6 个 daemon-specific 字段
(admin / llm_providers / active_conns / log_ring / started_at / processing_loop_enabled)
留在 `qianxun/src/runtime/mod.rs::AppState`, 嵌入 `Arc<RuntimeState>`.

**实战效果**:
- `qianxun/src/runtime/mod.rs::AppState` 现在 2 字段 (`runtime: Arc<RuntimeState>` + 自己的
  daemon-specific 字段), 比抽 crate 前 (15 字段) 干净很多
- `qianxun/src/runtime/router.rs` 50+ 处 `state.xxx` → `state.runtime.xxx` 替换, 编译器报错
  一个个改, 1-2 小时搞定
- 集成测试用 `RuntimeState::new_for_test()` 1 行构造, 替换手搓 30+ 行 (provider + tools +
  memory + skills + shared + agent_host + store + shutdown_tx) — 业务 0 重复, ✅ 目标达成

**回头看**: 9 字段 / 6 字段这个拆分是**对的**. 6 个 daemon-specific 字段都不该跨 binary
共享 (admin auth 是 daemon HTTP endpoint 用的, llm_providers 是 daemon 配置的, active_conns
是 HTTP conn 计数, log_ring 是 daemon 进程内的 ring buffer). 强行塞 RuntimeState 会让 desktop
背一堆不需要的字段.

### 2. sse_axum.rs 25 行单独文件

原本想把 sse 整块都塞 qianxun-runtime, 但发现 25 行 axum 包装 (`SseEvent` → `axum::response::sse::Event`)
是 daemon 唯一需要的 (desktop 走 Tauri invoke, 不走 SSE wire, 见 ADR-0003).

**实战效果**:
- `qianxun-runtime/src/sse.rs` 包含 `SseEvent` enum + `SseEventBuilder`, 纯业务 (13 个变体 + JSON 序列化)
- `qianxun/src/runtime/sse_axum.rs` 包含 1 个 `event_from_sse()` 函数, 把业务 SseEvent 转 axum 帧
- 干净分层, desktop 想用 sse (unlikely 但可能) 走 Tauri command, 不需要碰 axum

**回头看**: 这个拆分是**对的**. 25 行不算多, 但把"axum 依赖"明确圈在 qianxun binary 里, 防止
qianxun-runtime 间接依赖 axum (否则 desktop path dep qianxun-runtime 会拖整个 axum 进去, 编译变慢).

### 3. 集成测试的 "RuntimeState::new_for_test()" helper

按 04c §3 step 8 设计, RuntimeState 提供 2 个测试入口:
- `new_for_test()` — 全 in-memory, 跟旧 `mod.rs::make_test_state()` 1:1
- `new_in_memory_with_config()` — 真 config + 真 provider, 但 store 走 `:memory:`, 避免污染 `~/.qianxun/daemon.db`

**实战效果**:
- `qianxun/src/runtime/{mod, router}.rs` 测试模块的 import 从 6-8 个 `use` 砍到 0-3 个
- `mod.rs::make_test_state()` 从 17 行 (手动 create_provider + tools + memory + shared + agent_host +
  store + shutdown_tx + Arc 包) 砍到 4 行 (一行 `new_for_test()` + 3 行 AppState 嵌入)
- 测试隔离性 1:1 保持, 247/248 测试 0 改动逻辑直接跑通

**回头看**: 这个 helper 是**关键收益**, 之前 `make_test_state` 跟 `RuntimeState::new()` 有 30% 重复
(provider + memory + skills 初始化逻辑), helper 一抽, 重复清零, 后续维护只改 1 处.

### 4. workspace `members` 顺序

`Cargo.toml` workspace members 加 `qianxun-runtime` 时, 放在 `qianxun-core` 之后,
`qianxun-memory` 之前 — 按依赖深度排, 看起来更顺.

**实战效果**: 0, cargo workspace 不关心顺序, 但 grep `Cargo.toml` 时能一眼看出依赖关系.

### 5. `[daemon]` → `[runtime]` 日志 tag

36 处日志 tag, 4 个 .rs:
- `qianxun-runtime/src/agent_host.rs`: 13
- `qianxun/src/runtime/mod.rs`: 16 (启动序列 + 优雅关停)
- `qianxun/src/runtime/router.rs`: 5
- `qianxun/src/main.rs`: 2

**做法**: PowerShell `Get-Content -Raw | ForEach-Object { $_ -replace '\[daemon\]', '[runtime]' } | Set-Content -NoNewline`
4 个文件一次改完, 跑测试验证.

**风险**: 0 — 纯字符串替换, 不会破语义 (tag 就是给人看的, 跟 grep / log filter 配套, 没有代码逻辑依赖).

**回头看**: 这个清理**早该做**, 留到 ef48ea1 commit 之后过了 1 个 commit 才做是失误.
下次类似 rename 应该**一次到位**: 改 .rs 文件名 + 改目录 + 改内部 use + 改日志 tag + 改 commit msg
一气呵成, 不要拆 2 个 commit.

### 6. 04c 规划文档的 `[daemon]` 字样保留

654 行规划里到处都是 "daemon" — 在引述旧 commit (ef48ea1 "rename daemon → runtime") /
历史 (Stage 8a "real LLM E2E daemon 8a") / 上位规划 (04b "sub-task #1") 等.

**判断**: 这些是**历史引述**, 不动. 改了反而失真 (让人误以为规划从来没用过 "daemon" 这个名字).

**回头看**: 这是**对的** — 文档可以保留旧术语作为历史锚点, 改 commit / 代码可以, 改历史叙事不行.

---

## 踩过的坑

### 1. PowerShell `Get-ChildItem -Recurse` 跑进 `node_modules` 出错

我开始数 `[daemon]` 残留时, `Get-ChildItem -Recurse -Path 'qianxun/src' -Include '*.rs'`
会把 `qianxun/src/runtime/ui/node_modules/` 也卷进来, 报一堆 "Could not find a part of the path"
(因为 node_modules 在 .gitignore 但物理存在 + Windows 文件锁 + 路径过长).

**修法**: 切到 `rg` (ripgrep), `-g '!qianxun/src/runtime/ui/node_modules/**'` 显式排除.

**教训**: 千寻 `qianxun/src/runtime/ui/` 是个 SvelteKit 项目, 里面 `node_modules/` 巨深, 任何
`-Recurse` 都要先想清楚是否要绕开. **rg 默认尊重 .gitignore**, 优先用 rg 不用 GCI.

### 2. `[daemon]` tag 数错了

我先说"139 处", 后来 grep 一查, 实际 `[daemon]` (方括号) 只有 36 处. 139 是泛指整个项目
(含 `04c` 文档 / `ef48ea1` commit msg / 一些旧 `sse.rs` 注释里不带方括号的 "daemon" 字样)
残留的 "daemon" 字符串总数.

**教训**: 写 commit msg 之前**先 grep 准确数字**, 别说"约 N 处" / "很多处". 数字错了 commit msg
就是地雷, 别人 review 会抓. 这块我做错了, 还好发现早, 没写进 commit msg.

### 3. 集成测试改 `make_test_state` 时删了 `use` 但没 cargo test

改 `qianxun/src/runtime/mod.rs` 测试模块, 删了 6 个 `use` 没及时 cargo test, 后来跑测试才发现
剩 4 个 warning (`LogRing` 跟 `watch` 跟 `PathBuf` 等), 又得返工.

**教训**: 改测试模块的 use 必须**改完立即 cargo test**, 别"先改完所有文件再统一测" — 改 5 个
use 编译器只会报 5 个 warning, 你看不到哪里冗余了, 只有 cargo test + 看 warning 列表才知道.
应该: 改 1 个 use → cargo test → 看到 1 个 warning 消失 → 继续下一个.

### 4. PowerShell here-string 提心吊胆

git commit multi-line 之前踩过坑 (2026-06-05 memory), 这次绕过去了 — 用 `git commit -F file.txt`
不直接传 here-string, 写到临时文件, 然后 `-F`. 0 风险, 缺点是留一个 tmp 文件 (已清, 不污染).

**教训**: 复杂 commit message (>20 行) 一律 `git commit -F file.txt`, 不在命令行传 multi-line.
PowerShell 跟 bash 跟 zsh 对 here-string 解析略有不同, `-F` 是最 portable 的写法.

---

## 没在本 commit 做 / 留给后续 sub-task

- `qianxun-desktop/src-tauri/Cargo.toml` 加 `qianxun-runtime` dep → 04c §5 sub-task #2
- Tauri commands 注册 `RuntimeState` 方法 → sub-task #3
- 抽 `RuntimeApi` trait (daemon router + Tauri command 共用) → sub-task #3
- 退役 `qianxun/src/runtime/ui/` (旧 SvelteKit, 已被 qianxun-desktop SvelteKit 替代) → 后续清理

这些不属于本次, 留 explicit TODO 文档, 不在 working tree 留桩.

---

## 验证清单 (04c §3 验收, 实际跑过的)

- [x] `cargo test -p qianxun-runtime` 5 核心 + state 测试全 pass (44/0)
- [x] `cargo test --bin qx` daemon 现有测试 (graceful_shutdown / mvp1_integration /
      llm_integration / stage7a_endpoint) 全 pass (147/0, 4 ignored)
- [x] `cargo test --workspace` 整体 248 passed, 0 failed, 4 ignored
- [x] `cargo clippy --workspace --all-targets` 0 warning
- [x] 5 旧 .rs 已从 `qianxun/src/runtime/` 搬走, 新 crate `qianxun-runtime/src/` 完整
- [x] desktop `Cargo.toml` **不**改 (sub-task #2 才加 dep)
- [x] 经验沉淀到 `docs/40_经验/` (本文件)

---

## 教训总结 (跨项目可复用)

1. **跨 crate 重构用 git rename detection 跨出 90%+ similarity**: 证明拆分对了, 99% similarity
   说明两文件几乎无逻辑差异, 只改了 use 路径. 这是 04c 设计的成功信号.
2. **测试 helper (`new_for_test()`) 提前写**: 别等老测试 1 个 1 个改完再统一抽 helper, 一开始
   就先把 helper 写到 qianxun-runtime, 然后用 helper 替换所有手搓 state, 1 步到位.
3. **规划文档 + 详细设计文档 + 项目日记 3 文档配套**:
   - `04c-qianxun-runtime-extraction.md` (规划, 654 行) — 写"应该怎么做", 在 commit 前写
   - 本文件 (`40_经验/2026-06-08_...`) — 写"实际踩了什么", 在 commit 后写
   - 二者内容不重复, 一个面向未来参考 (规划), 一个面向历史回溯 (日记)
4. **跨 binary 抽 crate 5 步法** (这次验证可行):
   1. 新 crate 空壳 (Cargo.toml + lib.rs)
   2. 1:1 搬代码 (git rename)
   3. 抽公共 helper (RuntimeState)
   4. 改 use 路径 (编译器报错逐个改)
   5. 改测试用 helper (替换手搓 state)
   5 步每步 cargo test, 5 步完 = 全绿
