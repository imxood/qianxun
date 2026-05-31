use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

type SessionId = String;

/// AgentLoop 会话宿主 —— 管理多个会话的生命周期。
pub struct AgentLoopHost {
    sessions: Arc<RwLock<HashMap<SessionId, SessionHandle>>>,
    max_sessions: usize,
}

struct SessionHandle {
    id: SessionId,
    created_at: Instant,
    last_active: Instant,
}

impl AgentLoopHost {
    pub fn new(max_sessions: usize) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            max_sessions,
        }
    }

    /// 创建新会话。
    pub fn create_session(&self) -> Result<SessionId, String> {
        let sessions = self.sessions.read().expect("AgentLoopHost lock poisoned");
        if sessions.len() >= self.max_sessions {
            return Err("Max sessions reached".into());
        }
        drop(sessions);

        let now = Utc::now();
        let id = format!("sess_{}", now.format("%Y%m%d_%H%M%S_%6f"));

        self.sessions.write().expect("AgentLoopHost lock poisoned").insert(id.clone(), SessionHandle {
            id: id.clone(),
            created_at: Instant::now(),
            last_active: Instant::now(),
        });

        Ok(id)
    }

    /// 检查会话是否存在。
    pub fn session_exists(&self, id: &str) -> bool {
        self.sessions.read().expect("AgentLoopHost lock poisoned").contains_key(id)
    }

    /// 删除会话。
    pub fn delete_session(&self, id: &str) {
        self.sessions.write().expect("AgentLoopHost lock poisoned").remove(id);
    }

    /// 清理过期会话（后台任务）。
    pub async fn reap_stale(&self) {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let timeout = Duration::from_secs(3600);
            let mut sessions = self.sessions.write().expect("AgentLoopHost lock poisoned");
            sessions.retain(|_, h| {
                let keep = h.last_active.elapsed() < timeout;
                if !keep {
                    tracing::info!("reaping stale session {}", h.id);
                }
                keep
            });
        }
    }
}
