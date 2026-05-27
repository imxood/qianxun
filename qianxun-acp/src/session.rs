use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::AgentLoop;
use std::collections::HashMap;

/// 单个 ACP 会话
pub struct AcpSession {
    pub id: String,
    pub conversation: Conversation,
    pub agent_loop: AgentLoop,
    pub created_at: String,
    pub is_running: bool,
}

/// 会话管理器
pub struct SessionManager {
    sessions: HashMap<String, AcpSession>,
    max_sessions: u32,
}

impl SessionManager {
    pub fn new(max_sessions: u32) -> Self {
        Self {
            sessions: HashMap::new(),
            max_sessions,
        }
    }

    /// 创建新会话
    pub fn create(
        &mut self,
        id: String,
        system_prompt: Option<String>,
        agent_loop: AgentLoop,
    ) -> Result<&mut AcpSession, String> {
        if self.sessions.len() as u32 >= self.max_sessions {
            return Err("max sessions reached".into());
        }

        let conversation = Conversation::new(system_prompt);
        let now = chrono::Utc::now().to_rfc3339();

        self.sessions.insert(
            id.clone(),
            AcpSession {
                id: id.clone(),
                conversation,
                agent_loop,
                created_at: now,
                is_running: false,
            },
        );

        Ok(self.sessions.get_mut(&id).unwrap())
    }

    /// 获取会话
    pub fn get(&mut self, id: &str) -> Option<&mut AcpSession> {
        self.sessions.get_mut(id)
    }

    /// 关闭会话
    pub fn close(&mut self, id: &str) -> bool {
        self.sessions.remove(id).is_some()
    }

    /// 会话数量
    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    /// 列出所有会话信息
    pub fn list(&self) -> Vec<crate::types::SessionInfo> {
        self.sessions
            .values()
            .map(|s| crate::types::SessionInfo {
                session_id: s.id.clone(),
                created_at: s.created_at.clone(),
                turn_count: s.agent_loop.turn_count,
            })
            .collect()
    }
}
