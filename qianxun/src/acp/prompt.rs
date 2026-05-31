use crate::acp::output::{AcpOutputEvent, AcpOutputSink};
use crate::acp::handler::AcpRequestHandler;
use crate::acp::types::*;
use qianxun_core::agent::context::window::AutoCompactWindow;
use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::processing_loop;
use qianxun_core::agent::engine::AgentLoop;
use qianxun_core::agent::message::ContentBlock;
use qianxun_core::config::ResolvedCompactionConfig;
use qianxun_core::context::memory::MemoryManager;
use qianxun_core::provider::LlmProvider;
use qianxun_core::skills::SkillManager;
use qianxun_core::tools::ToolRegistry;
use qianxun_core::types::AgentConfig;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tracing;

// ─── 提示词执行桥接 ─────────────────────────────────────

/// 创建 AgentLoop 并初始化上下文压缩窗口（如有配置）。
pub(crate) fn new_agent_loop(agent_config: AgentConfig, compact_config: &Option<ResolvedCompactionConfig>) -> AgentLoop {
    let mut agent_loop = AgentLoop::new(agent_config);
    if let Some(cc) = compact_config {
        agent_loop.compact_config = Some(cc.clone());
        agent_loop.compact_window = Some(AutoCompactWindow::new(
            cc.model_window,
            cc.max_output_tokens,
            cc.circuit_breaker_limit,
        ));
    }
    agent_loop
}

impl AcpRequestHandler {
    /// session/prompt 的延迟处理入口：先校验参数，然后异步执行
    pub(crate) async fn handle_session_prompt_deferred(&self, id: RequestId, params: Option<Value>) {
        match self.prepare_prompt(params).await {
            Ok((session_id, agent_loop, conversation, prompt_text, memory_context, memory_manager, tools, skills_catalog, skill_injections, cancel_flag, skill_manager)) => {
                self.run_prompt_task(id, session_id, agent_loop, conversation, prompt_text, memory_context, memory_manager, tools, skills_catalog, skill_injections, cancel_flag, skill_manager)
                    .await;
            }
            Err(e) => {
                let resp = rpc_error(id, -32603, e);
                let _ = self.transport.send_response(&resp).await;
            }
        }
    }

    /// 准备 prompt 执行的参数（校验 + 提取 session）
    #[allow(clippy::type_complexity)]
    async fn prepare_prompt(
        &self,
        params: Option<Value>,
    ) -> Result<(String, AgentLoop, Conversation, String, String, Option<MemoryManager>, Arc<ToolRegistry>, String, String, Arc<AtomicBool>, Option<SkillManager>), String> {
        let p: PromptParams = params
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| "invalid prompt params".to_string())?;

        let session_id = p.session_id.clone();
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get(&session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?;

        if session.is_running {
            return Err("session already running".to_string());
        }
        session.is_running = true;

        // 提取记忆上下文
        let memory_context = match &session.memory_manager {
            Some(mm) => mm.build_context().await,
            None => String::new(),
        };
        let memory_manager = std::mem::take(&mut session.memory_manager);

        // 提取会话级工具注册表（如有），否则使用基础注册表
        let session_tools = session
            .tools
            .take()
            .unwrap_or_else(|| self.tools.clone());

        let empty_loop = new_agent_loop(self.agent_config.clone(), &self.compact_config);
        let empty_conv = Conversation::new(None);
        let agent_loop = std::mem::replace(&mut session.agent_loop, empty_loop);
        let conversation = std::mem::replace(&mut session.conversation, empty_conv);
        let cancel_flag = session.cancel_flag.clone();

        // 检查技能文件变更（在取出 skill_manager 之前）
        if let Some(ref mut watcher) = session.skill_watcher {
            if watcher.has_changed() {
                tracing::info!("[skill_watcher] file change detected (prepare_prompt), reloading skills");
                if let Some(ref mut sm) = session.skill_manager {
                    sm.reload(session.ws_root.as_deref());
                    session.skills_catalog = sm.build_catalog_prompt();
                }
            }
        }

        let skills_catalog = session.skills_catalog.clone();
        let skill_manager = session.skill_manager.take();

        drop(sessions);

        let user_text: String = p
            .prompt
            .iter()
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n");

        // 构建技能注入内容（Layer 2 — 自动匹配 + 手动引用）
        // 使用会话缓存的 skill_manager，避免每个 prompt 从磁盘重载
        let loaded_fallback;
        let skills_mgr: &SkillManager = match skill_manager.as_ref() {
            Some(sm) => sm,
            None => {
                loaded_fallback = SkillManager::load_all(None::<&std::path::Path>);
                &loaded_fallback
            }
        };
        let skill_injections = {
            let user_text_for_match = if user_text.is_empty() { "..." } else { &user_text };

            // 手动引用 @技能名
            let manual_names: Vec<String> = SkillManager::extract_manual_mentions(user_text_for_match)
                .into_iter()
                .filter(|name| skills_mgr.select_by_name(name).is_some())
                .collect();

            // 自动匹配
            let exclude: Vec<&str> = manual_names.iter().map(|s| s.as_str()).collect();
            let auto_names = skills_mgr.auto_select(user_text_for_match, &exclude);

            let mut inject_names = manual_names;
            inject_names.extend(auto_names);

            skills_mgr.build_injections(&inject_names)
        };

        Ok((session_id, agent_loop, conversation, user_text, memory_context, memory_manager, session_tools, skills_catalog, skill_injections, cancel_flag, skill_manager))
    }

    /// 在后台任务中执行 prompt，完成后通过 output_tx 发送 JSON-RPC 响应
    #[allow(clippy::too_many_arguments)]
    async fn run_prompt_task(
        &self,
        id: RequestId,
        session_id: String,
        mut agent_loop: AgentLoop,
        mut conversation: Conversation,
        prompt_text: String,
        memory_context: String,
        memory_manager: Option<MemoryManager>,
        tools: Arc<ToolRegistry>,
        skills_catalog: String,
        skill_injections: String,
        cancel_flag: Arc<AtomicBool>,
        skill_manager: Option<SkillManager>,
    ) {
        conversation
            .push_user_message(vec![ContentBlock::text(&prompt_text)]);

        let sink = AcpOutputSink::new(session_id.clone(), self.output_tx.clone());
        let provider: Arc<dyn LlmProvider> = self.provider.clone();
        let tools_for_spawn = tools.clone();
        let skills_catalog_for_spawn = skills_catalog;
        let skill_injections_for_spawn = skill_injections;
        let sid = session_id.clone();

        // 第一层：处理循环（可能 panic）
        let processing_handle = tokio::spawn(async move {
            processing_loop::handle_user_message(
                &mut agent_loop,
                &mut conversation,
                provider.as_ref(),
                tools_for_spawn.as_ref(),
                &sink,
                &memory_context,
                &skills_catalog_for_spawn,
                &skill_injections_for_spawn,
                cancel_flag,
            )
            .await;
            (agent_loop, conversation) // 正常完成后返还所有权
        });

        // 第二层：监护任务，确保响应一定发送
        let output_tx = self.output_tx.clone();
        let sessions_arc2 = self.sessions.clone();
        tokio::spawn(async move {
            match processing_handle.await {
                Ok((agent_loop, conversation)) => {
                    // 处理正常完成，保存会话状态
                    // 写入记忆
                    let summary = if prompt_text.len() > 200 {
                        let end = (0..=200).rev().find(|&i| prompt_text.is_char_boundary(i)).unwrap_or(0);
                        &prompt_text[..end]
                    } else {
                        &prompt_text
                    };
                    if let Some(mm) = &memory_manager {
                        mm.write_memory(summary, &["conversation"], &prompt_text).await;
                    }

                    let mut sessions = sessions_arc2.lock().await;
                    if let Some(s) = sessions.get(&sid) {
                        drop(std::mem::replace(&mut s.conversation, conversation));
                        drop(std::mem::replace(&mut s.agent_loop, agent_loop));
                        s.memory_manager = memory_manager;
                        s.tools = Some(tools);
                        s.skill_manager = skill_manager;
                        s.is_running = false;
                    }
                    // 持久化到磁盘
                    sessions.save_session(&sid).await;
                }
                Err(panic_err) => {
                    // 处理过程 panic，标记会话为未运行
                    tracing::error!("Processing task panicked: {:?}", panic_err);
                    let mut sessions = sessions_arc2.lock().await;
                    if let Some(s) = sessions.get(&sid) {
                        s.memory_manager = memory_manager;
                        s.skill_manager = skill_manager;
                        s.is_running = false;
                    }
                }
            }

            // ★ 关键：无论处理成功还是失败，都必须发送响应
            let _ = output_tx.send(AcpOutputEvent::PromptResponse {
                id,
                stop_reason: "end_turn".to_string(),
            });
        });
    }
}
