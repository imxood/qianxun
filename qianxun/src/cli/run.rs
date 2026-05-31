use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::AgentLoop;
use qianxun_core::agent::context::AutoCompactWindow;
use qianxun_core::agent::system_prompt;
use qianxun_core::config::ResolvedConfig;
use qianxun_core::skills::SkillWatcher;
use qianxun_core::tools::ToolRegistry;
use qianxun_core::workspace::ProjectRoot;
use crate::cli::cli::Repl;

pub async fn run_repl(
    resolved: &ResolvedConfig,
    workspace: Option<ProjectRoot>,
    resume: bool,
) -> anyhow::Result<()> {
    // 系统提示词（包含项目根路径 + 项目规则）
    let ws_context = workspace
        .as_ref()
        .map(qianxun_core::workspace::build_project_context)
        .unwrap_or_default();
    let skills_mgr = qianxun_core::skills::SkillManager::load_all(
        workspace.as_ref().map(|ws| ws.root.as_path()),
    );
    let skills_catalog = skills_mgr.build_catalog_prompt();
    let skills_list = skills_mgr.build_skills_list();
    let skills_count = skills_mgr.skill_count();
    let global_instructions = qianxun_core::workspace::read_global_agents_md();
    let system_prompt = system_prompt::build_system_prompt(&ws_context, global_instructions.as_deref());

    // 对话
    let mut conversation = Conversation::new(Some(system_prompt));
    conversation.set_budget(resolved.budget.max_input_tokens, resolved.budget.max_output_tokens);

    // Agent 循环
    let mut agent_loop = AgentLoop::new(resolved.agent.clone());
    agent_loop.compact_config = Some(resolved.compaction.clone());
    agent_loop.compact_window = Some(AutoCompactWindow::new(
        resolved.compaction.model_window,
        resolved.compaction.max_output_tokens,
        resolved.compaction.circuit_breaker_limit,
    ));

    // LLM Provider
    let provider = qianxun_core::provider::create_provider(&resolved.deepseek);

    // 工具注册
    let mut tools = ToolRegistry::new();
    qianxun_core::tools::builtin::register_all(&mut tools);

    // MCP 服务器引导（从工作区 .claude/mcp.json 加载）
    if let Some(ref ws) = workspace {
        match qianxun_core::mcp::config::McpConfigFile::find_in_workspace(&ws.root) {
            Ok(Some(config_file)) => {
                let server_configs = config_file.to_server_configs();
                for sc in &server_configs {
                    match qianxun_core::mcp::client::McpClient::connect(sc.clone()).await {
                        Ok(client) => {
                            match client.list_tools().await {
                                Ok(tool_list) => {
                                    for t in tool_list {
                                        tools.register_mcp_tool(qianxun_core::tools::McpToolEntry {
                                            client_id: client.server_name().to_string(),
                                            name: t.name,
                                            description: t.description,
                                            input_schema: t.input_schema,
                                        });
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("[mcp:{}] list_tools failed: {e}", sc.name);
                                }
                            }
                            tools.register_mcp_client(std::sync::Arc::new(client));
                            tracing::info!("[mcp] '{}' connected ({} tools)", sc.name, tools.mcp_count());
                        }
                        Err(e) => {
                            tracing::warn!("[mcp] '{}' connect failed: {e}", sc.name);
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("[mcp] config error: {e}");
            }
        }
    }

    // Memory（新引擎：SQLite）
    let memory: Option<Box<dyn qianxun_core::context::MemoryObserver + Send>> = qianxun_core::workspace::qianxun_dir().map(|mem_dir| {
        let db_path = mem_dir.join("mem.db");
        match qianxun_memory::MemoryCore::open(&db_path) {
            Ok(mc) => {
                tracing::info!("memory opened: {}", db_path.display());
                Box::new(mc) as Box<dyn qianxun_core::context::MemoryObserver + Send>
            }
            Err(e) => {
                tracing::warn!("memory init failed: {e}");
                // 没有 MemoryObserver 实现时返回一个无操作的默认实现
                Box::new(qianxun_memory::MemoryCore::open_in_memory().expect("in-memory fallback"))
                    as Box<dyn qianxun_core::context::MemoryObserver + Send>
            }
        }
    });

    // 工具列表（用于 /tools 命令）
    let tools_list = tools.format_tools_list();

    // 启动文件变更监听
    let skill_watcher = SkillWatcher::new(workspace.as_ref().map(|ws| ws.root.as_path()));

    // 启动 REPL
    let ws_root = workspace.as_ref().map(|ws| ws.root.clone());
    let mut repl = Repl::new(
        agent_loop, conversation, provider, tools,
        ws_context, memory,
        skills_mgr, skill_watcher, skills_catalog, skills_list, skills_count,
        tools_list, ws_root, resume, global_instructions,
    );
    repl.run().await
}
