# 08-tui-migration.md (2026-06-04)

> 模块: `qianxun/src/tui/mod.rs` (1717 行) → `qianxun/src/tui/` 子目录
> 流程: **分析 → 迁移文档 → 执行 → verify → 提交** (沿 07-client-migration 风格)
> 状态: 待执行

---

## Context (为什么做)

`qianxun/src/tui/mod.rs` 1717 行, 单一文件含 const + 28 个 fn/struct/impl + 1 个 mod tests.
违反 1000 行红线. 跟 `client/` `persistence/` `output_sink/` 风格对齐:
- 文件夹 = 模块边界
- tests/ 子目录放测试
- 公共 API 仍从 `crate::tui::XXX` 平面访问 (pub use re-export)

## 现状盘点 (2026-06-04 commit e9b753a — 精确行号)

| 行号区间 | 内容 | 行数 |
|---:|---|---:|
| 1-36 | 模块 doc + 5 use + 5 const (MAX_MESSAGES / PALETTE_MAX_ROWS / INLINE_VIEWPORT_HEIGHT) | 36 |
| 37-100 | DIRTY_* 4 个 const + COMMANDS 数组 | 64 |
| 101-105 | `struct CommandSpec` | 5 |
| 106-119 | `pub async fn run` (TUI 入口) | 14 |
| 120-161 | `struct App` + 字段 | 42 |
| 162-1145 | `impl App` (大块, 几乎所有 App 方法) | 984 |
| 1146-1199 | `message_lines_for` + `render_one_message` (render helper) | 54 |
| 1200-1211 | `write_full_output` | 12 |
| 1212-1236 | `CommandPalette` struct + impl | 25 |
| 1237-1261 | `Modal` struct + impl + `ModalKind` enum | 25 |
| 1262-1276 | `UiRole` enum | 15 |
| 1277-1316 | `UiMessage` struct + impl | 40 |
| 1317-1336 | `AgentEvent` enum | 20 |
| 1337-1384 | `TuiOutputSink` struct + impl OutputSink | 48 |
| 1385-1395 | `render_modal` | 11 |
| 1396-1415 | `centered_rect` | 20 |
| 1416-1429 | `filtered_commands` | 14 |
| 1430-1438 | `unique_command_match` | 9 |
| 1439-1445 | `mode_desc` | 7 |
| 1446-1504 | `new_agent_loop` + `connect_workspace_mcp` + `format_json_compact` + `truncate_chars` | 59 |
| 1505-1524 | 末尾空行 | 20 |
| 1525-1717 | `mod tests` (~190 行) | 193 |

## 目标结构

```
qianxun/src/tui/
├── mod.rs                       (~60 行)  — 顶层 use + 7 mod + 7 pub use + re-export
├── app.rs                       (~990 行) — App struct + 1000 行的 impl App
├── render.rs                    (~120 行) — message_lines_for + render_one_message + write_full_output + render_modal
├── layout.rs                    (~30 行)  — centered_rect
├── command_palette.rs           (~80 行)  — CommandSpec + COMMANDS + CommandPalette struct + impl + filtered_commands + unique_command_match
├── modal.rs                     (~60 行)  — Modal + ModalKind + UiRole
├── messages.rs                  (~60 行)  — UiMessage + AgentEvent
├── streaming.rs                 (~60 行)  — TuiOutputSink
├── helpers.rs                   (~80 行)  — mode_desc + new_agent_loop + connect_workspace_mcp + format_json_compact + truncate_chars + DIRTY_* const
├── run.rs                       (~30 行)  — pub async fn run (TUI 入口)
└── tests/
    └── mod.rs                  (~200 行) — 7 个 test fn
```

每个文件 < 1000 行 (最大 app.rs ~990, 接近红线但**实际**可以拆 impl App 多块, 后续 v2 再细分).

## 跨文件依赖 (impl 块, use 块)

### app.rs (含 App struct + impl)
- 引用: messages (UiMessage, AgentEvent), streaming (TuiOutputSink), modal (Modal, UiRole), command_palette (CommandSpec, CommandPalette), render (render_messages 调), run (run fn 不在 app.rs)
- impl App 调: 所有 fn 几乎都调 render/modal/streaming/command_palette

### render.rs
- 调: messages (UiMessage)
- 被: app.rs, command_palette.rs (光标), modal.rs (渲染)

### streaming.rs
- 调: messages (AgentEvent)
- 被: app.rs (输出推送)

### command_palette.rs
- 调: messages (UiMessage for ctor)
- 被: app.rs (命令注册)

### modal.rs
- 调: messages (UiMessage)
- 被: app.rs (handle_modal_key)

### run.rs
- 调: app.rs (App struct, new, run)
- 被: 外部 (main.rs 或 binary 入口)

### helpers.rs
- 调: ratatui, tui_input, agent_loop, registry
- 被: app.rs (各 fn)

## 迁移步骤 (执行蓝图)

### 步骤 1: 准备
- `mkdir -p qianxun/src/tui/tests`
- `git show HEAD:qianxun/src/tui/mod.rs > /tmp/tui_orig.txt`

### 步骤 2: 创建 9 个子文件 (顺序: types → app → render → ...)
- 每次 Write 一个 .rs, 内容从 /tmp/tui_orig.txt 取的行号区间
- 头部补 use 块

### 步骤 3: 重写 mod.rs (~60 行)
- 顶层 use + 7 mod 声明 + 7 pub use re-export

### 步骤 4: 验证
- `cargo build -p qianxun`: 0 error
- `cargo test -p qianxun tui`: 7 test pass
- `cargo test -p qianxun --no-fail-fast`: 254 pass + 1 pre-existing fail

### 步骤 5: 提交
- `git add -A qianxun/src/tui/`
- `git commit -m "refactor(tui): split tui/mod.rs (1717) into tui/ subdir (8 src + 1 tests)"`

## 关键 import 易错点

1. **impl App 跨多文件不分割**: `impl App { ... }` (984 行) 全部在 app.rs (含所有 100+ fn). 不拆, 避免大量 `impl App { ... } ... impl App { ... }` 跨文件分散.
2. **DIRTY_* const**: 放 helpers.rs 顶部, 公开 (`pub const`) 让 app.rs 引用. 但实际原版是 const (私有 impl block 访问), 拆 file 后需改 pub(crate) 让 impl 块能见.
3. **pub async fn run (106-119)**: 14 行入口, 放独立 run.rs (跟 client 的 run_thin_repl 同 pattern).
4. **嵌套 free fn 调 super 关系**: `render_modal` 调 ratatui (不依赖 tui types). `centered_rect` 也独立. `filtered_commands` + `unique_command_match` 用 COMMANDS 数组 (放 command_palette.rs).
5. **mod tests 12 个 test fn**: tests/mod.rs `use super::*;` 失效 (跟 client 同样问题), 需显式 `use crate::tui::app::App; use crate::tui::messages::UiMessage;` 等.
6. **CommandSpec, UiRole, ModalKind, AgentEvent** 全部 `pub enum/struct` (原版就 pub), 无需改可见性.

## 风险

| 风险 | 缓解 |
|---|---|
| impl App 跨 1000 行 | app.rs 仍 < 1000 (984 行, 含 + use 块约 990) — 接近但**不超** |
| `use super::*;` 失效 (跟 client 同样) | tests/mod.rs 显式 use 子 mod |
| run fn 内部 14 行 (很小) 独立文件 | 14 行值得独立, 跟 client run_thin_repl 一致 |
| 5 个 const (DIRTY_*) 私有 → pub(crate) | helpers.rs 顶部 pub(crate) 暴露 |
| 删除原 mod.rs 1213 行时 cargo 优先选 mod/ 目录 | 步骤 3 写新 mod.rs (43 行) 覆盖旧 1213 |

## Verify

```bash
cd E:/git/maxu/qianxun
cargo build -p qianxun                                      # exit 0
cargo test -p qianxun tui 2>&1 | tail -3                      # 7 test pass
cargo test -p qianxun --no-fail-fast 2>&1 | tail -3          # 254 pass + 1 pre-existing fail
wc -l qianxun/src/tui/*.rs qianxun/src/tui/tests/*.rs         # max < 1000
```

## 执行日志

- [ ] 步骤 1: mkdir + 备份
- [ ] 步骤 2a-2h: 8 个子文件
- [ ] 步骤 3: mod.rs 重写
- [ ] 步骤 4: 验证
- [ ] 步骤 5: 提交
