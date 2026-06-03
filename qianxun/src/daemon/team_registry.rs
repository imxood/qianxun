//! Team Registry (v6 §14.1 MVP-3 plan 0)
//!
//! daemon 端 Profile / Role 加载 + 跟 KanbanDispatcher 集成.
//! v1 硬编码 4 个默认 role + profile (techlead / coder / verifier / researcher).

use qianxun_core::kanban::team::{Profile, TeamRegistry};

/// Daemon 端 Team Registry 封装 (v6 §6.4 落地).
///
/// MVP-3 阶段: 复用 qianxun-core 的 TeamRegistry (MVP-2 plan 4 已实现),
/// 加 daemon 侧配置加载 (从 ~/.qianxun/teams.toml 读 user 自定义 profile,
/// 留 v2).
pub struct DaemonTeamRegistry {
    inner: TeamRegistry,
}

impl DaemonTeamRegistry {
    /// 创建 daemon team registry (默认 4 role + profile, MVP-2 plan 4)
    pub fn load_default() -> Self {
        Self {
            inner: TeamRegistry::load_default(),
        }
    }

    /// 暴露内部 registry (供 dispatcher 复用)
    pub fn inner(&self) -> &TeamRegistry {
        &self.inner
    }

    /// 列出所有 profile (HTTP 路由用)
    pub async fn list_profiles(&self) -> Vec<Profile> {
        let roles = self.inner.list_roles().await;
        let mut profiles = Vec::new();
        for r in roles {
            let name = r.default_profile_id.replace("prof_", "");
            if let Some(p) = self.inner.get_profile(&name).await {
                profiles.push(p);
            }
        }
        profiles
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_load_default_has_4_profiles() {
        let reg = DaemonTeamRegistry::load_default();
        let profiles = reg.list_profiles().await;
        assert_eq!(profiles.len(), 4);
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"techlead"));
        assert!(names.contains(&"coder"));
        assert!(names.contains(&"verifier"));
        assert!(names.contains(&"researcher"));
    }
}
