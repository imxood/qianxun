use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::{processing_loop, AgentLoop};
use qianxun_core::agent::message::{ContentBlock, Message};
use qianxun_core::agent::system_prompt;
use qianxun_core::context::memory::MemoryManager;
use qianxun_core::provider::LlmProvider;
use qianxun_core::skills::SkillManager;
use qianxun_core::tools::ToolRegistry;

use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::history::FileHistory;
use rustyline::highlight::CmdKind;
use rustyline::{Config, Context, Editor, Helper};

use crate::output::CliOutputSink;

const SLASH_COMMANDS: &[&str] = &[
    "/help", "/quit", "/exit", "/reset",
    "/usage", "/workspace",
    "/skills", "/tools", "/memory",
    "/retry", "/edit",
    "/sessions",
];

// ─── ReplHelper: rustyline 补全/提示/高亮/验证 ─────────────

#[derive(Default)]
struct ReplHelper;

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        if line.starts_with('/') {
            let candidates: Vec<Pair> = SLASH_COMMANDS
                .iter()
                .filter(|cmd| cmd.starts_with(line))
                .map(|cmd| Pair {
                    display: cmd.to_string(),
                    replacement: cmd.strip_prefix('/').unwrap_or(cmd).to_string(),
                })
                .collect();
            return Ok((1, candidates));
        }
        Ok((pos, Vec::new()))
    }
}

impl Hinter for ReplHelper {
    type Hint = String;

    fn hint(&self, line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<String> {
        if line == "/" {
            return Some("quit | help | reset | skills | tools | usage | workspace | memory | retry | edit | sessions".into());
        }
        if line.starts_with('/') && line.len() > 1 {
            for cmd in SLASH_COMMANDS {
                if cmd.starts_with(line) && cmd.len() > line.len() {
                    return Some(cmd[line.len()..].to_string());
                }
            }
        }
        None
    }
}

impl Highlighter for ReplHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if line.starts_with('/') {
            Cow::Owned(format!("\x1b[36m{line}\x1b[0m"))
        } else {
            Cow::Borrowed(line)
        }
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _ch: CmdKind) -> bool {
        true
    }
}

impl Validator for ReplHelper {
    fn validate(&self, ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        let input = ctx.input().trim().to_string();

        // 空行结束多行模式
        if input.is_empty() && input.contains('\n') {
            return Ok(ValidationResult::Valid(None));
        }

        // { 开头但未闭合 → 多行模式
        if input.starts_with('{') && !input.ends_with('}') {
            return Ok(ValidationResult::Incomplete);
        }

        Ok(ValidationResult::Valid(None))
    }
}

impl Helper for ReplHelper {}

// ─── REPL ───────────────────────────────────────────────────

pub struct Repl {
    running: bool,
    agent_loop: AgentLoop,
    conversation: Conversation,
    provider: Box<dyn LlmProvider>,
    tools: ToolRegistry,
    sink: CliOutputSink,
    budget: (Option<u64>, Option<u64>),
    workspace_context: String,
    memory_manager: Option<MemoryManager>,
    skill_manager: SkillManager,
    skills_catalog: String,
    skills_list: String,
    skills_count: usize,
    tools_list: String,
    recently_injected: Vec<String>,
    last_message: Option<String>,
    cancel_flag: Arc<AtomicBool>,
    shutdown_notify: Arc<tokio::sync::Notify>,
    rl: Editor<ReplHelper, FileHistory>,
    #[allow(dead_code)]
    ws_root: Option<PathBuf>,
    sessions_dir: Option<PathBuf>,
    current_session: Option<String>,
    resume: bool,
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
        skill_manager: SkillManager,
        skills_catalog: String,
        skills_list: String,
        skills_count: usize,
        tools_list: String,
        ws_root: Option<PathBuf>,
        resume: bool,
    ) -> Self {
        let budget = (
            conversation.budget().max_input_tokens,
            conversation.budget().max_output_tokens,
        );

        // 初始化 rustyline
        let config = Config::builder()
            .max_history_size(100)
            .expect("max_history_size failed")
            .build();
        let mut rl = Editor::<ReplHelper, FileHistory>::with_config(config).expect("failed to create REPL editor");
        rl.set_helper(Some(ReplHelper));
        if let Some(path) = Self::history_path() {
            let _ = rl.load_history(&path);
        }

        Self {
            running: true,
            agent_loop,
            conversation,
            provider,
            tools,
            sink: CliOutputSink::new(),
            budget,
            workspace_context,
            memory_manager,
            skill_manager,
            skills_catalog,
            skills_list,
            skills_count,
            tools_list,
            recently_injected: Vec::new(),
            last_message: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            rl,
            ws_root,
            sessions_dir: Self::sessions_dir(),
            current_session: None,
            resume,
        }
    }

    /// 历史文件路径: ~/.qianxun/history.txt
    fn history_path() -> Option<PathBuf> {
        let home = if cfg!(target_os = "windows") {
            std::env::var("USERPROFILE").ok()
        } else {
            std::env::var("HOME").ok()
        }?;
        let dir = PathBuf::from(home).join(".qianxun");
        let _ = std::fs::create_dir_all(&dir);
        Some(dir.join("history.txt"))
    }

    /// 会话目录: ~/.qianxun/sessions/
    fn sessions_dir() -> Option<PathBuf> {
        let home = if cfg!(target_os = "windows") {
            std::env::var("USERPROFILE").ok()
        } else {
            std::env::var("HOME").ok()
        }?;
        let dir = PathBuf::from(home).join(".qianxun").join("sessions");
        let _ = std::fs::create_dir_all(&dir);
        Some(dir)
    }

    /// 保存当前会话到 JSONL + meta 文件
    async fn save_conversation(&self) {
        let dir = match &self.sessions_dir {
            Some(d) => d.clone(),
            None => return,
        };
        let Some(session_id) = self.current_session.as_deref() else { return };
        let jsonl_path = dir.join(format!("{session_id}.jsonl"));
        let meta_path = dir.join(format!("{session_id}.meta"));

        if let Err(e) = self.conversation.save_to(&jsonl_path).await {
            tracing::warn!("保存会话失败: {e}");
            return;
        }

        // 更新 meta 文件
        let msg_count = self.conversation.messages().len();
        let preview = self.conversation.messages().first()
            .and_then(|m| {
                m.content().first()
                    .and_then(|b| b.text.as_deref())
                    .map(|t| {
                        if t.len() > 80 {
                            let end = (0..=80).rev().find(|&i| t.is_char_boundary(i)).unwrap_or(0);
                            &t[..end]
                        } else {
                            t
                        }
                    })
            })
            .unwrap_or("");
        let meta = serde_json::json!({
            "created_at": session_id,
            "message_count": msg_count,
            "preview": preview,
        });
        let _ = std::fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap_or_default());
    }

    /// 列出所有会话
    fn list_sessions() -> Vec<SessionInfo> {
        let dir = match Self::sessions_dir() {
            Some(d) => d,
            None => return Vec::new(),
        };

        let mut sessions = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else { return sessions };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let id = path.file_stem().and_then(|n| n.to_str()).unwrap_or("").to_string();
            let meta = path.with_extension("meta");
            let (created_at, preview) = if let Ok(content) = std::fs::read_to_string(&meta) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    let ca = val["created_at"].as_str().unwrap_or(&id).to_string();
                    let pr = val["preview"].as_str().unwrap_or("").to_string();
                    (ca, pr)
                } else {
                    (id.clone(), String::new())
                }
            } else {
                (id.clone(), String::new())
            };
            let message_count = std::fs::read_to_string(&path)
                .ok()
                .map(|c| c.lines().count().saturating_sub(1))
                .unwrap_or(0);

            sessions.push(SessionInfo { id, created_at, message_count, preview });
        }

        sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        sessions
    }

    /// 创建新会话 ID（基于当前时间）
    fn new_session_id() -> String {
        chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
    }

    /// 恢复历史会话（模糊匹配 ID）
    async fn handle_session_resume(&mut self, partial: &str) {
        let sessions = Self::list_sessions();
        let matches: Vec<&SessionInfo> = sessions.iter().filter(|s| s.id.contains(partial)).collect();

        if matches.is_empty() {
            eprintln!("未找到包含 \"{partial}\" 的会话。\n使用 /sessions 查看可用的历史会话。");
            return;
        }
        if matches.len() > 1 {
            eprintln!("\"{partial}\" 匹配了多个会话：");
            for s in &matches {
                eprintln!("  \x1b[36m{}\x1b[0m  {} 条消息  {}", s.id, s.message_count, s.preview);
            }
            eprintln!("请提供更精确的 ID。");
            return;
        }

        let session = matches[0];
        let dir = match &self.sessions_dir {
            Some(d) => d.clone(),
            None => {
                eprintln!("无法访问会话目录。");
                return;
            }
        };
        let path = dir.join(format!("{}.jsonl", session.id));

        match Conversation::load_from(&path).await {
            Ok(conv) => {
                // 保存当前会话（如果有）
                self.save_conversation().await;
                self.conversation = conv;
                self.conversation.set_budget(self.budget.0, self.budget.1);
                self.current_session = Some(session.id.clone());
                self.agent_loop.reset();
                self.recently_injected.clear();
                eprintln!("已恢复会话: \x1b[36m{}\x1b[0m ({} 条消息)", session.id, session.message_count);
                Self::print_conversation(&self.conversation);
            }
            Err(e) => {
                eprintln!("恢复会话失败: {e}");
            }
        }
    }

    /// 删除历史会话
    async fn handle_session_delete(&mut self, partial: &str) {
        let sessions = Self::list_sessions();
        let matches: Vec<&SessionInfo> = sessions.iter().filter(|s| s.id.contains(partial)).collect();

        if matches.is_empty() {
            eprintln!("未找到包含 \"{partial}\" 的会话。");
            return;
        }
        if matches.len() > 1 {
            eprintln!("\"{partial}\" 匹配了多个会话，请提供更精确的 ID。");
            return;
        }

        let session = matches[0];
        let dir = match &self.sessions_dir {
            Some(d) => d.clone(),
            None => {
                eprintln!("无法访问会话目录。");
                return;
            }
        };

        let jsonl_path = dir.join(format!("{}.jsonl", session.id));
        let meta_path = dir.join(format!("{}.meta", session.id));

        let _ = std::fs::remove_file(&jsonl_path);
        let _ = std::fs::remove_file(&meta_path);

        // 如果删除的是当前会话，清除 current_session
        if self.current_session.as_deref() == Some(&session.id) {
            self.current_session = None;
        }

        eprintln!("已删除会话: \x1b[36m{}\x1b[0m", session.id);
    }

    /// 启动时恢复最后一次会话
    async fn resume_last_session(&mut self) {
        let dir = match &self.sessions_dir {
            Some(d) => d.clone(),
            None => return,
        };
        let sessions = Self::list_sessions();
        let latest = match sessions.into_iter().next() {
            Some(s) => s,
            None => return,
        };
        let path = dir.join(format!("{}.jsonl", latest.id));
        match Conversation::load_from(&path).await {
            Ok(conv) => {
                let msg_count = conv.messages().len();
                self.conversation = conv;
                self.conversation.set_budget(self.budget.0, self.budget.1);
                self.current_session = Some(latest.id.clone());
                self.agent_loop.reset();
                self.recently_injected.clear();
                eprintln!(
                    "\x1b[32m已恢复会话: {} ({} 条消息)\x1b[0m",
                    latest.id, msg_count,
                );
                Self::print_conversation(&self.conversation);
            }
            Err(e) => {
                eprintln!("\x1b[33m恢复会话失败: {e}\x1b[0m");
            }
        }
    }

    /// 打印会话中的消息文本（用于恢复时展示历史）
    fn print_conversation(conv: &Conversation) {
        let messages = conv.messages();
        if messages.is_empty() {
            return;
        }
        eprintln!("\x1b[2m━━━ 历史消息 ━━━\x1b[0m");
        for msg in messages {
            match msg {
                Message::User { content, .. } => {
                    for block in content {
                        if block.r#type == "text" {
                            if let Some(ref text) = block.text {
                                for line in text.lines() {
                                    eprintln!("\x1b[32m❯ {}\x1b[0m", line);
                                }
                            }
                        }
                    }
                }
                Message::Assistant { content, .. } => {
                    for block in content {
                        match block.r#type.as_str() {
                            "text" => {
                                if let Some(ref text) = block.text {
                                    for line in text.lines() {
                                        eprintln!("{}", line);
                                    }
                                }
                            }
                            "tool_use" => {
                                if let Some(ref name) = block.tool_name {
                                    eprintln!("\x1b[2m  [工具调用: {name}]\x1b[0m");
                                }
                            }
                            _ => {}
                        }
                    }
                    eprintln!();
                }
            }
        }
        eprintln!("\x1b[2m━━━━━━━━━━━━━━━\x1b[0m");
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        self.print_banner();

        // --resume：自动恢复最后一次会话
        if self.resume {
            self.resume_last_session().await;
        }

        // Spawn Ctrl+C signal handler for aborting LLM generation
        let sig_notify = self.shutdown_notify.clone();
        let sig_flag = self.cancel_flag.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = sig_notify.notified() => {
                        tracing::info!("signal handler shutting down");
                        break;
                    }
                    result = tokio::signal::ctrl_c() => {
                        match result {
                            Ok(()) => {
                                sig_flag.store(true, Ordering::SeqCst);
                                tracing::info!("Ctrl+C received, cancellation requested");
                            }
                            Err(e) => {
                                tracing::warn!("ctrl+c handler error: {e}");
                                break;
                            }
                        }
                    }
                }
            }
        });

        while self.running {
            match self.rl.readline("\n❯ ") {
                Ok(input) => {
                    let trimmed = input.trim().to_string();
                    if trimmed.is_empty() {
                        continue;
                    }

                    self.rl.add_history_entry(&trimmed)?;

                    if trimmed.starts_with('/') {
                        self.handle_slash(&trimmed).await;
                    } else {
                        self.last_message = Some(trimmed.clone());
                        self.handle_message(&trimmed).await;
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    self.cancel_flag.store(true, Ordering::SeqCst);
                    continue;
                }
                Err(ReadlineError::Eof) => {
                    // Ctrl+D 退出
                    self.running = false;
                }
                Err(e) => {
                    tracing::error!("input error: {e}");
                    eprintln!("\x1b[31m输入错误: {e}\x1b[0m");
                    break;
                }
            }
        }

        // 持久化历史
        if let Some(path) = Self::history_path() {
            let _ = self.rl.save_history(&path);
        }

        self.tools.shutdown_all().await;
        Ok(())
    }

    fn print_banner(&self) {
        let model = "deepseek-v4-flash";
        let skills_info = if self.skills_count > 0 {
            format!("技能: {} 个已加载", self.skills_count)
        } else {
            "技能: 无".to_string()
        };
        let mcp_count = self.tools.mcp_client_count();

        eprintln!("🐾 \x1b[1m千寻 (Qianxun)\x1b[0m — AI 编程助手");
        eprintln!("\x1b[2m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m");
        if !self.workspace_context.is_empty() {
            eprintln!("  工作区上下文已加载");
        }
        eprintln!("  模型: {model}");
        eprintln!("  {skills_info}");
        if mcp_count > 0 {
            eprintln!("  MCP: {mcp_count} 个已连接");
        }
        eprintln!("\x1b[2m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m");
        eprintln!("  输入 /help 查看命令  |  /quit 退出");
    }

    // ─── 命令分发 ───────────────────────────────────────────

    async fn handle_slash(&mut self, cmd: &str) {
        match cmd {
            "/quit" | "/exit" => {
                eprintln!("再见！");
                self.shutdown_notify.notify_one();
                self.running = false;
            }
            "/help" => {
                eprintln!("\x1b[1m\x1b[4m对话控制:\x1b[0m");
                eprintln!("  \x1b[36m/quit\x1b[0m      退出千寻");
                eprintln!("  \x1b[36m/reset\x1b[0m     重置对话（开始新会话）");
                eprintln!("  \x1b[36m/retry\x1b[0m     重新发送上一条消息");
                eprintln!("  \x1b[36m/edit\x1b[0m      编辑上一条消息并重发");
                eprintln!();
                eprintln!("\x1b[1m\x1b[4m会话管理:\x1b[0m");
                eprintln!("  \x1b[36m/sessions\x1b[0m      列出历史会话");
                eprintln!("  \x1b[36m/sessions <id>\x1b[0m  恢复历史会话");
                eprintln!("  \x1b[36m/sessions delete <id>\x1b[0m  删除历史会话");
                eprintln!();
                eprintln!("\x1b[1m\x1b[4m信息查询:\x1b[0m");
                eprintln!("  \x1b[36m/help\x1b[0m      显示此帮助");
                eprintln!("  \x1b[36m/usage\x1b[0m     Token 用量");
                eprintln!("  \x1b[36m/workspace\x1b[0m 工作区信息");
                eprintln!("  \x1b[36m/skills\x1b[0m    已加载技能");
                eprintln!("  \x1b[36m/tools\x1b[0m     可用工具");
                eprintln!("  \x1b[36m/memory\x1b[0m    最近记忆");
            }
            "/reset" => {
                eprint!("确定要重置对话吗？(y/N): ");
                let _ = std::io::stderr().flush();
                let mut confirm = String::new();
                if std::io::stdin().read_line(&mut confirm).is_ok() {
                    let trimmed = confirm.trim().to_lowercase();
                    if trimmed != "y" && trimmed != "yes" {
                        eprintln!("已取消。");
                        return;
                    }
                }
                // 保存当前会话后再重置
                self.save_conversation().await;
                let sys = system_prompt::build_system_prompt(
                    &self.workspace_context, &self.skills_catalog, None,
                );
                self.conversation = Conversation::new(Some(sys));
                self.conversation.set_budget(self.budget.0, self.budget.1);
                self.agent_loop.reset();
                self.recently_injected.clear();
                self.current_session = None;
                eprintln!("对话已重置，开始新会话。");
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
                if self.skills_list.is_empty() {
                    eprintln!("未加载任何技能。");
                } else {
                    eprintln!("已加载的技能: ({})", self.skills_count);
                    eprintln!();
                    eprintln!("{}", self.skills_list);
                    eprintln!("\x1b[2m自动注入: 消息包含触发词时自动注入完整技能指令");
                    eprintln!("手动注入: 在消息中使用 @技能名 手动引用技能\x1b[0m");
                }
            }
            "/tools" => {
                if self.tools_list.is_empty() {
                    eprintln!("无可用工具。");
                } else {
                    eprintln!("可用工具：");
                    eprintln!("{}", self.tools_list);
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
            "/sessions" => {
                let sessions = Self::list_sessions();
                if sessions.is_empty() {
                    eprintln!("无历史会话。");
                } else {
                    eprintln!("历史会话 ({}):", sessions.len());
                    eprintln!();
                    for s in &sessions {
                        let marker = self.current_session.as_deref()
                            .filter(|id| *id == s.id)
                            .map(|_| " ← 当前")
                            .unwrap_or("");
                        let preview = if s.preview.len() > 50 {
                            let end = (0..=50).rev().find(|&i| s.preview.is_char_boundary(i)).unwrap_or(0);
                            &s.preview[..end]
                        } else {
                            &s.preview
                        };
                        eprintln!("  \x1b[36m{}\x1b[0m  {} 条消息  {}{}", s.id, s.message_count, preview, marker);
                    }
                    eprintln!();
                    eprintln!("使用 \x1b[36m/sessions <id>\x1b[0m 恢复会话（支持部分匹配）");
                    eprintln!("使用 \x1b[36m/sessions delete <id>\x1b[0m 删除会话");
                }
            }
            "/retry" => {
                if let Some(last) = &self.last_message.clone() {
                    eprintln!("重试: {last}");
                    self.handle_message(last).await;
                } else {
                    eprintln!("尚无消息可重试。");
                }
            }
            "/edit" => {
                if let Some(last) = &self.last_message.clone() {
                    match self.rl.readline_with_initial("❯ ", (last.as_str(), "")) {
                        Ok(edited) => {
                            let trimmed = edited.trim().to_string();
                            if !trimmed.is_empty() {
                                self.last_message = Some(trimmed.clone());
                                self.handle_message(&trimmed).await;
                            }
                        }
                        Err(ReadlineError::Interrupted) => {
                            eprintln!("已取消。");
                        }
                        Err(e) => {
                            eprintln!("\x1b[31m编辑失败: {e}\x1b[0m");
                        }
                    }
                } else {
                    eprintln!("尚无消息可编辑。");
                }
            }
            _ => {
                // /sessions <id> → 恢复会话
                // /sessions delete <id> → 删除会话
                if cmd.starts_with("/sessions ") {
                    let rest = cmd.trim_start_matches("/sessions ").trim();
                    if !rest.is_empty() {
                        if let Some(delete_id) = rest.strip_prefix("delete ") {
                            self.handle_session_delete(delete_id.trim()).await;
                            return;
                        }
                        self.handle_session_resume(rest).await;
                        return;
                    }
                }

                // 模糊匹配: 唯一前缀匹配时直接执行
                let matches: Vec<&&str> = SLASH_COMMANDS.iter().filter(|c| c.starts_with(cmd)).collect();
                if matches.len() == 1 {
                    eprintln!("  匹配: {}", matches[0]);
                    Box::pin(self.handle_slash(matches[0])).await;
                } else {
                    eprintln!("未知命令: {cmd}");
                }
            }
        }
    }

    // ─── 消息处理 ───────────────────────────────────────────

    async fn handle_message(&mut self, msg: &str) {
        // 重置取消标志，以便上一次取消不影响本轮
        self.cancel_flag.store(false, Ordering::SeqCst);

        // 1. 展开 $FILE:path
        let msg = expand_file_refs(msg, self.ws_root.as_deref());

        // 1b. 自动获取 URL 内容
        let url_context = fetch_urls(&msg).await;
        let full_msg = if url_context.is_empty() {
            msg.clone()
        } else {
            format!("{}\n\n---\n以下是从消息中 URL 自动获取的内容：\n{}", msg, url_context)
        };

        // 2. 提取手动引用 @技能名
        let manual_mentions = SkillManager::extract_manual_mentions(&full_msg);
        let manual_names: Vec<String> = manual_mentions
            .into_iter()
            .filter(|name| self.skill_manager.select_by_name(name).is_some())
            .collect();

        // 3. 构建排除列表
        let exclude: Vec<&str> = {
            let mut e: Vec<&str> = self.recently_injected.iter().map(|s| s.as_str()).collect();
            for name in &manual_names {
                if !e.contains(&name.as_str()) {
                    e.push(name.as_str());
                }
            }
            e
        };

        // 4. 自动匹配
        let auto_names = self.skill_manager.auto_select(&msg, &exclude);

        // 5. 合并待注入技能
        let mut inject_names: Vec<String> = manual_names;
        inject_names.extend(auto_names);

        // 6. 构建技能注入内容（Layer 2）
        let skill_injections = self.skill_manager.build_injections(&inject_names);

        // 7. 记录最近注入
        for name in &inject_names {
            self.recently_injected.push(name.clone());
        }
        if self.recently_injected.len() > 10 {
            self.recently_injected.drain(0..self.recently_injected.len() - 10);
        }

        if !inject_names.is_empty() {
            tracing::info!(
                "[skills] injected {}: {}",
                inject_names.len(),
                inject_names.join(", "),
            );
        }

        // 8. 构建记忆上下文
        let memory_context = self
            .memory_manager
            .as_ref()
            .map(|mm| mm.build_context())
            .unwrap_or_default();

        self.conversation
            .push_user_message(vec![ContentBlock::text(&full_msg)]);

        processing_loop::handle_user_message(
            &mut self.agent_loop,
            &mut self.conversation,
            &*self.provider,
            &self.tools,
            &self.sink,
            &memory_context,
            &self.skills_catalog,
            &skill_injections,
            self.cancel_flag.clone(),
        )
        .await;

        // 轮次后写入记忆
        if let Some(mm) = &self.memory_manager {
            let summary = if msg.len() > 200 { &msg[..200] } else { &msg };
            mm.write_memory(summary, &["conversation"], &msg);
        }

        // 自动保存会话（每次轮次后）
        if self.current_session.is_none() {
            self.current_session = Some(Self::new_session_id());
        }
        self.save_conversation().await;
    }
}

// ─── $FILE:path 展开 ────────────────────────────────────────

/// 将消息中的 `$FILE:path` 替换为文件内容。
fn expand_file_refs(msg: &str, ws_root: Option<&std::path::Path>) -> String {
    let mut result = msg.to_string();
    while let Some(start) = result.find("$FILE:") {
        let after = &result[start + 6..];
        let end = after.find(|c: char| c.is_whitespace()).unwrap_or(after.len());
        let path_str = &after[..end];

        let resolved = resolve_file_path(path_str, ws_root);
        match std::fs::read_to_string(&resolved) {
            Ok(content) => {
                let replacement = format!(
                    "\n```\n// {}\n{}\n```\n",
                    resolved.display(),
                    content,
                );
                result.replace_range(start..start + 6 + end, &replacement);
            }
            Err(e) => {
                let err_msg = format!("\n\x1b[31m[FILE 错误] {} — {e}\x1b[0m\n", resolved.display());
                result.replace_range(start..start + 6 + end, &err_msg);
            }
        }
    }
    result
}

// ─── URL 自动获取 ─────────────────────────────────────────────

/// 检测消息中的 `https?://` URL 并并发获取内容，返回拼接后的上下文块。
async fn fetch_urls(msg: &str) -> String {
    let urls: Vec<&str> = msg
        .split_whitespace()
        .filter(|w| w.starts_with("http://") || w.starts_with("https://"))
        .collect();

    if urls.is_empty() {
        return String::new();
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("qianxun/1.0")
        .build()
        .expect("reqwest Client::builder()");

    let mut results: Vec<(String, String)> = Vec::new();
    for url in urls {
        let url = url.trim_end_matches([')', ']', '>', '"', '\'']);
        match client.get(url).send().await {
            Ok(resp) => {
                if let Ok(text) = resp.text().await {
                    let truncated_len = text.len();
                    let content = if truncated_len > 10_000 {
                        format!("{}…\n[截断, 共 {truncated_len} 字节]", &text[..10_000])
                    } else {
                        text
                    };
                    tracing::debug!("URL fetched: {url} ({truncated_len} bytes)");
                    results.push((url.to_string(), content));
                }
            }
            Err(e) => {
                tracing::debug!("URL fetch failed: {url} — {e}");
            }
        }
    }

    if results.is_empty() {
        return String::new();
    }

    results
        .into_iter()
        .map(|(url, content)| format!("<url>\n<source>{url}</source>\n{content}\n</url>"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 会话摘要信息
struct SessionInfo {
    id: String,
    created_at: String,
    message_count: usize,
    preview: String,
}

fn resolve_file_path(path_str: &str, ws_root: Option<&std::path::Path>) -> PathBuf {
    let p = std::path::Path::new(path_str);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    if path_str.starts_with("./") || path_str.starts_with(".\\") {
        // 相对 cwd
        return std::env::current_dir().unwrap_or_default().join(path_str);
    }
    // 优先相对 workspace root
    if let Some(root) = ws_root {
        let from_ws = root.join(path_str);
        if from_ws.exists() {
            return from_ws;
        }
    }
    // 回退到 cwd
    std::env::current_dir().unwrap_or_default().join(path_str)
}
