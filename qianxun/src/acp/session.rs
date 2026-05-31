use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::AgentLoop;
use qianxun_core::context::memory::MemoryManager;
use qianxun_core::skills::{SkillManager, SkillWatcher};
use qianxun_core::tools::ToolRegistry;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// 单个 ACP 会话
pub struct AcpSession {
    pub id: String,
    pub conversation: Conversation,
    pub agent_loop: AgentLoop,
    pub created_at: String,
    pub is_running: bool,
    pub memory_manager: Option<MemoryManager>,
    /// 会话级工具注册表（含 MCP 工具），None 表示使用基础注册表
    pub tools: Option<Arc<ToolRegistry>>,
    /// 会话级技能目录，在 prompt 时注入
    pub skills_catalog: String,
    /// 工作区根路径，用于技能重载
    pub ws_root: Option<PathBuf>,
    /// 会话级技能管理器（避免每个 prompt 重载）
    pub skill_manager: Option<SkillManager>,
    /// 会话级技能文件变更监听
    pub skill_watcher: Option<SkillWatcher>,
    /// 取消标志，由 session/cancel 通知触发
    pub cancel_flag: Arc<AtomicBool>,
}

/// 会话管理器
pub struct SessionManager {
    sessions: HashMap<String, AcpSession>,
    max_sessions: u32,
    sessions_dir: Option<PathBuf>,
}

impl SessionManager {
    pub fn new(max_sessions: u32) -> Self {
        Self {
            sessions: HashMap::new(),
            max_sessions,
            sessions_dir: None,
        }
    }

    /// 创建带持久化目录的 SessionManager
    pub fn new_with_dir(max_sessions: u32, sessions_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&sessions_dir);
        Self {
            sessions: HashMap::new(),
            max_sessions,
            sessions_dir: Some(sessions_dir),
        }
    }

    /// 创建新会话
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        id: String,
        system_prompt: Option<String>,
        agent_loop: AgentLoop,
        memory_manager: Option<MemoryManager>,
        tools: Option<Arc<ToolRegistry>>,
        skills_catalog: String,
        skill_manager: Option<SkillManager>,
        skill_watcher: Option<SkillWatcher>,
        ws_root: Option<PathBuf>,
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
                memory_manager,
                tools,
                skills_catalog,
                ws_root,
                skill_manager,
                skill_watcher,
                cancel_flag: Arc::new(AtomicBool::new(false)),
            },
        );

        Ok(self.sessions.get_mut(&id).unwrap())
    }

    /// 保存会话到 JSONL（含 .meta 文件）。
    pub async fn save_session(&self, id: &str) {
        let dir = match self.sessions_dir.as_ref() {
            Some(d) => d.clone(),
            None => return,
        };
        let session = match self.sessions.get(id) {
            Some(s) => s,
            None => return,
        };

        let jsonl_path = dir.join(format!("{id}.jsonl"));
        if let Err(e) = session.conversation.save_to(&jsonl_path).await {
            tracing::warn!("[session] save {id} jsonl failed: {e}");
            return;
        }

        // .meta 文件
        let preview = session.conversation.messages().first()
            .and_then(|m| m.content().first())
            .and_then(|b| b.text.as_deref())
            .map(|t| {
                if t.len() > 80 {
                    let end = (0..=80).rev().find(|&i| t.is_char_boundary(i)).unwrap_or(0);
                    &t[..end]
                } else {
                    t
                }
            })
            .unwrap_or("");
        let meta = serde_json::json!({
            "created_at": session.created_at,
            "message_count": session.conversation.messages().len(),
            "turn_count": session.agent_loop.turn_count,
            "preview": preview,
        });
        let meta_path = dir.join(format!("{id}.meta"));
        let _ = std::fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap_or_default());
        tracing::debug!("[session] saved {id} to disk");
    }

    /// 从磁盘加载 JSONL 到会话（替换内存中的 conversation）。
    /// 会话必须已存在于内存中，否则静默返回 false。
    pub async fn load_session(&mut self, id: &str) -> bool {
        let dir = match self.sessions_dir.as_ref() {
            Some(d) => d.clone(),
            None => return false,
        };
        let jsonl_path = dir.join(format!("{id}.jsonl"));
        if !jsonl_path.exists() {
            return false;
        }
        match Conversation::load_from(&jsonl_path).await {
            Ok(conv) => {
                if let Some(session) = self.sessions.get_mut(id) {
                    let msg_count = conv.messages().len();
                    session.conversation = conv;
                    tracing::info!("[session] loaded {id} from disk ({msg_count} messages)");
                    true
                } else {
                    false
                }
            }
            Err(e) => {
                tracing::warn!("[session] load {id} failed: {e}");
                false
            }
        }
    }

    /// 获取会话
    pub fn get(&mut self, id: &str) -> Option<&mut AcpSession> {
        self.sessions.get_mut(id)
    }

    /// 从现有会话 fork 出一个新会话（复制会话状态到新 ID）。
    /// 新会话获得独立的 conversation/agent_loop，共享 tools（Arc）。
    /// memory_manager 和 skill_watcher 不支持 clone，fork 后设为 None。
    pub fn fork(&mut self, new_id: &str, source_id: &str) -> Result<&mut AcpSession, String> {
        if self.sessions.len() as u32 >= self.max_sessions {
            return Err("max sessions reached".into());
        }
        let source = self.sessions.get(source_id).ok_or_else(|| format!("source session not found: {source_id}"))?;

        let now = chrono::Utc::now().to_rfc3339();
        self.sessions.insert(
            new_id.to_string(),
            AcpSession {
                id: new_id.to_string(),
                conversation: source.conversation.clone(),
                agent_loop: source.agent_loop.clone(),
                created_at: now,
                is_running: false,
                memory_manager: None,
                tools: source.tools.clone(),
                skills_catalog: source.skills_catalog.clone(),
                ws_root: source.ws_root.clone(),
                skill_manager: source.skill_manager.clone(),
                skill_watcher: None,
                cancel_flag: Arc::new(AtomicBool::new(false)),
            },
        );

        Ok(self.sessions.get_mut(new_id).unwrap())
    }

    /// 关闭会话
    pub fn close(&mut self, id: &str) -> bool {
        self.sessions.remove(id).is_some()
    }

    /// 删除会话（从内存和磁盘同时删除）。
    /// 先保存再删除磁盘文件，然后从内存移除。
    pub async fn delete(&mut self, id: &str) -> bool {
        self.save_session(id).await;
        if let Some(ref dir) = self.sessions_dir {
            let jsonl = dir.join(format!("{id}.jsonl"));
            let meta = dir.join(format!("{id}.meta"));
            let _ = std::fs::remove_file(&jsonl);
            let _ = std::fs::remove_file(&meta);
        }
        self.sessions.remove(id).is_some()
    }

    /// 会话数量
    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    /// 列出所有会话信息
    pub fn list(&self) -> Vec<crate::acp::types::SessionInfo> {
        self.sessions
            .values()
            .map(|s| crate::acp::types::SessionInfo {
                session_id: s.id.clone(),
                created_at: s.created_at.clone(),
                turn_count: s.agent_loop.turn_count,
            })
            .collect()
    }
}
