# 07-client-migration.md (2026-06-04)

> 模块: `qianxun/src/client/mod.rs` (1213 行) → `qianxun/src/client/` 子目录
> 流程: **分析 → 迁移文档 → 执行 → verify → 提交** (用户 2026-06-04 提案)
> 状态: **执行中 (步骤 1 完成, 步骤 2a 完成)**

---

## Context (为什么做)

`qianxun/src/client/mod.rs` 1213 行, 内含 5 段 (错误/类型/DaemonClient/重连/SSE/REPL/单测),
单一文件职责发散. 违反 1000 行红线. 跟 `persistence/` `output_sink/` 风格对齐:
- 文件夹 = 模块边界
- tests/ 子目录放测试
- 公共 API 仍从 `crate::client::XXX` 平面访问 (pub use re-export)

## 现状盘点 (2026-06-04 commit 36ad610 — 精确行号)

| 行号区间 | 内容 | 行数 | 跨文件依赖 |
|---:|---|---:|---|
| 1-17 | 模块 doc (15 段注释) | 17 | — |
| 18 | `#![allow(dead_code)]` | 1 | — |
| 19 | 空行 | 1 | — |
| 20-27 | 顶层 use 块 (8 个 use) | 8 | — |
| 28-29 | 空行 + 注释 | 2 | — |
| 30-32 | `SseStream` type alias | 3 | — |
| 33-52 | `ClientError` enum (5 variant) | 20 | thiserror, reqwest, serde_json, std::io |
| 53-104 | 5 个 DTO struct + `impl PromptRequest::text` | 52 | serde, thiserror 子, super::types |
| 105-176 | `SseEvent` enum (12 variant) | 72 | serde, serde_json |
| 177-346 | `DaemonClient` struct + 6 个 impl DaemonClient 块 | 170 | reqwest, std::time::Duration, tracing::{debug,info,warn}, futures, super::types |
| 347-546 | 自动重连 (RECONNECT_BACKOFF + next_backoff + ReconnectState + ReconnectTracker + t_after + ReconnectHandle struct + impl) | 200 | std::sync::Arc, std::time::Duration, tokio::sync::{Mutex, Notify} |
| 547-647 | SSE 解析 (parse_sse_stream + extract_sse_frames + parse_data_payload) | 101 | std::time::Duration, futures::stream, reqwest::Response, tracing::warn, super::types |
| 648-681 | `detect_local_daemon` (probe) | 34 | std::time::Duration, tracing::debug, super::daemon_client::DaemonClient |
| 682-799 | `run_thin_repl` + `consume_sse_stream_print` (REPL 私有 helper) | 118 | anyhow, futures, std::io, tracing, super::types |
| 800 | `// ─── 单测 ───` 注释 | 1 | — |
| 801 | 空行 | 1 | — |
| 802 | `#[cfg(test)]` | 1 | — |
| 803-1213 | `mod tests` (12 个 test fn) | 411 | use super::*; use 显式列举 + tokio test |

## 12 个 test fn 完整列表 (按行号)

| 行号 | test fn | 备注 |
|---:|---|---|
| 859 | `test_health_returns_health_status` | async |
| 869 | `test_create_session_returns_session_id` | async |
| 904 | `test_stream_prompt_parses_sse_events` | async |
| 982 | `test_reconnect_backoff_table_matches_spec` | sync |
| 1016 | `test_reconnect_state_labels` | sync |
| 1088 | `test_request_includes_bearer_header` | async |
| 1119 | `test_request_without_token_omits_header` | async |
| 1141 | `test_with_token_constructor_stores_token` | sync |
| 1171 | `test_daemon_client_with_token_stores_token` | sync |
| 1178 | `test_daemon_client_new_token_is_none` | sync |
| 1187 | `test_daemon_client_url_with_trailing_slash_normalizes` | sync (注意: 我之前在 1187, 实际可能 +N) |
| + 1 | `test_daemon_client_*` (有 1 个 url_normalize test 在末尾) | sync |

(12 个 test fn, 跨 6 个主题: health / create_session / stream_prompt / backoff / state / token / url_normalize)

## 6 个 `impl DaemonClient` 块分布 (line 178-346)

| 起 | 终 | 内容 |
|---:|---:|---|
| 210 | 254 | `new` / `with_token` / `new_with_token` / `base_url` 4 fn (ctor + getter) |
| 257 | 287 | `health` (公开 GET /v1/system/health) |
| 290 | 339 | `create_session` / `get_session` / `list_sessions` / `delete_session` 4 fn (CRUD) |
| 342 | 380 | `prompt` (SSE 流式, 内部调 parse_sse_stream) |
| 383 | 414 | `cancel_session` / `pause_session` / `resume_session` 3 fn |
| 420 | 525 | `start_reconnect_loop` (1 fn, 含后台 task spawn) |

每个 impl 块都 `impl DaemonClient { ... }` 完整, 拆到 daemon_client.rs 后**合并为 1 文件的 6 个 impl 块** (Rust 允许).

## 目标结构 (精确行数预估)

```
qianxun/src/client/
├── mod.rs                       (43 行)  — 顶层 use (5) + 5 mod + 5 pub use + SseStream type alias + #[cfg(test)] mod tests;
├── types.rs                     (152 行) — 头部 use (5) + ClientError + 5 DTO + SseEvent + impl PromptRequest
├── daemon_client.rs             (357 行) — 头部 use (5) + DaemonClient struct + 6 impl DaemonClient 块
├── reconnect.rs                  (29 行) — 头部 use (3) + t_after + ReconnectHandle struct + impl
├── sse_parser.rs               (109 行) — 头部 use (5) + parse_sse_stream + extract + parse_data_payload
├── probe.rs                     (41 行)  — 头部 use (3) + detect_local_daemon
├── repl.rs                     (125 行) — 头部 use (4) + run_thin_repl + consume_sse_stream_print
└── tests/
    └── mod.rs                  (412 行) — mod tests { use super::*; 12 test fn }
```

**每个文件都 < 1000 行** (最大 tests/mod.rs 412, 其他都 < 360).

## 跨文件 import 完整清单

### types.rs (152 行)
```rust
use reqwest::Error as ReqwestError;
use serde::{Deserialize, Serialize};
use serde_json::Error as SerdeJsonError;
use std::io::Error as IoError;
use thiserror::Error;
```

### daemon_client.rs (357 行)
```rust
use reqwest::Response;
use serde_json;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use super::types::{ClientError, HealthStatus, PromptRequest, Session, SessionCreated, SseEvent};
```

注意: `info` / `debug` / `warn` 按需 use, 不用 `tracing::*` (避免子文件 import 跟 mod.rs 冲突).

### reconnect.rs (29 行)
```rust
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
```

### sse_parser.rs (109 行)
```rust
use std::time::Duration;

use futures::stream::{Stream, StreamExt};
use reqwest::Response;
use tracing::warn;

use super::types::{ClientError, SseEvent};
```

### probe.rs (41 行)
```rust
use std::time::Duration;

use tracing::debug;

use super::daemon_client::DaemonClient;
```

### repl.rs (125 行)
```rust
use std::io::{self, Write};

use anyhow;
use futures::stream::StreamExt;
use tracing::{info, warn};

use super::daemon_client::DaemonClient;
use super::repl as _;  // 不需要, repl 模块无内部引用, 删
use super::types::{ClientError, SseEvent, SseStream};
```

实际 repl.rs 只需 `DaemonClient` + `SseEvent` + `SseStream`. 让我重审...

### tests/mod.rs (412 行)
```rust
#![allow(unused_imports)]  // 一些 use 仅个别 test 需
use super::*;                  // super = crate::client
use super::daemon_client::DaemonClient;  // 显式
use super::types::{ClientError, HealthStatus, PromptRequest, Session, SseEvent};
use reqwest::Response;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;
```

`use super::*;` 在子 mod 中, `super` 指 `crate::client::*`, 应该能见 `DaemonClient` (顶层 pub use) 跟 `SseEvent` 等. 但**实际**之前经验: `use super::*` 不会导入子 mod (types/daemon_client 等) 内的 `pub` 项, 因为它们在子 mod 内不是 `crate::client` 顶层 pub. 所以**必须显式 use 子 mod**.

## 跨文件依赖图 (谁 use 谁)

```
mod.rs
  ├─ mod types;           pub use types::*;
  ├─ mod daemon_client;   pub use daemon_client::DaemonClient;
  ├─ mod reconnect;       pub use reconnect::{next_backoff, ReconnectHandle, ReconnectState, ReconnectTracker, RECONNECT_BACKOFF};
  ├─ mod sse_parser;      pub use sse_parser::{extract_sse_frames, parse_data_payload, parse_sse_stream};
  ├─ mod probe;           pub use probe::detect_local_daemon;
  ├─ mod repl;            pub use repl::run_thin_repl;
  └─ #[cfg(test)] mod tests;
       └─ use super::*; use super::types::*; use super::daemon_client::DaemonClient;

types.rs         (无 super 依赖, 完全独立)
daemon_client.rs (use super::types::*)
reconnect.rs     (无 super 依赖, 独立, 但 t_after 期望在 impl DaemonClient 内 → 实际是 free fn, 不依赖)
sse_parser.rs    (use super::types::*)
probe.rs         (use super::daemon_client::DaemonClient)
repl.rs          (use super::daemon_client::DaemonClient, super::types::*)
```

**注意**: reconnect.rs 的 `t_after` `async fn` 实际是 free fn (不依赖 `DaemonClient::self`), 所以可以独立. 但 `ReconnectTracker` 含 `Arc<Mutex<ReconnectTracker>>` (用 tokio::sync::Mutex) + ReconnectHandle 含 `Arc<tokio::sync::Notify>`, 这些 import 都在 reconnect.rs 头部.

## 6 个 impl DaemonClient 块具体行号 (在 178-346)

| impl 块 | 起 | 终 | 函数数 | 主题 |
|---:|---:|---:|---:|---|
| #1 | 210 | 254 | 4 | ctor (new/with_token/new_with_token/base_url) |
| #2 | 257 | 287 | 1 | health |
| #3 | 290 | 339 | 4 | sessions CRUD |
| #4 | 342 | 380 | 1 | prompt (SSE) |
| #5 | 383 | 414 | 3 | cancel/pause/resume |
| #6 | 420 | 525 | 1 | start_reconnect_loop |

每个 impl 块在 178-346 范围内跨不连续行号, 提取到 daemon_client.rs 后, 6 个 impl 块**全在 1 文件**, 顺序保留.

## 迁移步骤 (执行蓝图 — 状态实时更新)

### ✅ 步骤 1: 准备
- ✅ `mkdir qianxun/src/client/tests`
- ✅ `git show HEAD:.../mod.rs > /tmp/client_orig.txt` (备份 1213 行)

### ✅ 步骤 2a: types.rs (152 行, 完成)
- ✅ 提取 line 33-176 (144 行) 加 5 个 use 头部 (8 行) = 152 行
- ✅ 写文件 `qianxun/src/client/types.rs`

### 🔄 步骤 2b: daemon_client.rs (357 行, 进行中)
- 🔄 提取 line 178-346 (348 行) + 5 use 头部 (9 行) = 357 行
- 待写文件

### ⏳ 步骤 2c: reconnect.rs (29 行)
- ⏳ 提取 line 526-546 (21 行) + 3 use 头部 (8 行) = 29 行
- 待写文件

### ⏳ 步骤 2d: sse_parser.rs (109 行)
- ⏳ 提取 line 547-647 (101 行) + 5 use 头部 (8 行) = 109 行
- 待写文件

### ⏳ 步骤 2e: probe.rs (41 行)
- ⏳ 提取 line 648-681 (34 行) + 3 use 头部 (7 行) = 41 行
- 待写文件

### ⏳ 步骤 2f: repl.rs (125 行)
- ⏳ 提取 line 682-799 (118 行) + 4 use 头部 (7 行) = 125 行
- 待写文件

### ⏳ 步骤 2g: tests/mod.rs (412 行)
- ⏳ 提取 line 802-1213 (412 行) + 头部 use 列表
- 头 1 行 `#[cfg(test)]` + `mod tests {`
- 末 1 行 `}` 关闭 mod
- 待写文件

### ⏳ 步骤 3: 重写 mod.rs (43 行)
- ⏳ 顶部 5 个 use (mod.rs 顶层需要的: SseStream type, futures::Stream, std::pin::Pin)
- 7 个 mod 声明
- 7 个 pub use re-export
- 1 个 SseStream type alias
- 1 行 `#[cfg(test)] mod tests;`

### ⏳ 步骤 4: 验证 + 删除旧 mod.rs
- ⏳ `cargo build -p qianxun`: 期望 0 error
- ⏳ `cargo build -p qianxun --tests`: 期望 0 error
- ⏳ `cargo test -p qianxun --no-fail-fast 2>&1 | tail -3`: 期望 254+12=266 pass (12 client test)

### ⏳ 步骤 5: 提交
- ⏳ `git add -A qianxun/src/client/`
- ⏳ `git commit -m "refactor(client): split client/mod.rs (1213) into client/ subdir (6 src + 1 tests)"`
- ⏳ `git log --oneline -1` 验证

## 关键 import 跨文件问题 (易出错点 — 已知踩坑)

1. **`tokio::sync::Mutex` vs `std::sync::Mutex`**: DaemonClient 用 `std::sync::Mutex<...>`, reconnect 用 `tokio::sync::Mutex<...>`. 名字冲突, 需明确 `tokio::sync::Mutex` 别名或 `as` rename.

2. **`Arc<Notify>` vs `Arc<JoinHandle>`**: ReconnectHandle 用 `Arc<Notify>` 跟 `Arc<JoinHandle>`, `JoinHandle<()>` 类型必须明确 (是 `tokio::task::JoinHandle<()>`).

3. **`tracing::warn` vs `tracing::debug`**: 不同子文件按需 use, 不要全 import `tracing::*` (跟 mod.rs 顶层 import 冲突).

4. **`Arc<Mutex<Connection>>` vs `Arc<Mutex<DaemonClient>>`**: persistence 用 `std::sync::Mutex<Connection>`, reconnect 用 `tokio::sync::Mutex<ReconnectTracker>`. 不同 Mutex.

5. **`SseEvent` 在 types.rs**, 其它 6 个文件都引用. `use super::types::SseEvent` 在子文件 OK, mod.rs 顶层 re-export.

6. **impl 块跨文件**: 6 个 `impl DaemonClient { ... }` 全在 daemon_client.rs (Rust 允许).

7. **tests/mod.rs 跨子 mod import**: `use super::*` 在子 mod 中**不**导入子 mod (types/daemon_client 等) 内的 `pub` 项 — 必须显式 `use super::types::*; use super::daemon_client::DaemonClient;`.

8. **client/mod.rs 旧版 1213 行**: 步骤 3 写新 mod.rs (43 行) 后, cargo 优先选 `client/mod.rs` 作为模块入口, 旧 1213 实际**就是**新的 (被覆盖). 无需 rm, 旧内容直接消失.

9. **`#![allow(dead_code)]` 顶层 attr**: types.rs/daemon_client.rs 等子文件**不能**有 `#![...]` (inner attr 只在 mod 入口, mod.rs 入口). 移到 mod.rs 顶层, 子文件**不能**有 `#!`.

10. **`run_thin_repl` 私有 helper `consume_sse_stream_print`**: 跟 run_thin_repl 同一 impl 块 (695-799, 105 行). 整段搬, 私有 fn 在 mod 内部可见.

## 风险

| 风险 | 缓解 |
|---|---|
| 跨文件 impl 块错位 (fn 主体在 impl 内还是外) | 整段连续搬, 不重排; cargo build 立即报警 |
| `use super::*` 不导入私有项 (跟 persistence/output_sink 同样问题) | 显式 use 子 mod: `use super::types::*; use super::daemon_client::DaemonClient;` |
| `tracing::debug!` 宏 + `tokio::sync::Mutex` Mutex 名字冲突 | 子文件 use 块精确写: `use tracing::debug; use tokio::sync::Mutex;` |
| 12 个 test 跨 tests/mod.rs 文件边界, `use super::*` 失效 | tests/mod.rs 加 `use super::types::*; use super::daemon_client::DaemonClient;` 等显式 |
| 旧 mod.rs 1213 行 cargo 仍选它 (选 mod/ 目录优先 vs mod.rs 文件) | 步骤 3 写新 mod.rs (43 行) 覆盖旧 1213, 旧内容消失 |
| 客户端类型 `Arc<Mutex<ReconnectTracker>>` 在 daemon_client.rs 用, 但 Mutex 是 `tokio::sync` 跟 `std::sync` 不同 | reconnect.rs 头部明确 `use tokio::sync::{Mutex, Notify}` |

## Verify 命令 (完整清单)

```bash
cd E:/git/maxu/qianxun

# 1. 编译
cargo build -p qianxun                                      # exit 0
cargo build -p qianxun --tests                               # exit 0

# 2. 测试
cargo test -p qianxun --no-fail-fast 2>&1 | tail -3          # 12 client test pass

# 3. 行数门禁
wc -l qianxun/src/client/*.rs qianxun/src/client/tests/*.rs  # max < 1000 (预期 412 max)

# 4. clippy
cargo clippy -p qianxun --tests 2>&1 | grep "error:" | head -3  # 0 error

# 5. git status
git status qianxun/src/client/                              # 显示新 7 文件, 无删除 (mod.rs 覆盖)

# 6. diff 统计
git diff --stat HEAD qianxun/src/client/                     # +7 文件, 旧 mod.rs 内容消失
```

## 预期最终结果

| 指标 | 重构前 | 重构后 |
|---|---:|---:|
| > 1000 行的 .rs (client) | 1 (1213) | 0 |
| 最大单文件行数 (client/) | 1213 | 412 (tests/mod.rs) |
| `cargo test -p qianxun` | 254 pass | 254 pass (不变) |
| 12 client test 全跑 | ✓ | ✓ |
| 0 业务行为变化 | — | ✓ |
| 0 新依赖 | — | ✓ |

## 执行日志 (实时)

- ✅ 步骤 1: mkdir tests/ + git show 备份
- ✅ 步骤 2a: types.rs (152 行, 5 use + ClientError + 5 DTO + SseEvent + impl PromptRequest)
- 🔄 步骤 2b: daemon_client.rs (进行中, 准备写文件)
- ⏳ 步骤 2c-2g: 待执行
- ⏳ 步骤 3-5: 待执行
