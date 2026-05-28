use std::io::{self, BufRead, Write};
use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::{processing_loop, AgentLoop};
use qianxun_core::agent::message::ContentBlock;
use qianxun_core::agent::system_prompt;
use qianxun_core::context::memory::MemoryManager;
use qianxun_core::provider::LlmProvider;
use qianxun_core::tools::ToolRegistry;
use crate::output::CliOutputSink;

pub struct Repl {
    running: bool,
    agent_loop: AgentLoop,
    conversation: Conversation,
    provider: Box<dyn LlmProvider>,
    tools: ToolRegistry,
    sink: CliOutputSink,
    budget: (Option<u64>, Option<u64>), // (max_input, max_output) for /reset
    workspace_context: String,
    memory_manager: Option<MemoryManager>,
    skills_catalog: String,
    skills_list: String,
}

impl Repl {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent_loop: AgentLoop,
        conversation: Conversation,
        provider: Box<dyn LlmProvider>,
        tools: ToolRegistry,
        workspace_context: String,
        memory_manager: Option<MemoryManager>,
        skills_catalog: String,
        skills_list: String,
    ) -> Self {
        let budget = (
            conversation.budget().max_input_tokens,
            conversation.budget().max_output_tokens,
        );
        Self {
            running: true,
            agent_loop,
            conversation,
            provider,
            tools,
            sink: CliOutputSink,
            budget,
            workspace_context,
            memory_manager,
            skills_catalog,
            skills_list,
        }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        eprintln!("🐾 千寻 (Qianxun) — AI 编程助手");
        eprintln!("输入 /quit 退出，/help 查看帮助");

        let stdin = io::stdin();
        let mut stdout = io::stdout();

        while self.running {
            print!("\n❯ ");
            stdout.flush()?;

            let mut line = String::new();
            if stdin.lock().read_line(&mut line).is_err() || line.trim().is_empty() {
                continue;
            }

            let input = line.trim();

            if input.starts_with('/') {
                self.handle_slash(input).await;
            } else {
                self.handle_message(input).await;
            }
        }

        Ok(())
    }

    async fn handle_slash(&mut self, cmd: &str) {
        match cmd {
            "/quit" | "/exit" => {
                eprintln!("再见！");
                self.running = false;
            }
            "/help" => {
                eprintln!("可用命令：");
                eprintln!("  /quit      退出千寻");
                eprintln!("  /help      显示此帮助");
                eprintln!("  /reset     重置对话");
                eprintln!("  /usage     显示 token 用量");
                eprintln!("  /workspace 显示工作区信息");
                eprintln!("  /skills    显示已加载的技能");
                eprintln!("  /memory    显示最近记忆");
            }
            "/reset" => {
                let sys = system_prompt::build_system_prompt(
                    &self.workspace_context, &self.skills_catalog, None,
                );
                self.conversation = Conversation::new(Some(sys));
                self.conversation.set_budget(self.budget.0, self.budget.1);
                self.agent_loop.reset();
                eprintln!("对话已重置");
            }
            "/workspace" => {
                if self.workspace_context.is_empty() {
                    eprintln!("未检测到工作区。使用 -w / --workspace 指定项目路径。");
                } else {
                    eprintln!("当前工作区：\n{}", self.workspace_context);
                }
            }
            "/usage" => {
                let usage = &self.agent_loop.accumulated_usage;
                eprintln!("Token 用量：");
                eprintln!("  输入 token: {}", usage.input);
                eprintln!("  输出 token: {}", usage.output);
                if let Some(cc) = usage.cache_creation_input {
                    eprintln!("  缓存创建: {cc}");
                }
                if let Some(cr) = usage.cache_read_input {
                    eprintln!("  缓存读取: {cr}");
                }
            }
            "/skills" => {
                if self.skills_list.contains("（无）") {
                    eprintln!("未加载任何技能。");
                } else {
                    eprintln!("已加载的技能：\n{}", self.skills_list);
                }
            }
            "/memory" => {
                match &self.memory_manager {
                    Some(mm) => {
                        let ctx = mm.build_context();
                        if ctx.is_empty() {
                            eprintln!("尚无记忆。");
                        } else {
                            eprintln!("最近记忆：\n{ctx}");
                        }
                    }
                    None => {
                        eprintln!("未启用记忆（需要工作区）。");
                    }
                }
            }
            _ => {
                eprintln!("未知命令: {cmd}");
            }
        }
    }

    async fn handle_message(&mut self, msg: &str) {
        // 构建记忆上下文
        let memory_context = self
            .memory_manager
            .as_ref()
            .map(|mm| mm.build_context())
            .unwrap_or_default();

        self.conversation
            .push_user_message(vec![ContentBlock::text(msg)]);

        processing_loop::handle_user_message(
            &mut self.agent_loop,
            &mut self.conversation,
            &*self.provider,
            &self.tools,
            &self.sink,
            &memory_context,
            &self.skills_catalog,
        )
        .await;

        // 轮次后写入记忆
        if let Some(mm) = &self.memory_manager {
            let summary = if msg.len() > 200 { &msg[..200] } else { msg };
            mm.write_memory(summary, &["conversation"], msg);
        }
    }
}
