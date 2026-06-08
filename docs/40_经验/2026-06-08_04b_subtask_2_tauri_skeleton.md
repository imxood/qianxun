# 04b sub-task #2 Tauri 集成骨架 项目日记

**时间**: 2026-06-08 下午
**目标**: 把 Tauri 桌面端的 lib.rs (315 行单文件) 拆成 4 domain 平行结构 + 注入 RuntimeState, 给 04b sub-task #3 接真 runtime 铺路
**作者**: Mavis (按 maxu 要求, 项目日记风格, 不写成正式 ADR)
**前置 commit**: `456d12a` (qianxun-runtime crate 抽取) + `b380bd2` (经验沉淀)
**本 commit**: 待定 (4 个 commit 拆好, 等 maxu 手动跑 `git add && git commit`)

---

## 背景

`qianxun-desktop/src-tauri/src/lib.rs` 已经 315 行, 4 个 invoke command (health_check /
daemon_health_fetch / set_secret / get_secret) 全堆里面. 04b sub-task #3 还要加 5 个
runtime command (list_sessions / send_message / create_plan / cancel_session / load_session),
如果不先重构就直接加, lib.rs 会奔 600+ 行.

maxu 的明确要求: **"代码设计, 文件/文件夹的设计需要合理, 不能单个大文件一直追加"**.

唯一解法: 按 domain 拆成 4 个子目录 (commands/{health, stronghold, runtime} + events + state),
每个文件 < 200 行, 业务逻辑不动 (1:1 搬), lib.rs 瘦身到薄壳.

规划文档 `docs/30_子项目规划/04b-tauri-runtime-integration.md` sub-task #2 写的是 "应该怎么做",
这篇日记写 "实际踩了什么坑, 走完一遍回头看哪些设计是对的哪些可以再优化".

---

## 时间线

### 14:00 - 14:30 — 拆分设计 + 落地 (4 domain 平行结构)

按 4 domain 拆:

```
src/
├── lib.rs                              # 66 行薄壳 (装 plugin + setup + invoke_handler)
├── main.rs                              # 9 行, mobile 复用点 (现状保留)
├── commands/                            # 4 domain 平行
│   ├── mod.rs                          # 18 行, 子目录收口
│   ├── health/                         # 158 行合计, Stage 2 mock + 远程探活
│   │   ├── mod.rs                      #   19 行
│   │   ├── check.rs                    #   15 行 (health_check 本地 mock)
│   │   ├── fetch.rs                    #   82 行 (daemon_health_fetch 真实 fetch)
│   │   ├── mock.rs                     #   15 行 (offline_status helper)
│   │   └── types.rs                    #   27 行 (DaemonState + HealthStatus)
│   ├── stronghold/                      # 173 行合计, iota_stronghold 凭据加密
│   │   ├── mod.rs                      #   23 行
│   │   ├── key.rs                      #   32 行 (2 个 Tauri command 包装层)
│   │   ├── vault.rs                    #   84 行 (set/get 实际逻辑)
│   │   ├── keyprovider.rs              #   14 行 (Argon2 + blake2b KDF)
│   │   └── snapshot.rs                 #   20 行 (vault_snapshot_path)
│   └── runtime/                        #   14 行, 空 stub (sub-task #3 实)
│       └── mod.rs
├── events/                              # 24 行, emit 事件 schema 收口
│   ├── mod.rs                          #   9 行
│   └── state_changed.rs                #   15 行 (daemon://state-changed)
└── state/                               # 21 行, Tauri State 注入
    ├── mod.rs                          #   5 行
    └── runtime.rs                      #   16 行 (build() helper stub)
```

最大单文件: vault.rs 84 行, fetch.rs 82 行, lib.rs 66 行 (含注释). 目标 < 200 行 ✓.

### 14:30 - 14:50 — 编译踩坑 (5 个错误一次修)

**坑 1: tauri macro 不会通过 `pub use` 传递** (E0433)
- `commands::health::mod.rs` 用 `pub use check::health_check` 重新 export,
  lib.rs 用 `commands::health::health_check` 简写
- 错误: `cannot find __cmd__health_check in health`
- 原因: `#[tauri::command]` macro 在 `check.rs` 里生成 `__cmd__health_check` 辅助 pub 函数,
  `pub use` 只重新导出 `health_check` 函数本身, **不**带 `__cmd__xxx` 辅助项
- 解法: lib.rs 用完整路径 `commands::health::check::health_check`,
  每个 domain 的 mod.rs 不再 `pub use` 转发, 直接 `pub mod xxx;` 让外部能看见

**坑 2: 模块可见性 (`mod xxx;` vs `pub mod xxx;`)** (E0603)
- 把 `mod check;` 改成 `pub mod check;` 才能让 lib.rs 通过路径引用
- 内部模块 (`mock.rs` / `types.rs` / `vault.rs` / `keyprovider.rs` / `snapshot.rs`) 保持 `mod xxx;` 私有,
  这些是子目录内部实现的细节, 不应该从外部路径访问

**坑 3: `app.path()` / `app.manage()` 缺 `use tauri::Manager;`**
- Tauri 2.x 的 `App` / `AppHandle` 上的 `path()` 和 `manage()` 是 `Manager` trait 的方法,
  必须 `use tauri::Manager;` 才能调到
- 修了 2 处: `src/lib.rs` (manage) + `src/commands/stronghold/snapshot.rs` (path)

**坑 4: 旧版 clippy 警告被带过来** (lints::unnecessary_to_owned)
- 复制 `vault.rs` 时把 `.to_vec()` 留着, 触发 `clippy::unnecessary_to_owned`
- 1 处: `store.get(&key.as_bytes().to_vec())` → `store.get(key.as_bytes())`
- 这是新版 clippy 1.96.0 才警告的, 老代码 (`tests/stronghold_e2e.rs`) 也有 6 处同样问题
- **不**主动修老代码 (Stage 6a 留下来的), 不在 sub-task #2 范围

**坑 5: `state/mod.rs` 找不到 `runtime` 子模块** (E0583)
- Step 2.3 才写 `state/runtime.rs`, 但 Step 2.1 改了 `commands/mod.rs` 引用了 `state::runtime::build()`
- 编译错误: `file not found for module 'runtime'`
- 解法: Step 2.1 一开始就把 `state/runtime.rs` 写成空 stub (`Ok(RuntimeState::new_for_test())`)
  让编译过, 后续 Step 2.3 在那个基础上加真初始化逻辑就不再有错

### 14:50 - 15:00 — 验证

```powershell
# desktop 端 (含 5 个 Argon2 KDF 慢测试, 1 个 ignored)
cd E:\git\maxu\qianxun\qianxun-desktop\src-tauri
cargo check              # 0 error
cargo clippy --lib       # 0 warning
cargo test               # 5 passed + 1 ignored, 245s (Argon2 KDF 慢)

# workspace 全测试 (含 qianxun binary + qianxun-runtime + qianxun-core + qianxun-memory)
cd E:\git\maxu\qianxun
cargo test --workspace   # 248 passed + 5 ignored, 0 failed
```

### 15:00 - 15:10 — 写经验沉淀 (本文件)

---

## 设计决策 (按重要性排)

### 决策 1: 4 domain 平行, 不按 layer (controller / service / repo) 拆

**考虑过** 的方案:
- 按 layer 拆: `controllers/` (Tauri command 包装) + `services/` (业务) + `repos/` (数据访问)
- 按 domain 拆: `health/` + `stronghold/` + `runtime/` (采用)

**采用 domain** 因为:
- Tauri command 数量少 (sub-task #3 之后 9 个), 不需要 service 抽象
- 每个 domain 内部就是 "Tauri command 包装 + 业务 + 数据访问" 3 件套,
  强行按 layer 拆会让一个简单 command 横跨 3 个目录, 反而难找
- domain 边界清楚: health 跟 RuntimeState 无关, stronghold 跟 RuntimeState 无关,
  runtime 才接 RuntimeState. 按 layer 拆会让"是否接 RuntimeState"这个关键差异模糊掉

### 决策 2: 业务 1:1 搬, 不重写

`vault.rs` / `fetch.rs` / `check.rs` 三个文件的代码都是从老 `lib.rs` 1:1 复制,
包括 verifier 报告里 3 个 finding 的修法 (load_client 替代 get_client / try-create /
with_passphrase_hashed_blake2b KDF), 注释也照搬.

**好处**:
- sub-task #2 是 refactor, 不是新功能. 1:1 搬保证业务不变, 容易 revert
- verifier 已经 review 过这些 finding 修法, 不要再走 review 流程
- 后续 sub-task #3 加真 runtime command 时, 老 business logic 在 `commands/runtime/`
  子目录下独立演进, 跟 health/stronghold 完全无关

**坏处**:
- 代码量几乎不变 (净 +3 个 mod.rs + +2 个 events 文件 + +1 个 state 文件 = -lib.rs 240 行 → 实际净 +6 行)
- 但每个文件都 < 200 行, 局部改动影响局部

### 决策 3: `runtime/` 现在空 stub, sub-task #3 才填

`commands/runtime/mod.rs` 现在只有 14 行注释, 没有 `pub mod xxx;` 子模块.

**为什么不让 sub-task #2 顺手填 5 个 command 进去?**

技术上可以 (RuntimeState 已经能 new_for_test), 但**业务上不应该**:
- sub-task #3 的范围是 "5 个 command 接真 RuntimeState", 包括 (a) 调 RuntimeState 方法,
  (b) Svelte 端 stores 改 invoke. 这两步绑在一起才完整
- 如果 sub-task #2 只填一半 (Rust 端有了, Svelte 端还是 mock), 会留半个中间状态:
  Rust 端调用 RuntimeState 的方法返 mock 数据, 但 Svelte 端还是 setTimeout, 行为不一致
- 留到 sub-task #3 一起做, Rust 端 + Svelte 端一次性切完, 跑通 e2e

**为什么留 `commands/runtime/` 目录而不删?**

让 sub-task #3 加 5 个 command 时, 不用再开 `mod.rs` / `Cargo.toml` / `lib.rs invoke_handler` 这些样板,
直接 `pub mod sessions;` + 写 `sessions.rs` + lib.rs 加 5 行 handler 即可.

### 决策 4: events/ 单独目录, 不放在 commands/ 下

**考虑过** 的方案:
- 方案 A: events 嵌在 commands 里 (`commands/runtime/message_delta.rs` 既 export payload 也 export emit)
- 方案 B: events 独立目录 (`events/state_changed.rs` 等)

**采用方案 B** 因为:
- Tauri event 的 schema (事件名 + payload 类型) 是前后端共享契约 (见 `_shared-contract.md`)
- 命令 (commands/) 是 "前端发起后端执行", 事件 (events/) 是 "后端推送前端接收"
- 两者方向相反, 强行放一个目录会让 "什么时机 emit" / "什么时机 listen" 混乱
- events 独立目录后, 加新事件只需 `pub mod xxx;` + 写 `xxx.rs` + emit 时调 `events::xxx::emit(...)`,
  不污染 commands/

### 决策 5: state/runtime.rs 用 `new_for_test()` 不是 `new(config)`

**考虑过** 的方案:
- 方案 A: `build()` 走 `RuntimeState::new(config)`, 真初始化 (provider / tools / memory / skills / SessionStore)
- 方案 B: `build()` 走 `RuntimeState::new_for_test()`, 跳过 config 跟 LLM 依赖 (采用)

**采用方案 B** 因为:
- sub-task #2 是骨架, 不接真 runtime. 此时让 `RuntimeState::new(config)` 真跑通,
  需要从 `~/.qianxun/config.json` 读 `ResolvedConfig`, 还要从 `DEEPSEEK_API_KEY` 读 LLM key,
  缺一个就 fail
- desktop release 打包后, OS 隔离目录 (`app_local_data_dir()`) 可能未创建, 直接 panic
- 用 `new_for_test()` 永远成功, 让 lib.rs::run() 的 setup() 永远能 `app.manage(rt)`,
  不阻塞 desktop 启动
- 真正的 `RuntimeState::new(config)` 等 sub-task #3 接 5 个 command 时再替换,
  那时 5 个 command 都要 RuntimeState, 一起做能验证 config 跟 LLM 都通

**坏处**:
- 启动后 `manage()` 的 RuntimeState 是 in-memory, 没有真 SQLite,
  5 个 command 拿到它调 list_sessions 会返空 list (真没有持久化)
- 但 sub-task #2 没接 command, 这不是问题. sub-task #3 替换 `build()` 时一起解决

---

## 踩过的坑 (教训)

### 坑 1: tauri 2.x 的 `pub use` 不传递 macro 辅助项

之前没想到, 直接 `pub use check::health_check;` 然后 `commands::health::health_check` 调用,
编译报 `cannot find __cmd__health_check`.

**教训**: Tauri 2.x 的 `#[tauri::command]` macro 在原文件生成 `__cmd__xxx` (供 `generate_handler!` 引用),
这个辅助函数必须通过 `pub mod xxx;` 让外部 mod tree 能直接访问, 不能用 `pub use` 跨文件转发.

**用法**:
```rust
// 在 lib.rs invoke_handler 用完整路径:
.invoke_handler(tauri::generate_handler![
    commands::health::check::health_check,        // 注意是 check, 不是 health
    commands::stronghold::key::set_secret,       // 注意是 key, 不是 stronghold
])
```

### 坑 2: Tauri `app.path()` / `app.manage()` 必须 `use tauri::Manager;`

Tauri 2.x 把这些常用方法挂到 `Manager` trait, 不在 `App` / `AppHandle` 的 inherent impl 上.
不 import trait 就调不到.

**用法**:
```rust
use tauri::{AppHandle, Manager};  // ← Manager 必须显式 import

let dir = app.path().app_local_data_dir()?;
```

### 坑 3: clippy 1.96 的 `unnecessary_to_owned` 新警告

旧 `lib.rs` 时代 `.to_vec()` 是 idiom, clippy 1.96 加了 `unnecessary_to_owned` lint,
发现 `&b"key".to_vec()` 跟 `&key.as_bytes().to_vec()` 都是冗余的, 直接传 slice 即可.

**教训**: copy-paste 旧代码后, 跑 `cargo clippy --lib` (不限 all-targets) 单独检查改过的文件,
发现就改. 老代码不动 (Stage 6a 留下来的 6 处 in `tests/stronghold_e2e.rs`, 不在 sub-task #2 范围).

### 坑 4: Step 2.1 + 2.3 顺序要反过来

我先写 `state/mod.rs` 加 `pub mod runtime;`, 然后写 `lib.rs` setup() 调 `state::runtime::build()`,
结果 Step 2.1 编译过不去 (找不到 `runtime` 文件).

**教训**: 写 mod tree 时, 先写最叶子 (`state/runtime.rs` 哪怕只是 stub), 再写中间 mod 文件,
最后写 `lib.rs`. 不能 lib.rs 先写完再补叶子, 编译一直在错状态.

我第二次修对: Step 2.1 一开始就把 `state/runtime.rs` 写成 `Ok(RuntimeState::new_for_test())` 的最小 stub,
让 lib.rs 编译过, 后续 Step 2.3 在那个 stub 上加真逻辑.

---

## 04b sub-task #2 验收清单

```markdown
- [x] cargo check (Tauri) 0 error
- [x] cargo clippy --lib 0 warning (refactor 范围内)
- [x] cargo test --workspace 248 passed, 0 failed (不回归)
- [x] lib.rs < 100 行 (实际 66 行)
- [x] 每个 .rs 文件 < 200 行 (最大 vault.rs 84 行)
- [x] commands/{health,stronghold,runtime} 4 domain 平行
- [x] events/ 单独目录, schema 收口
- [x] state/runtime.rs 注入到 Tauri State (stub 模式, 等 sub-task #3 替换)
- [x] Cargo.toml 加 qianxun-runtime + qianxun-memory path deps
- [x] bridge.ts (Svelte 端) 不用改 (sub-task #3 才改)
- [x] _shared-contract.md 不用改 (sub-task #3 才改)
```

---

## 不在 sub-task #2 范围 (留给后续)

- ❌ 5 个真 runtime command (list_sessions / send_message / create_plan / cancel_session / load_session) → sub-task #3
- ❌ Svelte 端 stores 改 invoke → sub-task #4-6
- ❌ delete_secret Rust 端实现 (TS 端有 mock, Rust 端没实现但 bridge.ts 走 isTauri 守卫 web fallback)
- ❌ _shared-contract.md 跟 Rust 端 types 同步更新 (4a 后续)
- ❌ runtime/ui/ 退役 (旧 SvelteKit, 跟 desktop 重叠, 后续清理)
- ❌ RuntimeApi trait 抽取 (daemon router + Tauri command 共用) → sub-task #3 顺手

---

## 4 条跨项目可复用教训

1. **Tauri 2.x command 注册路径**: 不要用 `pub use` 转发, lib.rs invoke_handler 用完整 `domain::action::fn_name` 路径
2. **Tauri 2.x 必备 import**: `app.path()` / `app.manage()` / `app.state()` 都来自 `tauri::Manager` trait, 必须显式 `use`
3. **写 mod tree 顺序**: 先叶子 stub → 再中间 mod → 最后根 mod (lib.rs), 不能反过来
4. **业务 1:1 搬 = 0 回归保证**: refactor 范围严格 1:1 复制业务逻辑, 加新功能留给后续 sub-task,
   这样 `cargo test --workspace` 不回归 = refactor 成功 (本次 248/248 ✓)

---

## 参考

- `docs/30_子项目规划/04b-tauri-runtime-integration.md` (上位规划, sub-task #1-7 排序)
- `docs/30_子项目规划/04c-qianxun-runtime-extraction.md` (sub-task #1 设计, 已完成)
- ADR-0003 (合并 desktop + 2-mode 互斥)
- `qianxun-desktop/src-tauri/src/lib.rs` (本 commit 改动的薄壳)
- `qianxun-desktop/src-tauri/src/commands/` (本 commit 新建的 4 domain)
- `qianxun-desktop/src-tauri/Cargo.toml` (本 commit 加的 2 个 path deps)
- 上次经验 `docs/40_经验/2026-06-08_qianxun-runtime-extraction.md`