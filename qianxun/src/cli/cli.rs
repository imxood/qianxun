// CLI REPL: 旧 cli 路径, TUI/daemon 模式上线后多数 helper/sink 暂未调用, 留 Phase 4 迁移后清理.
#![allow(dead_code, unused_imports)]

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::{AgentLoop, processing_loop};
use qianxun_core::agent::message::{ContentBlock, Message};
use qianxun_core::agent::system_prompt;
use qianxun_core::provider::LlmProvider;
use qianxun_core::skills::SkillManager;
use qianxun_core::skills::SkillWatcher;
use qianxun_core::tools::{ToolCategoryFilter, ToolRegistry};
use qianxun_core::types::Mode;

use crate::cli::output::CliOutputSink;

// ─── ANSI 颜色助手（console 替代）────────────────────────
use std::fmt::Display;
struct Style(usize);
fn style<D: Display>(d: D) -> AnsiStyled<D> {
    AnsiStyled(d, Vec::new())
}
struct AnsiStyled<D>(D, Vec<&'static str>);
impl<D: Display> AnsiStyled<D> {
    fn apply(self, code: &'static str) -> Self {
        let mut v = self.1;
        v.push(code);
        AnsiStyled(self.0, v)
    }
    fn cyan(self) -> Self {
        self.apply("36")
    }
    fn green(self) -> Self {
        self.apply("32")
    }
    fn red(self) -> Self {
        self.apply("31")
    }
    fn yellow(self) -> Self {
        self.apply("33")
    }
    fn dim(self) -> Self {
        self.apply("2")
    }
    fn bold(self) -> Self {
        self.apply("1")
    }
    fn color256(self, _c: u8) -> Self {
        self.apply("90")
    }
}
impl<D: Display> Display for AnsiStyled<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.1.is_empty() {
            return write!(f, "{}", self.0);
        }
        let codes = self.1.join(";");
        write!(f, "\x1b[{codes}m{}\x1b[0m", self.0)
    }
}

const HORIZ: &str = "─";
const TOP_L: &str = "╭";
const BOT_L: &str = "╰";
const VERT: &str = "│";

const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/quit",
    "/exit",
    "/reset",
    "/usage",
    "/workspace",
    "/skills",
    "/tools",
    "/memory",
    "/retry",
    "/edit",
    "/sessions",
    "/mode",
    "/plan",
];

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
    memory: Option<Box<dyn qianxun_core::context::MemoryObserver + Send>>,
    skill_manager: SkillManager,
    skill_watcher: SkillWatcher,
    skills_catalog: String,
    skills_list: String,
    skills_count: usize,
    tools_list: String,
    recently_injected: Vec<String>,
    last_message: Option<String>,
    cancel_flag: Arc<AtomicBool>,
    shutdown_notify: Arc<tokio::sync::Notify>,

    ws_root: Option<PathBuf>,
    sessions_dir: Option<PathBuf>,
    current_session: Option<String>,
    resume: bool,
    mode: Mode,
    global_instructions: Option<String>,
}

impl Repl {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent_loop: AgentLoop,
        conversation: Conversation,
        provider: Box<dyn LlmProvider>,
        tools: ToolRegistry,
        workspace_context: String,
        memory: Option<Box<dyn qianxun_core::context::MemoryObserver + Send>>,
        skill_manager: SkillManager,
        skill_watcher: SkillWatcher,
        skills_catalog: String,
        skills_list: String,
        skills_count: usize,
        tools_list: String,
        ws_root: Option<PathBuf>,
        resume: bool,
        global_instructions: Option<String>,
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
            sink: CliOutputSink::new(),
            budget,
            workspace_context,
            memory,
            skill_manager,
            skill_watcher,
            skills_catalog,
            skills_list,
            skills_count,
            tools_list,
            recently_injected: Vec::new(),
            last_message: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),

            ws_root,
            sessions_dir: Self::sessions_dir(),
            current_session: None,
            resume,
            mode: Mode::Auto,
            global_instructions,
        }
    }

    /// 历史文件路径: ~/.qianxun/history.txt
    /// 会话目录: ~/.qianxun/sessions/
    fn sessions_dir() -> Option<PathBuf> {
        let dir = qianxun_core::workspace::qianxun_dir()?.join("sessions");
        let _ = std::fs::create_dir_all(&dir);
        Some(dir)
    }

    /// 保存当前会话到 JSONL + meta 文件
    async fn save_conversation(&self) {
        let dir = match &self.sessions_dir {
            Some(d) => d.clone(),
            None => return,
        };
        let Some(session_id) = self.current_session.as_deref() else {
            return;
        };
        let jsonl_path = dir.join(format!("{session_id}.jsonl"));
        let meta_path = dir.join(format!("{session_id}.meta"));

        if let Err(e) = self.conversation.save_to(&jsonl_path).await {
            tracing::warn!("保存会话失败: {e}");
            return;
        }

        // 更新 meta 文件
        let msg_count = self.conversation.messages().len();
        let preview = self
            .conversation
            .messages()
            .first()
            .and_then(|m| {
                m.content()
                    .first()
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
        let _ = std::fs::write(
            &meta_path,
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        );
    }

    /// 列出所有会话
    fn list_sessions() -> Vec<SessionInfo> {
        let dir = match Self::sessions_dir() {
            Some(d) => d,
            None => return Vec::new(),
        };

        let mut sessions = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return sessions;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
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

            sessions.push(SessionInfo {
                id,
                created_at,
                message_count,
                preview,
            });
        }

        sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        sessions
    }

    /// 创建新会话 ID（基于当前时间）
    fn new_session_id() -> String {
        chrono::Local::now().format("%Y%m%d_%H%M%S").to_string()
    }

    /// 恢复历史会话（模糊匹配 ID）
    async fn handle_session_resume(&mut self, partial: &str) {
        let sessions = Self::list_sessions();
        let matches: Vec<&SessionInfo> =
            sessions.iter().filter(|s| s.id.contains(partial)).collect();

        if matches.is_empty() {
            eprintln!("未找到包含 \"{partial}\" 的会话。\n使用 /sessions 查看可用的历史会话。");
            return;
        }
        if matches.len() > 1 {
            eprintln!("\"{partial}\" 匹配了多个会话：");
            for s in &matches {
                eprintln!(
                    "  {}  {} 条消息  {}",
                    style(&s.id).cyan(),
                    s.message_count,
                    s.preview
                );
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
                eprintln!(
                    "已恢复会话: {} ({} 条消息)",
                    style(&session.id).cyan(),
                    session.message_count
                );
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
        let matches: Vec<&SessionInfo> =
            sessions.iter().filter(|s| s.id.contains(partial)).collect();

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

        eprintln!("已删除会话: {}", style(&session.id).cyan());
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
                    "{}",
                    style(format!("已恢复会话: {} ({} 条消息)", latest.id, msg_count)).green(),
                );
                Self::print_conversation(&self.conversation);
            }
            Err(e) => {
                eprintln!("{}", style(format!("恢复会话失败: {e}")).yellow());
            }
        }
    }

    /// 打印会话中的消息文本（用于恢复时展示历史）
    fn print_conversation(conv: &Conversation) {
        let messages = conv.messages();
        if messages.is_empty() {
            return;
        }
        let msg_count = messages.len();
        let mut content_lines: Vec<String> = Vec::new();

        for msg in messages {
            match msg {
                Message::User { content, .. } => {
                    for block in content {
                        if block.r#type == "text" {
                            if let Some(ref text) = block.text {
                                for line in text.lines() {
                                    content_lines
                                        .push(style(format!("❯ {line}")).color256(34).to_string());
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
                                        content_lines.push(line.to_string());
                                    }
                                }
                            }
                            "tool_use" => {
                                if let Some(ref name) = block.tool_name {
                                    content_lines
                                        .push(style(format!("  [工具: {name}]")).dim().to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                    content_lines.push(String::new());
                }
            }
        }

        // 计算框线宽度
        let max_w = content_lines
            .iter()
            .map(|l| {
                let mut len = 0;
                let mut in_esc = false;
                for ch in l.chars() {
                    if ch == '\x1b' {
                        in_esc = true;
                    } else if in_esc {
                        if ch == 'm' {
                            in_esc = false;
                        }
                    } else {
                        len += 1;
                    }
                }
                len
            })
            .fold(0, usize::max);
        let inner_w = max_w.clamp(20, 80);

        let title = format!(" 历史消息 ({msg_count} 条) ");
        eprintln!(
            "{}╭{}{}",
            style("").color256(236),
            title,
            style(HORIZ.repeat(inner_w.saturating_sub(title.chars().count()) + 2)).color256(236),
        );

        for line in &content_lines {
            eprintln!("{} {}", style(VERT).color256(236), line);
        }

        eprintln!(
            "{}{}",
            style(BOT_L).color256(236),
            style(HORIZ.repeat(inner_w + 2)).color256(236)
        );
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        // 启动时清屏（使用 console crate 跨平台支持）

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
            let _turn_count = self.agent_loop.turn_count;
            // TUI 接管事件循环
            let mut input = String::new();
            print!("❯ ");
            let _ = std::io::stdout().flush();
            if std::io::stdin().read_line(&mut input).is_ok() {
                let trimmed = input.trim().to_string();
                if !trimmed.is_empty() {
                    if trimmed.starts_with('/') {
                        self.handle_slash(&trimmed).await;
                    } else {
                        self.last_message = Some(trimmed.clone());
                        self.handle_message(&trimmed).await;
                    }
                }
            }
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
        let ws_info = if self.workspace_context.is_empty() {
            String::new()
        } else {
            "  工作区上下文已加载".to_string()
        };

        let mut lines: Vec<String> = Vec::new();
        lines.push(format!(
            " {} {} — AI 编程助手",
            style("千寻 (Qianxun)").bold(),
            style("").color256(244),
        ));
        lines.push(String::new());
        if !ws_info.is_empty() {
            lines.push(format!("  {}", style(ws_info).dim()));
        }
        lines.push(format!("  模型: {model}"));
        lines.push(format!("  {skills_info}"));
        if mcp_count > 0 {
            lines.push(format!("  MCP: {mcp_count} 个已连接"));
        }
        lines.push(String::new());
        lines.push(format!(
            "  {}  {}  {}",
            style("输入 /help 查看命令").color256(244),
            style("|").dim(),
            style("/quit 退出").color256(244),
        ));

        let block = self.boxed_banner(&lines);
        eprint!("{block}");
    }

    /// 框线横幅（与 output.rs 风格一致，但横幅无顶线）
    fn boxed_banner(&self, lines: &[String]) -> String {
        let max_w = lines.iter().map(|l| l.len()).fold(0, usize::max);
        let inner_w = max_w.max(20);
        let mut out = String::new();

        out.push_str(&format!(
            "{}{}\n",
            style(TOP_L).color256(236),
            style(HORIZ.repeat(inner_w + 2)).color256(236),
        ));

        for line in lines {
            out.push_str(&format!("{} {line}\n", style(VERT).color256(236),));
        }

        out.push_str(&format!(
            "{}{}\n",
            style(BOT_L).color256(236),
            style(HORIZ.repeat(inner_w + 2)).color256(236),
        ));
        out
    }

    /// 检查技能目录是否有文件变更，有则自动重载。
    fn check_skill_reload(&mut self, source: &str) {
        if !self.skill_watcher.has_changed() {
            return;
        }
        tracing::info!("[skill_watcher] file change detected ({source}), reloading skills");
        self.skill_manager.reload(self.ws_root.as_deref());
        self.skills_catalog = self.skill_manager.build_catalog_prompt();
        self.skills_list = self.skill_manager.build_skills_list();
        self.skills_count = self.skill_manager.skill_count();
    }

    // ─── 命令分发 ───────────────────────────────────────────

    /// cmd 是原始输入（如 "/mode plan"），base 是匹配到的斜杠命令（如 "/mode"）
    /// 处理子命令时用 cmd 提取子命令参数
    async fn handle_slash(&mut self, cmd: &str) {
        // 提取基础命令名（/mode plan → /mode）
        let base = cmd.split_whitespace().next().unwrap_or(cmd);
        match base {
            "/quit" | "/exit" => {
                self.save_conversation().await;
                eprintln!("再见！");
                self.shutdown_notify.notify_one();
                self.running = false;
            }
            "/help" => {
                eprintln!("# 帮助\n");

                let sections: &[(&str, &[(&str, &str)])] = &[
                    (
                        "基本",
                        &[
                            ("/mode [plan | auto]", "切换计划/自动模式"),
                            ("/plan", "切换到计划模式（/mode plan 的快捷方式）"),
                            ("/quit | /exit", "退出千寻"),
                            ("/help", "显示此帮助"),
                        ],
                    ),
                    (
                        "对话",
                        &[
                            ("/reset", "重置对话"),
                            ("/retry", "重新发送上一条消息"),
                            ("/edit", "编辑上一条消息并重发"),
                        ],
                    ),
                    (
                        "会话",
                        &[
                            ("/sessions", "列出历史会话"),
                            ("/sessions <id>", "恢复历史会话"),
                            ("/sessions delete <id>", "删除会话"),
                        ],
                    ),
                    (
                        "信息",
                        &[
                            ("/usage", "Token 用量"),
                            ("/workspace", "工作区信息"),
                            ("/skills", "已加载技能"),
                            ("/tools", "可用工具"),
                            ("/memory", "最近记忆"),
                        ],
                    ),
                ];

                for (sec_title, items) in sections {
                    eprintln!("## {sec_title}");
                    for (cmd, desc) in *items {
                        eprintln!("  {cmd}  {desc}");
                    }
                    eprintln!();
                }
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
                let mode = self.mode.display_name();
                let sys = system_prompt::build_system_prompt(
                    &self.workspace_context,
                    self.global_instructions.as_deref(),
                    mode,
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
                let b = |s: &str| style(s).color256(236).to_string();
                eprintln!("{}╭ Token 用量 {}", b(""), b(&HORIZ.repeat(22)));
                eprintln!("{}  输入 token: {}", b(VERT), usage.input);
                eprintln!("{}  输出 token: {}", b(VERT), usage.output);
                if let Some(cc) = usage.cache_creation_input {
                    eprintln!("{}  缓存创建: {cc}", b(VERT));
                }
                if let Some(cr) = usage.cache_read_input {
                    eprintln!("{}  缓存读取: {cr}", b(VERT));
                }
                eprintln!("{}{}", b(BOT_L), b(&HORIZ.repeat(24)));
            }
            "/skills" => {
                self.check_skill_reload("handle_slash");
                let b = |s: &str| style(s).color256(236).to_string();
                if self.skills_list.is_empty() {
                    eprintln!("{}╭ 技能 {}", b(""), b(&HORIZ.repeat(18)));
                    eprintln!("{}  未加载任何技能。", b(VERT));
                    eprintln!("{}{}", b(BOT_L), b(&HORIZ.repeat(22)));
                } else {
                    let title = format!(" 已加载的技能 ({}) ", self.skills_count);
                    eprintln!(
                        "{}╭{}{}",
                        b(""),
                        title,
                        b(&HORIZ.repeat(72usize.saturating_sub(title.chars().count()) + 2))
                    );
                    for line in self.skills_list.lines() {
                        eprintln!("{}  {line}", b(VERT));
                    }
                    eprintln!("{}", b(VERT));
                    eprintln!(
                        "{}  {}",
                        b(VERT),
                        style("自动注入: 消息包含触发词时自动注入完整技能指令").dim()
                    );
                    eprintln!(
                        "{}  {}",
                        b(VERT),
                        style("手动注入: 在消息中使用 @技能名 手动引用技能").dim()
                    );
                    eprintln!("{}{}", b(BOT_L), b(&HORIZ.repeat(74)));
                }
            }
            "/tools" => {
                let b = |s: &str| style(s).color256(236).to_string();
                if self.tools_list.is_empty() {
                    eprintln!("{}╭ 工具 {}", b(""), b(&HORIZ.repeat(18)));
                    eprintln!("{}  无可用工具。", b(VERT));
                    eprintln!("{}{}", b(BOT_L), b(&HORIZ.repeat(22)));
                } else {
                    eprintln!("{}╭ 可用工具 {}", b(""), b(&HORIZ.repeat(64)));
                    for line in self.tools_list.lines() {
                        eprintln!("{}  {line}", b(VERT));
                    }
                    eprintln!("{}{}", b(BOT_L), b(&HORIZ.repeat(74)));
                }
            }
            "/memory" => match &self.memory {
                Some(m) => {
                    let ctx = m.build_context("", 1000).await;
                    if ctx.is_empty() {
                        eprintln!("尚无记忆。");
                    } else {
                        eprintln!("最近记忆：\n{ctx}");
                    }
                }
                None => {
                    eprintln!("未启用记忆。");
                }
            },
            "/sessions" => {
                let sessions = Self::list_sessions();
                let b = |s: &str| style(s).color256(236).to_string();
                if sessions.is_empty() {
                    eprintln!("{}╭ 历史会话 {}", b(""), b(&HORIZ.repeat(18)));
                    eprintln!("{}  无历史会话。", b(VERT));
                    eprintln!("{}{}", b(BOT_L), b(&HORIZ.repeat(22)));
                } else {
                    eprintln!(
                        "{}╭ 历史会话 ({}) {}",
                        b(""),
                        sessions.len(),
                        b(&HORIZ.repeat(56))
                    );
                    eprintln!("{}", b(VERT));
                    for s in &sessions {
                        let marker = self
                            .current_session
                            .as_deref()
                            .filter(|id| *id == s.id)
                            .map(|_| " ← 当前")
                            .unwrap_or("");
                        let preview = if s.preview.len() > 40 {
                            let end = (0..=40)
                                .rev()
                                .find(|&i| s.preview.is_char_boundary(i))
                                .unwrap_or(0);
                            &s.preview[..end]
                        } else {
                            &s.preview
                        };
                        eprintln!(
                            "{}  {}  {} 条  {preview}{marker}",
                            b(VERT),
                            style(&s.id).color256(75),
                            s.message_count
                        );
                    }
                    eprintln!("{}", b(VERT));
                    eprintln!(
                        "{}  使用 {} 恢复  |  {} 删除",
                        b(VERT),
                        style("/sessions <id>").color256(75),
                        style("/sessions delete <id>").color256(75)
                    );
                    eprintln!("{}{}", b(BOT_L), b(&HORIZ.repeat(60)));
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
                if let Some(last) = self.last_message.clone() {
                    eprintln!("编辑（请输入新内容）:");
                    eprintln!("原消息: {last}");
                    let mut edited = String::new();
                    if std::io::stdin().read_line(&mut edited).is_ok() {
                        let trimmed = edited.trim().to_string();
                        if !trimmed.is_empty() {
                            self.last_message = Some(trimmed.clone());
                            self.handle_message(&trimmed).await;
                        }
                    }
                } else {
                    eprintln!("尚无消息可编辑。");
                }
            }
            "/mode" => {
                let sub = cmd.strip_prefix("/mode").map(|s| s.trim()).unwrap_or("");
                match sub {
                    "plan" => {
                        self.mode = Mode::Plan;
                        eprintln!(
                            "  切换到 {} — 仅允许读取类工具（Read / Search / Think）",
                            style("计划模式").cyan(),
                        );
                    }
                    "auto" => {
                        self.mode = Mode::Auto;
                        eprintln!("  切换到 {} — 所有工具可用", style("自动模式").green(),);
                    }
                    "" => {
                        eprintln!(
                            "  当前模式: {}  — {}",
                            style(self.mode.display_name()).cyan(),
                            match self.mode {
                                Mode::Auto => "所有工具可用",
                                Mode::Plan => "仅允许读取类工具（Read / Search / Think）",
                            },
                        );
                        eprintln!("  用法: {} [plan | auto]", style("/mode").cyan());
                    }
                    _ => {
                        eprintln!(
                            "  未知模式: {}。用法: {}",
                            style(sub).red(),
                            style("/mode [plan | auto]").cyan(),
                        );
                    }
                }
            }
            "/plan" => {
                // /plan 是 /mode plan 的快捷方式，/plan auto 回到自动模式
                let sub = cmd.strip_prefix("/plan").map(|s| s.trim()).unwrap_or("");
                match sub {
                    "auto" => {
                        self.mode = Mode::Auto;
                        eprintln!("  切换到 {} — 所有工具可用", style("自动模式").green());
                    }
                    _ => {
                        // 包括 "plan" 和任何其他参数: 静默回退到 Plan 模式
                        // (未知参数不影响主行为, 简化 REPL 体验)
                        self.mode = Mode::Plan;
                        eprintln!(
                            "  切换到 {} — 仅允许读取类工具（Read / Search / Think）",
                            style("计划模式").cyan()
                        );
                    }
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

                // 子命令前缀匹配（例如 /h → /help）
                let matches: Vec<&&str> = SLASH_COMMANDS
                    .iter()
                    .filter(|c| c.starts_with(cmd))
                    .collect();
                if matches.len() == 1 {
                    eprintln!("  匹配: {}", matches[0]);
                    Box::pin(self.handle_slash(matches[0])).await;
                    return;
                }

                // 基础命令匹配（例如 /mode plan → /mode）
                let _base = cmd.split_whitespace().next().unwrap_or(cmd);
                let base_matches: Vec<&&str> = SLASH_COMMANDS
                    .iter()
                    .filter(|c| cmd.starts_with(*c))
                    .collect();
                if base_matches.len() == 1 {
                    Box::pin(self.handle_slash(cmd)).await;
                    return;
                }

                eprintln!("未知命令: {cmd}");
            }
        }
    }

    // ─── 消息处理 ───────────────────────────────────────────

    fn mode_to_filter(&self) -> ToolCategoryFilter {
        match self.mode {
            Mode::Auto => ToolCategoryFilter::all(),
            Mode::Plan => ToolCategoryFilter::read_only(),
        }
    }

    async fn handle_message(&mut self, msg: &str) {
        // 检查技能文件变更
        self.check_skill_reload("handle_message");

        // 重置取消标志，以便上一次取消不影响本轮
        self.cancel_flag.store(false, Ordering::SeqCst);

        // 1. 展开 $FILE:path
        let msg = expand_file_refs(msg, self.ws_root.as_deref());

        // 1b. 自动获取 URL 内容
        let url_context = fetch_urls(&msg).await;
        let full_msg = if url_context.is_empty() {
            msg.clone()
        } else {
            format!(
                "{}\n\n---\n以下是从消息中 URL 自动获取的内容：\n{}",
                msg, url_context
            )
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
            self.recently_injected
                .drain(0..self.recently_injected.len() - 10);
        }

        if !inject_names.is_empty() {
            tracing::info!(
                "[skills] injected {}: {}",
                inject_names.len(),
                inject_names.join(", "),
            );
        }

        // 8. 构建记忆上下文
        let memory_context = match &self.memory {
            Some(m) => m.build_context("", 1000).await,
            None => String::new(),
        };

        self.conversation
            .push_user_message(vec![ContentBlock::text(&full_msg)]);

        // ── LLM 生成 ──

        let tool_filter = self.mode_to_filter();
        processing_loop::handle_user_message(
            &mut self.agent_loop,
            &mut self.conversation,
            &*self.provider,
            &self.tools,
            tool_filter,
            &self.sink,
            &memory_context,
            &self.skills_catalog,
            &skill_injections,
            self.cancel_flag.clone(),
            None,
        )
        .await;

        // 轮次后写入记忆
        if let Some(m) = &self.memory {
            let summary = if msg.len() > 200 {
                let end = (0..=200)
                    .rev()
                    .find(|&i| msg.is_char_boundary(i))
                    .unwrap_or(0);
                &msg[..end]
            } else {
                &msg
            };
            let _ = m.remember(summary, "conversation").await;
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
        let end = after
            .find(|c: char| c.is_whitespace())
            .unwrap_or(after.len());
        let path_str = &after[..end];

        let resolved = resolve_file_path(path_str, ws_root);
        match std::fs::read_to_string(&resolved) {
            Ok(content) => {
                let replacement = format!("\n```\n// {}\n{}\n```\n", resolved.display(), content,);
                result.replace_range(start..start + 6 + end, &replacement);
            }
            Err(e) => {
                let styled = style(format!("[FILE 错误] {} — {e}", resolved.display())).red();
                let err_msg = format!("\n{}\n", styled);
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
