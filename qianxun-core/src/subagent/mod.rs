//! 缺口 03: SubAgent 工具白名单.
//!
//! ## 设计要点
//!
//! - **DEFAULT_SUBAGENT_TOOLS**: 默认只读工具白名单 (`read_file` / `glob` / `grep` / `bash_read_only`)
//! - **SubAgentContext**: 携带白名单 + 自定义拒因
//! - **check_tool_allowed**: 核心检查函数, 拒绝时返 `ToolDenied { tool, reason }`
//! - **白名单可定制**: 调用方可传 `&[&str]` 覆盖默认
//!
//! ## 不做什么
//!
//! - 不重做 SubAgent 调度逻辑 (本模块只提供白名单检查)
//! - 不做工具调用 hook (那是缺口 01 责任)
//! - 不做 per-tool 参数限制 (e.g. bash 只读 — 留给未来)
//!
//! ## 调用方
//!
//! - `qianxun-core/src/agent/subagent_dispatch` 在工具调用前调 `check_tool_allowed`
//! - `qianxun-runtime/src/api/sse_event.rs` 新增 `SseEvent::ToolDenied` 变体 (Stage 6 接入)

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

// ─── 默认白名单 ─────────────────────────────────────────────

/// SubAgent 默认可用工具白名单.
///
/// 只读工具集, 防止 subagent 误改文件 / 删库.
pub const DEFAULT_SUBAGENT_TOOLS: &[&str] = &[
    "read_file",
    "glob",
    "grep",
    "bash_read_only",
    "list_directory",
    "search_files",
];

// ─── ToolDenied 原因 ──────────────────────────────────────

/// 工具被拒绝的原因.
#[derive(Debug, Clone, Error, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ToolDenialReason {
    /// 工具不在白名单
    #[error("tool '{tool}' not in subagent whitelist")]
    NotInWhitelist { tool: String },

    /// 工具是写操作, subagent 默认禁止
    #[error("tool '{tool}' is a write operation, blocked by default")]
    WriteOperation { tool: String },

    /// 工具需要特殊权限
    #[error("tool '{tool}' requires elevated permissions")]
    RequiresElevation { tool: String },

    /// 父 agent 显式拒绝
    #[error("tool '{tool}' explicitly denied by parent agent")]
    ParentDenied { tool: String },
}

impl ToolDenialReason {
    pub fn tool_name(&self) -> &str {
        match self {
            Self::NotInWhitelist { tool } => tool,
            Self::WriteOperation { tool } => tool,
            Self::RequiresElevation { tool } => tool,
            Self::ParentDenied { tool } => tool,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::NotInWhitelist { .. } => "not_in_whitelist",
            Self::WriteOperation { .. } => "write_operation",
            Self::RequiresElevation { .. } => "requires_elevation",
            Self::ParentDenied { .. } => "parent_denied",
        }
    }
}

// ─── ToolDenied ───────────────────────────────────────────

/// 工具调用被拒绝 (子 agent 路径).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDenied {
    pub tool: String,
    pub reason: ToolDenialReason,
}

impl std::fmt::Display for ToolDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.reason)
    }
}

// ─── SubAgentContext ──────────────────────────────────────

/// SubAgent 上下文: 携带白名单配置.
#[derive(Debug, Clone)]
pub struct SubAgentContext {
    /// 允许的工具列表 (默认 = DEFAULT_SUBAGENT_TOOLS)
    pub allowed_tools: Vec<String>,
    /// SubAgent 标识
    pub subagent_id: String,
    /// 父 agent id
    pub parent_id: Option<String>,
    /// 是否允许写操作 (默认 false)
    pub allow_writes: bool,
}

impl SubAgentContext {
    /// 默认白名单 + 禁用写操作.
    pub fn new(subagent_id: impl Into<String>) -> Self {
        Self {
            allowed_tools: DEFAULT_SUBAGENT_TOOLS.iter().map(|s| s.to_string()).collect(),
            subagent_id: subagent_id.into(),
            parent_id: None,
            allow_writes: false,
        }
    }

    /// 自定义白名单构造.
    pub fn with_whitelist(
        subagent_id: impl Into<String>,
        whitelist: &[&str],
    ) -> Self {
        Self {
            allowed_tools: whitelist.iter().map(|s| s.to_string()).collect(),
            subagent_id: subagent_id.into(),
            parent_id: None,
            allow_writes: false,
        }
    }

    /// 自定义白名单 + 允许写.
    pub fn permissive(subagent_id: impl Into<String>, whitelist: &[&str]) -> Self {
        Self {
            allowed_tools: whitelist.iter().map(|s| s.to_string()).collect(),
            subagent_id: subagent_id.into(),
            parent_id: None,
            allow_writes: true,
        }
    }

    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        self.allowed_tools.iter().any(|t| t == tool_name)
    }
}

// ─── 核心检查函数 ──────────────────────────────────────────

/// 检查 subagent 是否可调用某工具.
///
/// 返 `Ok(())` 表示允许, `Err(ToolDenied)` 表示拒绝.
pub fn check_tool_allowed(
    ctx: &SubAgentContext,
    tool_name: &str,
) -> Result<(), ToolDenied> {
    // 1. 白名单检查
    if !ctx.is_tool_allowed(tool_name) {
        return Err(ToolDenied {
            tool: tool_name.to_string(),
            reason: ToolDenialReason::NotInWhitelist {
                tool: tool_name.to_string(),
            },
        });
    }

    // 2. 写操作检查 (默认禁止, 除非 allow_writes=true)
    if !ctx.allow_writes && is_write_operation(tool_name) {
        return Err(ToolDenied {
            tool: tool_name.to_string(),
            reason: ToolDenialReason::WriteOperation {
                tool: tool_name.to_string(),
            },
        });
    }

    Ok(())
}

/// 判断工具是否为写操作.
fn is_write_operation(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "write_file" | "edit_file" | "delete_file" | "bash" | "bash_write"
    )
}

// ─── SubAgentToolGuard (供 HookRegistry 集成用) ────────────

/// Hook 钩子: 包装白名单检查, 供 `HookRegistry` 在 BeforeToolCall 时调.
pub struct SubAgentToolGuard {
    pub ctx: Arc<SubAgentContext>,
}

impl SubAgentToolGuard {
    pub fn new(ctx: Arc<SubAgentContext>) -> Self {
        Self { ctx }
    }
}

// ─── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── DEFAULT_SUBAGENT_TOOLS ──

    #[test]
    fn test_default_whitelist_contains_readonly_tools() {
        assert!(DEFAULT_SUBAGENT_TOOLS.contains(&"read_file"));
        assert!(DEFAULT_SUBAGENT_TOOLS.contains(&"glob"));
        assert!(DEFAULT_SUBAGENT_TOOLS.contains(&"grep"));
        assert!(!DEFAULT_SUBAGENT_TOOLS.contains(&"write_file"));
        assert!(!DEFAULT_SUBAGENT_TOOLS.contains(&"bash"));
    }

    // ── SubAgentContext ──

    #[test]
    fn test_new_uses_default_whitelist_and_no_writes() {
        let ctx = SubAgentContext::new("sa1");
        assert!(ctx.is_tool_allowed("read_file"));
        assert!(!ctx.is_tool_allowed("write_file"));
        assert!(!ctx.allow_writes);
    }

    #[test]
    fn test_with_whitelist_uses_custom_tools() {
        let ctx = SubAgentContext::with_whitelist("sa1", &["read_file", "custom_tool"]);
        assert!(ctx.is_tool_allowed("custom_tool"));
        assert!(!ctx.is_tool_allowed("glob"));
    }

    #[test]
    fn test_permissive_allows_writes() {
        let ctx = SubAgentContext::permissive("sa1", &["write_file"]);
        assert!(ctx.allow_writes);
    }

    // ── check_tool_allowed ──

    #[test]
    fn test_check_allows_whitelisted_readonly_tool() {
        let ctx = SubAgentContext::new("sa1");
        assert!(check_tool_allowed(&ctx, "read_file").is_ok());
        assert!(check_tool_allowed(&ctx, "glob").is_ok());
    }

    #[test]
    fn test_check_denies_write_tool_by_default() {
        let ctx = SubAgentContext::new("sa1");
        let r = check_tool_allowed(&ctx, "write_file");
        assert!(r.is_err());
        let denied = r.unwrap_err();
        assert_eq!(denied.tool, "write_file");
        // write_file 不在默认白名单 → NotInWhitelist (而非 WriteOperation)
        assert!(matches!(denied.reason, ToolDenialReason::NotInWhitelist { .. }));
    }

    #[test]
    fn test_check_denies_write_tool_when_in_whitelist_but_allow_writes_false() {
        // 显式把 write_file 加进白名单, 但 allow_writes=false → WriteOperation
        let ctx = SubAgentContext::with_whitelist("sa1", &["write_file"]);
        let r = check_tool_allowed(&ctx, "write_file");
        assert!(r.is_err());
        let denied = r.unwrap_err();
        assert!(matches!(denied.reason, ToolDenialReason::WriteOperation { .. }));
    }

    #[test]
    fn test_check_denies_tool_not_in_whitelist() {
        let ctx = SubAgentContext::new("sa1");
        let r = check_tool_allowed(&ctx, "some_unknown_tool");
        assert!(r.is_err());
        let denied = r.unwrap_err();
        assert!(matches!(denied.reason, ToolDenialReason::NotInWhitelist { .. }));
    }

    #[test]
    fn test_check_allows_write_tool_when_allow_writes() {
        let ctx = SubAgentContext::permissive("sa1", &["write_file"]);
        assert!(check_tool_allowed(&ctx, "write_file").is_ok());
    }

    #[test]
    fn test_check_denies_bash_by_default() {
        let ctx = SubAgentContext::new("sa1");
        let r = check_tool_allowed(&ctx, "bash");
        assert!(r.is_err());
        // bash 不在白名单 (只有 bash_read_only 在), 应该是 NotInWhitelist
        assert!(matches!(r.unwrap_err().reason, ToolDenialReason::NotInWhitelist { .. }));
    }

    // ── ToolDenialReason ──

    #[test]
    fn test_denial_reason_tool_name_extraction() {
        let r = ToolDenialReason::NotInWhitelist {
            tool: "evil_tool".to_string(),
        };
        assert_eq!(r.tool_name(), "evil_tool");
        assert_eq!(r.code(), "not_in_whitelist");
    }

    #[test]
    fn test_denial_reason_codes() {
        assert_eq!(ToolDenialReason::WriteOperation { tool: "x".into() }.code(), "write_operation");
        assert_eq!(ToolDenialReason::RequiresElevation { tool: "x".into() }.code(), "requires_elevation");
        assert_eq!(ToolDenialReason::ParentDenied { tool: "x".into() }.code(), "parent_denied");
    }
}
