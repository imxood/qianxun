//! Pattern dispatcher (v6 §3.5 模式 1-3 + Hybrid, MVP-2 plan 6 精简版)
//!
//! 4 个 pattern 决定 user input 走 chat 串行 / Kanban 单任务 / Kanban 多任务 / 双轨并行.
//! MVP-2 阶段: 决策函数 `decide_pattern` 已经能用, 真 dispatch 留 v2.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 4 个 pattern (v6 §3.5 完整列表).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DispatchPattern {
    /// 模式 1: chat 串行, 加到当前 session conversation 末尾
    Chat,
    /// 模式 2: Kanban 单任务, 跟当前 session 解耦
    SingleTask,
    /// 模式 3: Kanban 多任务, kanban_decompose 拆 N 个子任务
    MultiTask,
    /// 模式 4: Hybrid, 一边 chat 一边派 task (TUI 左右分屏)
    Hybrid,
}

impl DispatchPattern {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::SingleTask => "single_task",
            Self::MultiTask => "multi_task",
            Self::Hybrid => "hybrid",
        }
    }
}

/// Pattern 决策结果.
#[derive(Debug, Clone)]
pub struct PatternDecision {
    pub pattern: DispatchPattern,
    pub rationale: String,
    /// Kanban 模式用, 存 user_input + 可能的拆解结果
    pub task_payload: Option<Value>,
}

/// 决策 user input 走哪个 pattern.
///
/// 规则 (v6 §3.5):
/// - `/dispatch <text>` 强制模式 2 (SingleTask)
/// - `/decompose <text>` 强制模式 3 (MultiTask)
/// - 默认模式 1 (Chat)
/// - 未来可加: 长输入 + 多动词 -> Hybrid
pub fn decide_pattern(user_input: &str) -> PatternDecision {
    let trimmed = user_input.trim_start();
    if let Some(rest) = trimmed.strip_prefix("/dispatch ") {
        return PatternDecision {
            pattern: DispatchPattern::SingleTask,
            rationale: "explicit /dispatch command".into(),
            task_payload: Some(serde_json::json!({"user_input": rest})),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("/decompose ") {
        return PatternDecision {
            pattern: DispatchPattern::MultiTask,
            rationale: "explicit /decompose command".into(),
            task_payload: Some(serde_json::json!({"user_input": rest})),
        };
    }
    PatternDecision {
        pattern: DispatchPattern::Chat,
        rationale: "default chat mode".into(),
        task_payload: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decide_pattern_dispatch_explicit() {
        let d = decide_pattern("/dispatch 写个 e2e 测试");
        assert_eq!(d.pattern, DispatchPattern::SingleTask);
        assert!(d.rationale.contains("/dispatch"));
        assert!(d.task_payload.is_some());
    }

    #[test]
    fn test_decide_pattern_decompose_explicit() {
        let d = decide_pattern("/decompose 调研 daemon 升级");
        assert_eq!(d.pattern, DispatchPattern::MultiTask);
        assert!(d.rationale.contains("/decompose"));
    }

    #[test]
    fn test_decide_pattern_default_chat() {
        let d = decide_pattern("这段代码啥意思");
        assert_eq!(d.pattern, DispatchPattern::Chat);
        assert!(d.task_payload.is_none());
    }

    #[test]
    fn test_decide_pattern_chat_with_slash_word_no_command() {
        // "/dispatch" without space 后跟 text -> 不识别, 走 default
        let d = decide_pattern("/dispatching something");
        assert_eq!(d.pattern, DispatchPattern::Chat);
    }

    #[test]
    fn test_pattern_as_str() {
        assert_eq!(DispatchPattern::Chat.as_str(), "chat");
        assert_eq!(DispatchPattern::SingleTask.as_str(), "single_task");
        assert_eq!(DispatchPattern::MultiTask.as_str(), "multi_task");
        assert_eq!(DispatchPattern::Hybrid.as_str(), "hybrid");
    }

    #[test]
    fn test_pattern_serde_round_trip() {
        for p in [
            DispatchPattern::Chat,
            DispatchPattern::SingleTask,
            DispatchPattern::MultiTask,
            DispatchPattern::Hybrid,
        ] {
            let json = serde_json::to_string(&p).unwrap();
            let back: DispatchPattern = serde_json::from_str(&json).unwrap();
            assert_eq!(p, back);
        }
    }
}
