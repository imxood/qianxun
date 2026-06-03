//! Team / Multi-agent 配置 (v6 §6.4 + §8.1)
//!
//! 3 个核心 struct: Profile / Role / TeamConfig + TeamRegistry.
//! 4 个默认 role + profile 硬编码 (MVP-2 plan 4, v1).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::tools::ToolCategoryFilter;

// =============================================================================
// Profile (Hermes 角色 概念的内化, v6 §6.4)
// =============================================================================

/// Profile — Agent 实例定义 (隔离执行单元).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// "prof_xxx"
    pub id: String,
    /// "techlead" | "coder-1" | "verifier" | "researcher"
    pub name: String,
    /// Local (in-process) | Remote (VPS 转发, v3)
    pub kind: ProfileKind,
    /// 隔离目录 (MVP v1 共享 cwd, v2 改子目录)
    pub working_dir: PathBuf,
    /// 默认 all, Role 可覆盖
    pub tool_filter: ToolCategoryFilter,
    /// 默认 32
    pub max_turns: u32,
    /// override 默认 model
    pub model: Option<String>,
    /// 占位符: {{role_instructions}} {{user_input}}
    pub system_prompt_template: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProfileKind {
    Local,
    Remote,
}

// =============================================================================
// Role (角色模板, v6 §6.4)
// =============================================================================

/// Role — 角色模板 (用户可注册, v1 硬编码 4 个默认).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    /// "role_xxx"
    pub id: String,
    /// "researcher" | "verifier" | "coder" | "techlead"
    pub name: String,
    pub description: String,
    /// "你是研究员, 关注 X, 产出 Y 格式"
    pub instructions: String,
    /// 默认绑定的 Profile
    pub default_profile_id: String,
    /// 允许的工具类别
    pub allowed_tool_categories: ToolCategoryFilter,
}

// =============================================================================
// TeamConfig (模式 6 调度预算, v6 §6.4)
// =============================================================================

/// Team 调度预算 (派生深度 + 并发预算 + 子超时 + 总开关).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    /// 默认 3 (含 orchestrator 本身)
    pub max_spawn_depth: u8,
    /// 默认 5
    pub max_concurrent_children: u16,
    /// 默认 5min
    pub child_timeout: Duration,
    /// 总开关, 紧急刹车. 默认 true
    pub orchestrator_enabled: bool,
    /// 是否对模糊任务自动 LLM Decompose (v1 默认 false, /decompose 强制)
    pub auto_decompose: bool,
    /// Swarm 必跑 Verifier? 默认 true
    pub verifier_required: bool,
}

impl Default for TeamConfig {
    fn default() -> Self {
        Self {
            max_spawn_depth: 3,
            max_concurrent_children: 5,
            child_timeout: Duration::from_secs(5 * 60),
            orchestrator_enabled: true,
            auto_decompose: false,
            verifier_required: true,
        }
    }
}

// =============================================================================
// TeamRegistry (v6 §6.4, MVP-2 plan 4)
// =============================================================================

/// Team 注册表 — 加载默认 4 个 role + profile, 提供 idle profile 查找.
#[derive(Clone)]
pub struct TeamRegistry {
    inner: Arc<Mutex<TeamRegistryInner>>,
}

struct TeamRegistryInner {
    roles: HashMap<String, Role>,
    profiles: HashMap<String, Profile>,
    /// profile_id -> 当前活跃 run 数
    active_runs: HashMap<String, u16>,
}

impl TeamRegistry {
    /// 加载默认 4 个 role + profile (techlead / coder / verifier / researcher).
    pub fn load_default() -> Self {
        let mut inner = TeamRegistryInner {
            roles: HashMap::new(),
            profiles: HashMap::new(),
            active_runs: HashMap::new(),
        };
        // 4 个默认 role + profile 互相对应
        for (id, name, instructions) in [
            (
                "role_techlead",
                "techlead",
                "你是技术 leader. 看到新 task 调 kanban_decompose 拆子任务, \
                 然后等 verifier 门控决定是否接受.",
            ),
            (
                "role_coder",
                "coder",
                "你是 coder. 完成后必须调 kanban_complete 写 outcome + summary.",
            ),
            (
                "role_verifier",
                "verifier",
                "你是 verifier. 完成后必须 [Review: OK] 或 [Review: Issues] \
                 写 metadata.gate=pass 或 block.",
            ),
            (
                "role_researcher",
                "researcher",
                "你是 researcher. 关注输出 Markdown 格式 + 引用清晰, 完成后调 kanban_complete.",
            ),
        ] {
            let role = Role {
                id: id.to_string(),
                name: name.to_string(),
                description: format!("Default {name} role"),
                instructions: instructions.to_string(),
                default_profile_id: format!("prof_{name}"),
                allowed_tool_categories: ToolCategoryFilter::all(),
            };
            let profile = Profile {
                id: format!("prof_{name}"),
                name: name.to_string(),
                kind: ProfileKind::Local,
                working_dir: PathBuf::from("."),
                tool_filter: ToolCategoryFilter::all(),
                max_turns: 32,
                model: None,
                system_prompt_template: "{{role_instructions}}\n\n{{user_input}}".to_string(),
            };
            inner.roles.insert(name.to_string(), role);
            inner.profiles.insert(name.to_string(), profile);
        }
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// 列出所有 role
    pub async fn list_roles(&self) -> Vec<Role> {
        let inner = self.inner.lock().await;
        let mut v: Vec<Role> = inner.roles.values().cloned().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    /// 按 name 查 role
    pub async fn get_role(&self, name: &str) -> Option<Role> {
        let inner = self.inner.lock().await;
        inner.roles.get(name).cloned()
    }

    /// 按 name 查 profile
    pub async fn get_profile(&self, name: &str) -> Option<Profile> {
        let inner = self.inner.lock().await;
        inner.profiles.get(name).cloned()
    }

    /// 找某 role 的 idle profile (active_runs < max_concurrent_children).
    /// 返 None 表示全部 profile 都在忙 (排队等).
    pub async fn find_idle_profile_for_role(
        &self,
        role_name: &str,
        max_concurrent: u16,
    ) -> Option<Profile> {
        let inner = self.inner.lock().await;
        let profile = inner.profiles.get(role_name)?;
        let active = inner.active_runs.get(&profile.id).copied().unwrap_or(0);
        // active 严格大于 max 才 busy (active == max 仍 idle, 等下一个 finished)
        if active > max_concurrent {
            return None;
        }
        Some(profile.clone())
    }

    /// 记录 profile 开始一个 run
    pub async fn mark_run_started(&self, profile_id: &str) {
        let mut inner = self.inner.lock().await;
        *inner.active_runs.entry(profile_id.to_string()).or_insert(0) += 1;
    }

    /// 记录 profile 结束一个 run
    pub async fn mark_run_finished(&self, profile_id: &str) {
        let mut inner = self.inner.lock().await;
        if let Some(count) = inner.active_runs.get_mut(profile_id) {
            if *count > 0 {
                *count -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_load_default_has_4_roles() {
        let reg = TeamRegistry::load_default();
        let roles = reg.list_roles().await;
        assert_eq!(roles.len(), 4);
        let names: Vec<&str> = roles.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"techlead"));
        assert!(names.contains(&"coder"));
        assert!(names.contains(&"verifier"));
        assert!(names.contains(&"researcher"));
    }

    #[tokio::test]
    async fn test_get_role_and_profile() {
        let reg = TeamRegistry::load_default();
        let role = reg.get_role("coder").await.expect("coder role");
        assert_eq!(role.name, "coder");
        let profile = reg.get_profile("coder").await.expect("coder profile");
        assert_eq!(profile.name, "coder");
        assert!(matches!(profile.kind, ProfileKind::Local));
        assert_eq!(profile.max_turns, 32);
    }

    #[tokio::test]
    async fn test_get_role_unknown_returns_none() {
        let reg = TeamRegistry::load_default();
        assert!(reg.get_role("unknown_role").await.is_none());
    }

    #[tokio::test]
    async fn test_find_idle_profile_default_max_5() {
        let reg = TeamRegistry::load_default();
        // max_concurrent=5: 初始 0 个 run, 应该有 idle profile
        let p = reg
            .find_idle_profile_for_role("coder", 5)
            .await
            .expect("idle coder");
        assert_eq!(p.name, "coder");
    }

    #[tokio::test]
    async fn test_find_idle_profile_full_returns_none() {
        let reg = TeamRegistry::load_default();
        // max_concurrent=1, 加 2 个 run 触发 busy
        reg.mark_run_started("prof_coder").await;
        reg.mark_run_started("prof_coder").await;
        let p = reg.find_idle_profile_for_role("coder", 1).await;
        assert!(p.is_none(), "should be busy at 2/1");
    }

    #[tokio::test]
    async fn test_mark_run_started_finished_tracking() {
        let reg = TeamRegistry::load_default();
        reg.mark_run_started("prof_coder").await;
        reg.mark_run_started("prof_coder").await;
        // 现在 coder profile 有 2 个 active run
        // max_concurrent=2: 应该 idle
        let p = reg
            .find_idle_profile_for_role("coder", 2)
            .await;
        assert!(p.is_some(), "should still be idle at 2/2");
        // 加第 3 个, 3 > 2: should be None
        reg.mark_run_started("prof_coder").await;
        let p = reg.find_idle_profile_for_role("coder", 2).await;
        assert!(p.is_none(), "should be busy at 3/2");
        // finished 1
        reg.mark_run_finished("prof_coder").await;
        let p = reg.find_idle_profile_for_role("coder", 2).await;
        assert!(p.is_some(), "should be idle at 2/2 after finished");
    }

    #[test]
    fn test_team_config_default_values() {
        let cfg = TeamConfig::default();
        assert_eq!(cfg.max_spawn_depth, 3);
        assert_eq!(cfg.max_concurrent_children, 5);
        assert_eq!(cfg.child_timeout, Duration::from_secs(300));
        assert!(cfg.orchestrator_enabled);
        assert!(!cfg.auto_decompose);
        assert!(cfg.verifier_required);
    }
}
