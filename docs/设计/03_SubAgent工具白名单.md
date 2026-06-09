# 缺口 03: SubAgent 工具白名单

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 借鉴源

[microclaw](E:\git\ai\microclaw) sub-agent 9-tool 白名单: 防递归 / 跨副作用 / 主 memory 写。

## 问题

千寻 v2 `SubAgentSpec.tool_filter: Option<ToolCategoryFilter>`, 粒度只到 category (Fs/Network/MCP):

```rust
pub enum ToolCategoryFilter {
    ReadOnly,
    All,
    // 太粗: sub-agent 可写主 conversation / 调 send_message / 递归 fork
}
```

sub-agent 实际可:
- 调 `send_message` 把噪声发到主 session
- 调 `write_memory` 污染主 memory
- 调 `schedule` 后台任务死循环
- 递归 `fork_subagent` 资源耗尽

## 方案

### 3.1 精确工具名单 (替代 category filter)

```rust
// qianxun-core/src/subagent/mod.rs

pub struct SubAgentSpec {
    pub id: SubAgentId,
    pub parent_session_id: SessionId,
    pub task: String,
    /// 精确工具名单, None = 默认白名单 (见下)
    pub tool_filter: Option<Vec<String>>,
    pub max_turns: u32,
    pub budget_tokens: Option<u32>,
}

/// 内置默认白名单 (microclaw 9-tool 同源)
pub const DEFAULT_SUBAGENT_TOOLS: &[&str] = &[
    // 读类 (5)
    "read_file", "grep", "glob", "list_dir", "web_search",
    // 反馈 (1)
    "ask_user",
    // 终结 (1)
    "task_complete",
    // 元 (2)
    "fetch_url", "git_status",
    // 严禁: write_file / edit_file / execute_command / delete_file /
    //        send_message / write_memory / schedule / fork_subagent /
    //        update_session_mode / respond_approval
];
```

### 3.2 工具调用前的白名单校验

```rust
// qianxun-core/src/processing_loop/v2.rs

fn execute_tool(&self, name: &str, args: Value) -> Result<Value, ToolError> {
    let session = self.session.read();
    if let SessionMode::Sub = session.mode {
        let allowed = session.spec.tool_filter
            .as_ref()
            .map(|v| v.iter().any(|n| n == name))
            .unwrap_or_else(|| DEFAULT_SUBAGENT_TOOLS.contains(&name));
        if !allowed {
            return Err(ToolError::Denied {
                tool: name.to_string(),
                reason: "sub-agent tool not in whitelist".into(),
            });
        }
    }
    // ... 实际调 ...
}
```

### 3.3 SseEvent 加越权告警

```rust
// qianxun-runtime/src/sse.rs

ToolDenied { session_id, tool_name, reason },
```

主 session 收到 ToolDenied, UI 可显示"sub-agent 越权, 已拒绝"。

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-core/src/subagent/mod.rs` | tool_filter 改 Vec<String> + 默认白名单常量 | +20 |
| `qianxun-core/src/processing_loop/v2.rs` | 白名单校验 | +15 |
| `qianxun-runtime/src/sse.rs` | +1 SseEvent 变体 | +5 |
| 测试 | 12-tool 拒绝 + 9-tool 允许 | +30 |

**总计 ~70 行**

## 不做什么

- 不做动态白名单 (sub-agent 运行时扩展工具) — 简单为先
- 不做 per-session 白名单覆盖 — 全局默认 + spec override 够了
- 不做"越权但 ask_user 弹窗" — 直接拒绝, 简单清晰

## 验收

- [ ] sub-agent 调 write_file → 拒绝 + SseEvent::ToolDenied
- [ ] sub-agent 调 read_file → 允许
- [ ] sub-agent 调 fork_subagent → 拒绝
- [ ] 显式 `tool_filter: ["read_file", "write_file"]` → 允许 write_file
- [ ] 显式 `tool_filter: ["nonexistent"]` → 调任何工具都拒绝
