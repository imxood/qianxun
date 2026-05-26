pub mod cli;
pub mod config;
pub mod output;

pub async fn run_repl(
    _verbose: bool,
    resolved: &qianxun_core::config::ResolvedConfig,
) -> anyhow::Result<()> {
    use qianxun_core::agent::conversation::Conversation;
    use qianxun_core::agent::engine::AgentLoop;
    use qianxun_core::agent::system_prompt;
    use qianxun_core::provider::deepseek::DeepSeekProvider;
    use qianxun_core::provider::LlmProvider;
    use qianxun_core::tools::ToolRegistry;
    use crate::cli::Repl;

    // 系统提示词
    let system_prompt = system_prompt::build_system_prompt("", "", None);

    // 对话
    let mut conversation = Conversation::new(Some(system_prompt));
    conversation.set_budget(resolved.budget.max_input_tokens, resolved.budget.max_output_tokens);

    // Agent 循环
    let agent_loop = AgentLoop::new(resolved.agent.clone());

    // LLM Provider
    let provider: Box<dyn LlmProvider> = Box::new(DeepSeekProvider::new(
        resolved.deepseek.api_key.clone(),
        resolved.deepseek.base_url.clone(),
        resolved.deepseek.model.clone(),
    ));

    // 工具注册
    let mut tools = ToolRegistry::new();
    qianxun_core::tools::builtin::register_all(&mut tools);

    // 启动 REPL
    let mut repl = Repl::new(agent_loop, conversation, provider, tools);
    repl.run().await
}
