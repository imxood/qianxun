use async_trait::async_trait;
use qianxun_core::agent::context::window::AutoCompactWindow;
use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::{AgentLoop, processing_loop};
use qianxun_core::agent::message::ContentBlock;
use qianxun_core::agent::system_prompt;
use qianxun_core::config::{ResolvedCompactionConfig, ResolvedConfig};
use qianxun_core::context::MemoryObserver;
use qianxun_core::output::OutputSink;
use qianxun_core::provider::LlmProvider;
use qianxun_core::skills::{SkillManager, SkillWatcher};
use qianxun_core::tools::{ToolRegistry, builtin};
use qianxun_core::types::{LlmError, Mode, StopReason, TokenUsage};
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Widget, Wrap};
use ratatui::{Frame, Terminal, TerminalOptions, Viewport};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

const MAX_MESSAGES: usize = 400;
const PALETTE_MAX_ROWS: usize = 9;
const INLINE_VIEWPORT_HEIGHT: u16 = 8;

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "/help",
        desc: "显示帮助",
    },
    CommandSpec {
        name: "/quit",
        desc: "退出千寻",
    },
    CommandSpec {
        name: "/exit",
        desc: "退出千寻",
    },
    CommandSpec {
        name: "/reset",
        desc: "重置对话",
    },
    CommandSpec {
        name: "/usage",
        desc: "Token 用量",
    },
    CommandSpec {
        name: "/workspace",
        desc: "工作区信息",
    },
    CommandSpec {
        name: "/skills",
        desc: "已加载技能",
    },
    CommandSpec {
        name: "/tools",
        desc: "可用工具",
    },
    CommandSpec {
        name: "/memory",
        desc: "最近记忆",
    },
    CommandSpec {
        name: "/retry",
        desc: "重试上一条消息",
    },
    CommandSpec {
        name: "/edit",
        desc: "编辑上一条消息",
    },
    CommandSpec {
        name: "/mode",
        desc: "查看或切换模式",
    },
    CommandSpec {
        name: "/plan",
        desc: "切换计划模式",
    },
];

#[derive(Clone, Copy)]
struct CommandSpec {
    name: &'static str,
    desc: &'static str,
}

pub async fn run(
    config: ResolvedConfig,
    project_root: Option<qianxun_core::workspace::ProjectRoot>,
    global_instructions: Option<String>,
) -> anyhow::Result<()> {
    let mut app = App::new(config, project_root, global_instructions).await?;
    let mut terminal = ratatui::try_init_with_options(TerminalOptions {
        viewport: Viewport::Inline(INLINE_VIEWPORT_HEIGHT),
    })?;
    let result = app.run(&mut terminal).await;
    ratatui::try_restore()?;
    result
}

struct App {
    input: Input,
    messages: Vec<UiMessage>,
    status: String,
    mode: Mode,
    running: bool,
    agent_running: bool,
    scrollback_cursor: usize,
    command_palette: CommandPalette,
    visible_command_rows: usize,
    modal: Option<Modal>,
    provider: Arc<dyn LlmProvider>,
    tools: ToolRegistry,
    agent_loop: Option<AgentLoop>,
    conversation: Option<Conversation>,
    memory: Option<Box<dyn MemoryObserver + Send>>,
    skills: SkillManager,
    skill_watcher: SkillWatcher,
    skills_catalog: String,
    skills_list: String,
    tools_list: String,
    recently_injected: Vec<String>,
    last_message: Option<String>,
    queued_messages: VecDeque<String>,
    cancel_flag: Arc<AtomicBool>,
    tx: mpsc::UnboundedSender<AgentEvent>,
    rx: mpsc::UnboundedReceiver<AgentEvent>,
    workspace_context: String,
    ws_root: Option<PathBuf>,
    global_instructions: Option<String>,
    budget: (Option<u64>, Option<u64>),
    last_tick: Instant,
    spinner_index: usize,
}

impl App {
    async fn new(
        config: ResolvedConfig,
        project_root: Option<qianxun_core::workspace::ProjectRoot>,
        global_instructions: Option<String>,
    ) -> anyhow::Result<Self> {
        let workspace_context = project_root
            .as_ref()
            .map(qianxun_core::workspace::build_project_context)
            .unwrap_or_default();
        let ws_root = project_root.as_ref().map(|w| w.root.clone());

        let skills = SkillManager::load_all(ws_root.as_deref());
        let skills_catalog = skills.build_catalog_prompt();
        let skills_list = skills.build_skills_list();
        let skill_watcher = SkillWatcher::new(ws_root.as_deref());

        let mut tools = ToolRegistry::new();
        builtin::register_all(&mut tools);
        tools.register_builtin(Arc::new(builtin::SkillReadTool {
            manager: Arc::new(skills.clone()),
        }));
        connect_workspace_mcp(ws_root.as_deref(), &mut tools).await;
        let tools_list = tools.format_tools_list();

        let mut agent_loop = new_agent_loop(config.agent.clone(), &Some(config.compaction.clone()));
        let system = system_prompt::build_system_prompt(
            &workspace_context,
            global_instructions.as_deref(),
            Mode::Auto.display_name(),
        );
        let mut conversation = Conversation::new(Some(system));
        conversation.set_budget(
            config.budget.max_input_tokens,
            config.budget.max_output_tokens,
        );
        agent_loop.config.max_tokens = config.budget.max_output_tokens;

        let memory = build_memory();
        let provider: Arc<dyn LlmProvider> =
            qianxun_core::provider::create_provider(&config.deepseek).into();
        let (tx, rx) = mpsc::unbounded_channel();
        let status = format!(
            "就绪 | 模式: {} | 技能: {} | 工具: {}",
            Mode::Auto.display_name(),
            skills.skill_count(),
            tools.definitions().len(),
        );

        Ok(Self {
            input: Input::default(),
            messages: vec![UiMessage::system(
                "千寻 TUI 已启动. 输入消息后按 Enter 发送, 输入 / 打开命令面板.",
            )],
            status,
            mode: Mode::Auto,
            running: true,
            agent_running: false,
            scrollback_cursor: 0,
            command_palette: CommandPalette::default(),
            visible_command_rows: 0,
            modal: None,
            provider,
            tools,
            agent_loop: Some(agent_loop),
            conversation: Some(conversation),
            memory,
            skills,
            skill_watcher,
            skills_catalog,
            skills_list,
            tools_list,
            recently_injected: Vec::new(),
            last_message: None,
            queued_messages: VecDeque::new(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            workspace_context,
            ws_root,
            global_instructions,
            budget: (
                config.budget.max_input_tokens,
                config.budget.max_output_tokens,
            ),
            last_tick: Instant::now(),
            spinner_index: 0,
        })
    }

    async fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()> {
        while self.running {
            self.drain_agent_events().await;
            self.flush_completed_messages_to_scrollback(terminal)?;
            if self.last_tick.elapsed() >= Duration::from_millis(120) {
                self.last_tick = Instant::now();
                if self.agent_running {
                    self.spinner_index = (self.spinner_index + 1) % 4;
                }
            }

            terminal.draw(|frame| self.render(frame))?;

            if event::poll(Duration::from_millis(30))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.handle_key(key).await;
                    }
                    Event::Resize(_, _) => {}
                    _ => {}
                }
            }
        }

        self.command_palette.visible = false;
        self.command_palette.selected = 0;
        self.modal = None;
        terminal.clear()?;

        if self.agent_running {
            self.cancel_flag.store(true, Ordering::SeqCst);
        }
        self.tools.shutdown_all().await;
        Ok(())
    }

    fn render(&mut self, frame: &mut Frame) {
        self.visible_command_rows = 0;
        let area = frame.area();
        let live_lines = self.live_message_lines();
        let live_height = (live_lines.len() as u16).min(area.height.saturating_sub(2));
        let body = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: live_height,
        };
        let input = Rect {
            x: area.x,
            y: area.y.saturating_add(live_height),
            width: area.width,
            height: area.height.saturating_sub(live_height).min(1),
        };
        let footer = Rect {
            x: area.x,
            y: input.y.saturating_add(input.height),
            width: area.width,
            height: area
                .height
                .saturating_sub(live_height)
                .saturating_sub(input.height)
                .min(1),
        };

        self.render_messages(frame, body, live_lines);
        self.render_input(frame, input);
        self.render_footer(frame, footer);

        if self.command_palette.visible {
            self.render_command_palette(frame, input);
        }
        if let Some(modal) = &self.modal {
            render_modal(frame, modal);
        }
    }

    fn render_messages(&mut self, frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
        if area.height == 0 {
            return;
        }
        let scroll = lines.len().saturating_sub(area.height as usize) as u16;
        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));
        frame.render_widget(paragraph, area);
    }

    fn flush_completed_messages_to_scrollback<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<(), B::Error> {
        let end = self.completed_scrollback_end();
        if self.scrollback_cursor >= end {
            return Ok(());
        }

        let lines = message_lines_for(&self.messages[self.scrollback_cursor..end]);
        let height = lines.len().min(u16::MAX as usize) as u16;
        if height == 0 {
            self.scrollback_cursor = end;
            return Ok(());
        }

        terminal.insert_before(height, |buf| {
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .render(buf.area, buf);
        })?;
        self.scrollback_cursor = end;
        Ok(())
    }

    fn completed_scrollback_end(&self) -> usize {
        if self.agent_running
            && self
                .messages
                .last()
                .is_some_and(|message| message.role == UiRole::Assistant && message.title == "回复")
        {
            self.messages.len().saturating_sub(1)
        } else {
            self.messages.len()
        }
    }

    fn live_message_lines(&self) -> Vec<Line<'static>> {
        message_lines_for(&self.messages[self.scrollback_cursor..])
    }

    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let width = area.width.saturating_sub(2) as usize;
        let scroll = self.input.visual_scroll(width);
        let visible = self.input.value().chars().skip(scroll).collect::<String>();
        let line = if visible.is_empty() {
            Line::from(vec![
                Span::styled("› ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    "给千寻发送消息, 或输入 / 查看命令",
                    Style::default().fg(Color::DarkGray),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled("› ", Style::default().fg(Color::Cyan)),
                Span::raw(visible),
            ])
        };
        let widget = Paragraph::new(line);
        frame.render_widget(widget, area);

        if self.modal.is_none() {
            let cursor_x = area
                .x
                .saturating_add(2)
                .saturating_add(self.input.visual_cursor().saturating_sub(scroll) as u16)
                .min(area.right().saturating_sub(1));
            frame.set_cursor_position((cursor_x, area.y));
        }
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let spinner = ["-", "\\", "|", "/"][self.spinner_index];
        let queue = (!self.queued_messages.is_empty())
            .then(|| format!("已排队 {}", self.queued_messages.len()));
        let text = if self.command_palette.visible {
            Some("↑↓ 选择 | Tab 补全 | Enter 执行 | Esc 关闭".to_string())
        } else if self.agent_running {
            Some(match queue {
                Some(queue) => format!("{spinner} 运行中 | {queue}"),
                None => format!("{spinner} 运行中"),
            })
        } else if let Some(queue) = queue {
            Some(queue)
        } else if self.status != "就绪" {
            Some(self.status.clone())
        } else {
            None
        };
        let line = Line::from(Span::styled(
            text.unwrap_or_default(),
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(Paragraph::new(line), area);
    }

    fn render_command_palette(&mut self, frame: &mut Frame, input_area: Rect) {
        let filtered = self.command_palette.filtered(self.input.value());
        let desired_height = filtered.len().min(PALETTE_MAX_ROWS).max(1) as u16;
        let frame_area = frame.area();
        let (y, height) = if input_area.y > frame_area.y {
            let height = desired_height.min(input_area.y - frame_area.y);
            (input_area.y - height, height)
        } else {
            let y = input_area.bottom();
            let height = desired_height.min(frame_area.bottom().saturating_sub(y));
            (y, height)
        };
        if height == 0 {
            return;
        }
        self.visible_command_rows = filtered.len().min(height as usize);
        self.command_palette
            .clamp_selection_to_len(self.visible_command_rows);
        let width = input_area.width.min(56);
        let area = Rect {
            x: input_area.x,
            y,
            width,
            height,
        };
        let items: Vec<ListItem> = if filtered.is_empty() {
            vec![ListItem::new(Line::from("无匹配命令"))]
        } else {
            filtered
                .iter()
                .take(self.visible_command_rows)
                .enumerate()
                .map(|(row, idx)| {
                    let cmd = COMMANDS[*idx];
                    let marker = if row == self.command_palette.selected {
                        ">"
                    } else {
                        " "
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(marker, Style::default().fg(Color::Cyan)),
                        Span::raw(" "),
                        Span::styled(cmd.name, Style::default().fg(Color::Yellow)),
                        Span::raw("  "),
                        Span::raw(cmd.desc),
                    ]))
                })
                .collect()
        };

        frame.render_widget(Clear, area);
        frame.render_widget(List::new(items), area);
    }

    async fn handle_key(&mut self, key: KeyEvent) {
        if self.handle_modal_key(key) {
            return;
        }

        if self.agent_running {
            match key.code {
                KeyCode::Esc if self.command_palette.visible => {
                    self.command_palette.visible = false;
                    self.command_palette.selected = 0;
                }
                KeyCode::Esc => {
                    self.cancel_flag.store(true, Ordering::SeqCst);
                    self.status = "正在取消当前生成...".to_string();
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.cancel_flag.store(true, Ordering::SeqCst);
                    self.status = "正在取消当前生成...".to_string();
                }
                KeyCode::Enter => {
                    self.queue_current_input();
                }
                KeyCode::Up | KeyCode::Down | KeyCode::Tab if self.command_palette.visible => {
                    self.handle_running_palette_key(key);
                }
                _ => {}
            }
            if !matches!(
                key.code,
                KeyCode::Esc
                    | KeyCode::Enter
                    | KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Tab
            ) && !key.modifiers.contains(KeyModifiers::CONTROL)
            {
                let _ = self.input.handle_event(&Event::Key(key));
                self.sync_command_palette_with_input();
            }
            return;
        }

        if self.command_palette.visible {
            self.handle_palette_key(key).await;
            return;
        }

        match key.code {
            KeyCode::Enter => {
                let value = self.input.value().trim().to_string();
                if value.is_empty() {
                    return;
                }
                if value.starts_with('/') {
                    self.input.reset();
                    self.command_palette.visible = false;
                    self.command_palette.selected = 0;
                    self.handle_command(&value).await;
                } else {
                    self.submit_message(value).await;
                }
            }
            KeyCode::Char('/') if self.input.value().is_empty() => {
                let _ = self.input.handle_event(&Event::Key(key));
                self.sync_command_palette_with_input();
            }
            KeyCode::Esc => {
                if !self.input.value().is_empty() {
                    self.input.reset();
                } else {
                    self.modal = Some(Modal::confirm_quit());
                }
            }
            _ => {
                let _ = self.input.handle_event(&Event::Key(key));
                self.sync_command_palette_with_input();
            }
        }
    }

    fn sync_command_palette_with_input(&mut self) {
        if self.input.value().starts_with('/')
            && !self.command_palette.filtered(self.input.value()).is_empty()
        {
            self.command_palette.visible = true;
            self.command_palette.clamp_selection(self.input.value());
        } else {
            self.command_palette.visible = false;
            self.command_palette.selected = 0;
        }
    }

    fn handle_running_palette_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                self.command_palette.selected = self.command_palette.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                let len = self.visible_command_count();
                if len > 0 {
                    self.command_palette.selected =
                        (self.command_palette.selected + 1).min(len - 1);
                }
            }
            KeyCode::Tab => {
                self.complete_selected_command();
                self.sync_command_palette_with_input();
            }
            _ => {}
        }
    }

    fn queue_current_input(&mut self) {
        let value = self.input.value().trim().to_string();
        if value.is_empty() {
            return;
        }
        if value.starts_with('/') {
            self.status = "生成中暂不执行命令, 可取消后再执行.".to_string();
            return;
        }
        self.input.reset();
        self.command_palette.visible = false;
        self.command_palette.selected = 0;
        self.queued_messages.push_back(value);
        self.status = format!(
            "已排队 {} 条消息, 当前生成结束后自动发送.",
            self.queued_messages.len()
        );
    }

    fn handle_modal_key(&mut self, key: KeyEvent) -> bool {
        let Some(kind) = self.modal.as_ref().map(|m| m.kind) else {
            return false;
        };
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.modal = None;
                match kind {
                    ModalKind::Quit => self.running = false,
                    ModalKind::Reset => self.reset_conversation(),
                }
                true
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.modal = None;
                true
            }
            _ => true,
        }
    }

    async fn handle_palette_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.command_palette.visible = false;
                self.command_palette.selected = 0;
            }
            KeyCode::Up => {
                self.command_palette.selected = self.command_palette.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                let len = self.visible_command_count();
                if len > 0 {
                    self.command_palette.selected =
                        (self.command_palette.selected + 1).min(len - 1);
                }
            }
            KeyCode::Tab => {
                self.complete_selected_command();
                self.sync_command_palette_with_input();
            }
            KeyCode::Enter => {
                if self.complete_selected_command() {
                    let value = self.input.value().trim().to_string();
                    self.input.reset();
                    self.command_palette.visible = false;
                    self.handle_command(&value).await;
                }
            }
            _ => {
                let _ = self.input.handle_event(&Event::Key(key));
                self.sync_command_palette_with_input();
            }
        }
    }

    fn complete_selected_command(&mut self) -> bool {
        let filtered = self.command_palette.filtered(self.input.value());
        if filtered.is_empty() {
            return false;
        }
        let max_selected = self.visible_command_count().saturating_sub(1);
        let idx = filtered[self.command_palette.selected.min(max_selected)];
        self.input = Input::new(COMMANDS[idx].name.to_string());
        true
    }

    fn visible_command_count(&self) -> usize {
        self.command_palette
            .filtered(self.input.value())
            .len()
            .min(PALETTE_MAX_ROWS)
            .min(self.visible_command_rows.max(1))
    }

    async fn handle_command(&mut self, command: &str) {
        let base = command.split_whitespace().next().unwrap_or(command);
        match base {
            "/quit" | "/exit" => {
                self.command_palette.visible = false;
                self.command_palette.selected = 0;
                self.running = false;
            }
            "/help" => self.push_panel(
                "帮助",
                "基本\n  /help 显示帮助\n  /quit 退出\n  /reset 重置对话\n\n模式\n  /mode 查看模式\n  /mode plan 切换计划模式\n  /mode auto 切换自动模式\n  /plan 切换计划模式\n\n信息\n  /usage token 用量\n  /workspace 工作区\n  /skills 技能\n  /tools 工具\n  /memory 最近记忆\n\n对话\n  /retry 重试上一条消息\n  /edit 编辑上一条消息",
            ),
            "/reset" => self.modal = Some(Modal::confirm_reset()),
            "/usage" => {
                let usage = &self
                    .agent_loop
                    .as_ref()
                    .map(|a| a.accumulated_usage.clone())
                    .unwrap_or_default();
                self.push_panel(
                    "Token 用量",
                    &format!("输入: {}\n输出: {}\n总计: {}", usage.input, usage.output, usage.total()),
                );
            }
            "/workspace" => {
                let text = if self.workspace_context.is_empty() {
                    "未检测到工作区.".to_string()
                } else {
                    self.workspace_context.clone()
                };
                self.push_panel("工作区", &text);
            }
            "/skills" => {
                self.reload_skills_if_needed();
                let text = if self.skills_list.is_empty() {
                    "未加载任何技能.".to_string()
                } else {
                    self.skills_list.clone()
                };
                self.push_panel("技能", &text);
            }
            "/tools" => {
                let text = if self.tools_list.is_empty() {
                    "无可用工具.".to_string()
                } else {
                    self.tools_list.clone()
                };
                self.push_panel("工具", &text);
            }
            "/memory" => {
                let text = match &self.memory {
                    Some(memory) => {
                        let ctx = memory.build_context("", 1000).await;
                        if ctx.is_empty() {
                            "尚无记忆.".to_string()
                        } else {
                            ctx
                        }
                    }
                    None => "未启用记忆.".to_string(),
                };
                self.push_panel("记忆", &text);
            }
            "/retry" => {
                if let Some(last) = self.last_message.clone() {
                    self.submit_message(last).await;
                } else {
                    self.status = "尚无消息可重试.".to_string();
                }
            }
            "/edit" => {
                if let Some(last) = self.last_message.clone() {
                    self.input = Input::new(last);
                    self.status = "已将上一条消息放回输入框.".to_string();
                } else {
                    self.status = "尚无消息可编辑.".to_string();
                }
            }
            "/mode" => self.handle_mode_command(command),
            "/plan" => {
                if command.trim() == "/plan auto" {
                    self.set_mode(Mode::Auto);
                } else {
                    self.set_mode(Mode::Plan);
                }
            }
            _ => {
                if let Some(matched) = unique_command_match(command) {
                    Box::pin(self.handle_command(matched.name)).await;
                } else {
                    self.status = format!("未知命令: {command}");
                }
            }
        }
    }

    fn handle_mode_command(&mut self, command: &str) {
        match command.strip_prefix("/mode").map(str::trim).unwrap_or("") {
            "plan" => self.set_mode(Mode::Plan),
            "auto" => self.set_mode(Mode::Auto),
            "" => {
                self.status = format!(
                    "当前模式: {} ({})",
                    self.mode.display_name(),
                    mode_desc(self.mode),
                );
            }
            other => {
                self.status = format!("未知模式: {other}. 用法: /mode [plan | auto]");
            }
        }
    }

    fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.rebuild_system_prompt();
        self.status = format!("已切换到 {}: {}", mode.display_name(), mode_desc(mode));
    }

    async fn submit_message(&mut self, text: String) {
        if self.agent_running {
            return;
        }
        let Some(mut agent_loop) = self.agent_loop.take() else {
            self.status = "Agent 状态不可用.".to_string();
            return;
        };
        let Some(mut conversation) = self.conversation.take() else {
            self.status = "对话状态不可用.".to_string();
            self.agent_loop = Some(agent_loop);
            return;
        };

        self.input.reset();
        self.last_message = Some(text.clone());
        self.push_message(UiMessage::user(text.clone()));
        self.agent_running = true;
        self.cancel_flag.store(false, Ordering::SeqCst);
        self.status = "正在请求模型...".to_string();

        self.reload_skills_if_needed();
        let skill_injections = self.skill_injections_for(&text);
        let memory_context = match &self.memory {
            Some(memory) => memory.build_context("", 1000).await,
            None => String::new(),
        };
        let memory = self.memory.take();
        let provider = self.provider.clone();
        let tools = self.tools.clone();
        let filter = self.mode.tool_filter();
        let catalog = self.skills_catalog.clone();
        let tx = self.tx.clone();
        let cancel_flag = self.cancel_flag.clone();
        let original_text = text.clone();

        tokio::spawn(async move {
            conversation.push_user_message(vec![ContentBlock::text(&original_text)]);
            let sink = TuiOutputSink { tx: tx.clone() };
            processing_loop::handle_user_message(
                &mut agent_loop,
                &mut conversation,
                provider.as_ref(),
                &tools,
                filter,
                &sink,
                &memory_context,
                &catalog,
                &skill_injections,
                cancel_flag,
            )
            .await;

            if let Some(memory) = &memory {
                let summary = truncate_chars(&original_text, 200);
                let _ = memory.remember(&summary, "conversation").await;
            }

            let _ = tx.send(AgentEvent::RunFinished {
                agent_loop,
                conversation,
                memory,
            });
        });
    }

    fn skill_injections_for(&mut self, text: &str) -> String {
        let manual_names: Vec<String> = SkillManager::extract_manual_mentions(text)
            .into_iter()
            .filter(|name| self.skills.select_by_name(name).is_some())
            .collect();
        let mut exclude: Vec<&str> = self.recently_injected.iter().map(String::as_str).collect();
        for name in &manual_names {
            if !exclude.contains(&name.as_str()) {
                exclude.push(name);
            }
        }
        let auto_names = self.skills.auto_select(text, &exclude);
        let mut names = manual_names;
        names.extend(auto_names);
        for name in &names {
            self.recently_injected.push(name.clone());
        }
        if self.recently_injected.len() > 10 {
            self.recently_injected
                .drain(0..self.recently_injected.len() - 10);
        }
        self.skills.build_injections(&names)
    }

    async fn drain_agent_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                AgentEvent::Text(text) => self.append_assistant_text(&text),
                AgentEvent::Thinking(text) => {
                    if !text.is_empty() {
                        self.status = format!("思考中: 已接收 {} 字符", text.len());
                    }
                }
                AgentEvent::ThinkingFlush => {}
                AgentEvent::ToolCall { id, name, args } => {
                    self.push_message(UiMessage::tool(format!(
                        "{name} ({id})\n{}",
                        format_json_compact(&args)
                    )));
                }
                AgentEvent::TokenUsage(usage) => {
                    self.status = format!("Token: 输入 {}, 输出 {}", usage.input, usage.output);
                }
                AgentEvent::Status(status) => {
                    self.status = status;
                }
                AgentEvent::Error(error) => {
                    self.push_message(UiMessage::error(error));
                }
                AgentEvent::TurnFinished(reason) => {
                    self.status = format!("本轮结束: {reason:?}");
                }
                AgentEvent::RunFinished {
                    agent_loop,
                    conversation,
                    memory,
                } => {
                    let was_cancelled = self.cancel_flag.load(Ordering::SeqCst);
                    self.agent_loop = Some(agent_loop);
                    self.conversation = Some(conversation);
                    self.memory = memory;
                    self.agent_running = false;
                    if was_cancelled {
                        if let Some(next) = self.queued_messages.pop_front() {
                            self.input = Input::new(next);
                            self.status = "已取消, 已将排队消息放回输入框.".to_string();
                        } else {
                            self.status = "已取消.".to_string();
                        }
                        self.cancel_flag.store(false, Ordering::SeqCst);
                    } else if let Some(next) = self.queued_messages.pop_front() {
                        self.status = "正在发送排队消息...".to_string();
                        self.submit_message(next).await;
                    } else if self.status.starts_with("正在") {
                        self.status = "就绪".to_string();
                    }
                }
            }
        }
    }

    fn append_assistant_text(&mut self, text: &str) {
        match self.messages.last_mut() {
            Some(last) if last.role == UiRole::Assistant && last.title == "回复" => {
                last.content.push_str(text);
            }
            _ => self.push_message(UiMessage {
                role: UiRole::Assistant,
                title: "回复".to_string(),
                content: text.to_string(),
            }),
        }
    }

    fn push_panel(&mut self, title: &str, content: &str) {
        self.push_message(UiMessage {
            role: UiRole::System,
            title: title.to_string(),
            content: content.to_string(),
        });
    }

    fn push_message(&mut self, message: UiMessage) {
        self.messages.push(message);
        if self.messages.len() > MAX_MESSAGES {
            let removed = self.messages.len() - MAX_MESSAGES;
            self.messages.drain(0..removed);
            self.scrollback_cursor = self.scrollback_cursor.saturating_sub(removed);
        }
    }

    fn reset_conversation(&mut self) {
        if self.agent_running {
            self.cancel_flag.store(true, Ordering::SeqCst);
        }
        let system = system_prompt::build_system_prompt(
            &self.workspace_context,
            self.global_instructions.as_deref(),
            self.mode.display_name(),
        );
        let mut conversation = Conversation::new(Some(system));
        conversation.set_budget(self.budget.0, self.budget.1);
        self.conversation = Some(conversation);
        if let Some(agent) = &mut self.agent_loop {
            agent.reset();
        }
        self.messages.clear();
        self.scrollback_cursor = 0;
        self.push_message(UiMessage::system("对话已重置."));
        self.status = "对话已重置.".to_string();
    }

    fn rebuild_system_prompt(&mut self) {
        let Some(conversation) = &mut self.conversation else {
            return;
        };
        let mut new_conv = Conversation::new(Some(system_prompt::build_system_prompt(
            &self.workspace_context,
            self.global_instructions.as_deref(),
            self.mode.display_name(),
        )));
        new_conv.set_budget(self.budget.0, self.budget.1);
        *conversation = new_conv;
        self.push_message(UiMessage::system("模式已切换, 新模式将从下一轮对话开始."));
    }

    fn reload_skills_if_needed(&mut self) {
        if !self.skill_watcher.has_changed() {
            return;
        }
        self.skills.reload(self.ws_root.as_deref());
        self.skills_catalog = self.skills.build_catalog_prompt();
        self.skills_list = self.skills.build_skills_list();
        self.tools_list = self.tools.format_tools_list();
        self.status = "技能文件已变更, 已重新加载.".to_string();
    }
}

fn message_lines_for(messages: &[UiMessage]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for message in messages {
        let (label, color) = match message.role {
            UiRole::User => ("你", Color::Green),
            UiRole::Assistant => ("千寻", Color::Cyan),
            UiRole::Tool => ("工具", Color::Magenta),
            UiRole::System => ("系统", Color::DarkGray),
            UiRole::Error => ("错误", Color::Red),
        };
        lines.push(Line::from(vec![
            Span::styled(
                label,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(message.title.clone(), Style::default().fg(Color::DarkGray)),
        ]));
        for line in message.content.lines() {
            lines.push(Line::from(format!("  {line}")));
        }
        lines.push(Line::from(""));
    }
    lines
}

#[derive(Default)]
struct CommandPalette {
    visible: bool,
    selected: usize,
}

impl CommandPalette {
    fn filtered(&self, input: &str) -> Vec<usize> {
        filtered_commands(input)
    }

    fn clamp_selection(&mut self, input: &str) {
        let len = self.filtered(input).len();
        self.clamp_selection_to_len(len);
    }

    fn clamp_selection_to_len(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(len - 1);
        }
    }
}

#[derive(Clone, Copy)]
struct Modal {
    kind: ModalKind,
    title: &'static str,
    body: &'static str,
}

impl Modal {
    fn confirm_quit() -> Self {
        Self {
            kind: ModalKind::Quit,
            title: "退出确认",
            body: "确定退出千寻吗?  y 确认, n/Esc 取消",
        }
    }

    fn confirm_reset() -> Self {
        Self {
            kind: ModalKind::Reset,
            title: "重置确认",
            body: "确定重置当前对话吗?  y 确认, n/Esc 取消",
        }
    }
}

#[derive(Clone, Copy)]
enum ModalKind {
    Quit,
    Reset,
}

#[derive(Clone, PartialEq, Eq)]
enum UiRole {
    User,
    Assistant,
    Tool,
    System,
    Error,
}

#[derive(Clone)]
struct UiMessage {
    role: UiRole,
    title: String,
    content: String,
}

impl UiMessage {
    fn user(content: String) -> Self {
        Self {
            role: UiRole::User,
            title: "消息".to_string(),
            content,
        }
    }

    fn system(content: impl Into<String>) -> Self {
        Self {
            role: UiRole::System,
            title: "提示".to_string(),
            content: content.into(),
        }
    }

    fn tool(content: String) -> Self {
        Self {
            role: UiRole::Tool,
            title: "调用".to_string(),
            content,
        }
    }

    fn error(content: String) -> Self {
        Self {
            role: UiRole::Error,
            title: "错误".to_string(),
            content,
        }
    }
}

enum AgentEvent {
    Text(String),
    Thinking(String),
    ThinkingFlush,
    ToolCall {
        id: String,
        name: String,
        args: serde_json::Value,
    },
    TokenUsage(TokenUsage),
    Status(String),
    Error(String),
    TurnFinished(StopReason),
    RunFinished {
        agent_loop: AgentLoop,
        conversation: Conversation,
        memory: Option<Box<dyn MemoryObserver + Send>>,
    },
}

struct TuiOutputSink {
    tx: mpsc::UnboundedSender<AgentEvent>,
}

#[async_trait]
impl OutputSink for TuiOutputSink {
    async fn on_text(&self, text: &str) {
        let _ = self.tx.send(AgentEvent::Text(text.to_string()));
    }

    async fn on_thinking(&self, text: &str) {
        let _ = self.tx.send(AgentEvent::Thinking(text.to_string()));
    }

    async fn on_thinking_flush(&self) {
        let _ = self.tx.send(AgentEvent::ThinkingFlush);
    }

    async fn on_tool_call(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) {
        let _ = self.tx.send(AgentEvent::ToolCall {
            id: tool_call_id.to_string(),
            name: tool_name.to_string(),
            args: arguments.clone(),
        });
    }

    async fn on_token_usage(&self, usage: &TokenUsage) {
        let _ = self.tx.send(AgentEvent::TokenUsage(usage.clone()));
    }

    async fn on_error(&self, error: &LlmError) {
        let _ = self.tx.send(AgentEvent::Error(error.to_string()));
    }

    async fn on_turn_finished(&self, reason: &StopReason, _usage: &TokenUsage) {
        let _ = self.tx.send(AgentEvent::TurnFinished(reason.clone()));
    }

    async fn on_status(&self, status: &str) {
        let _ = self.tx.send(AgentEvent::Status(status.to_string()));
    }
}

fn render_modal(frame: &mut Frame, modal: &Modal) {
    let area = centered_rect(54, 7, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(modal.body)
            .block(Block::default().title(modal.title).borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(width.min(area.width)),
            Constraint::Fill(1),
        ])
        .split(area);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height.min(area.height)),
            Constraint::Fill(1),
        ])
        .split(horizontal[1]);
    vertical[1]
}

fn filtered_commands(input: &str) -> Vec<usize> {
    let needle = input.strip_prefix('/').unwrap_or(input).to_lowercase();
    COMMANDS
        .iter()
        .enumerate()
        .filter(|(_, cmd)| {
            needle.is_empty()
                || cmd.name[1..].starts_with(&needle)
                || cmd.desc.to_lowercase().contains(&needle)
        })
        .map(|(idx, _)| idx)
        .collect()
}

fn unique_command_match(input: &str) -> Option<CommandSpec> {
    let matches = filtered_commands(input);
    if matches.len() == 1 {
        Some(COMMANDS[matches[0]])
    } else {
        None
    }
}

fn mode_desc(mode: Mode) -> &'static str {
    match mode {
        Mode::Auto => "所有工具可用",
        Mode::Plan => "仅允许读取, 搜索和思考类工具",
    }
}

fn new_agent_loop(
    agent_config: qianxun_core::types::AgentConfig,
    compact_config: &Option<ResolvedCompactionConfig>,
) -> AgentLoop {
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

fn build_memory() -> Option<Box<dyn MemoryObserver + Send>> {
    let db_path = qianxun_core::workspace::qianxun_dir()?.join("mem.db");
    Some(Box::new(qianxun_memory::MemoryCore::open(&db_path).ok()?))
}

async fn connect_workspace_mcp(ws_root: Option<&std::path::Path>, registry: &mut ToolRegistry) {
    let Some(root) = ws_root else {
        return;
    };
    let Ok(Some(config)) = qianxun_core::mcp::config::McpConfigFile::find_in_workspace(root) else {
        return;
    };
    for server in config.to_server_configs() {
        match qianxun_core::mcp::client::McpClient::connect(server.clone()).await {
            Ok(client) => {
                let client = Arc::new(client);
                match client.list_tools().await {
                    Ok(tools) => {
                        for tool in tools {
                            registry.register_mcp_tool(qianxun_core::tools::McpToolEntry {
                                client_id: client.server_name().to_string(),
                                name: tool.name,
                                description: tool.description,
                                input_schema: tool.input_schema,
                            });
                        }
                        registry.register_mcp_client(client);
                    }
                    Err(e) => {
                        tracing::warn!("[mcp:{}] list tools failed: {e}", server.name);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("[mcp:{}] connect failed: {e}", server.name);
            }
        }
    }
}

fn format_json_compact(value: &serde_json::Value) -> String {
    let raw = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
    truncate_chars(&raw, 240)
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let end = text
        .char_indices()
        .nth(max)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    format!("{}...", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn test_app(input: impl Into<String>) -> App {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut tools = ToolRegistry::new();
        builtin::register_all(&mut tools);
        let cfg = qianxun_core::config::ResolvedConfig::default();
        App {
            input: Input::new(input.into()),
            messages: vec![UiMessage::system("ready")],
            status: "就绪".to_string(),
            mode: Mode::Auto,
            running: true,
            agent_running: false,
            scrollback_cursor: 0,
            command_palette: CommandPalette::default(),
            visible_command_rows: 0,
            modal: None,
            provider: qianxun_core::provider::create_provider(&cfg.deepseek).into(),
            tools,
            agent_loop: Some(AgentLoop::new(cfg.agent.clone())),
            conversation: Some(Conversation::new(None)),
            memory: None,
            skills: SkillManager::new(),
            skill_watcher: SkillWatcher::new(None),
            skills_catalog: String::new(),
            skills_list: String::new(),
            tools_list: String::new(),
            recently_injected: Vec::new(),
            last_message: None,
            queued_messages: VecDeque::new(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            workspace_context: String::new(),
            ws_root: None,
            global_instructions: None,
            budget: (None, None),
            last_tick: Instant::now(),
            spinner_index: 0,
        }
    }

    #[test]
    fn filters_slash_commands() {
        let matches = filtered_commands("/mo");
        assert_eq!(COMMANDS[matches[0]].name, "/mode");
    }

    #[tokio::test]
    async fn command_palette_closes_after_slash_is_removed() {
        let mut app = test_app("");

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.input.value(), "/");
        assert!(app.command_palette.visible);

        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
            .await;
        assert_eq!(app.input.value(), "");
        assert!(!app.command_palette.visible);

        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.input.value(), "h");
        assert!(!app.command_palette.visible);
    }

    #[tokio::test]
    async fn command_palette_closes_for_unknown_prefix() {
        let mut app = test_app("");

        for ch in ['/', 'z', 'z', 'z'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .await;
        }

        assert_eq!(app.input.value(), "/zzz");
        assert!(!app.command_palette.visible);
    }

    #[tokio::test]
    async fn slash_q_exits_without_confirmation() {
        let mut app = test_app("/q");
        app.command_palette.visible = true;

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .await;

        assert!(!app.running);
        assert!(app.modal.is_none());
        assert!(!app.command_palette.visible);
        assert_eq!(app.command_palette.selected, 0);
    }

    #[tokio::test]
    async fn command_palette_down_stops_at_last_visible_item() {
        let mut app = test_app("/");
        app.command_palette.visible = true;
        app.visible_command_rows = 7;

        for _ in 0..(PALETTE_MAX_ROWS + 5) {
            app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                .await;
        }

        assert_eq!(app.command_palette.selected, 6);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .await;

        assert_eq!(app.command_palette.selected, 6);
        assert_eq!(app.input.value(), "/");
    }

    #[tokio::test]
    async fn enter_queues_message_while_agent_is_running() {
        let mut app = test_app("next task");
        app.agent_running = true;

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .await;

        assert_eq!(app.input.value(), "");
        assert_eq!(app.queued_messages.len(), 1);
        assert_eq!(app.queued_messages.front().unwrap(), "next task");
        assert!(app.status.contains("已排队"));
    }

    #[test]
    fn live_messages_exclude_completed_scrollback() {
        let mut app = test_app("");
        app.messages = (0..8)
            .map(|idx| UiMessage::system(format!("message {idx}")))
            .collect();
        app.scrollback_cursor = 7;

        let lines = app.live_message_lines();

        assert!(lines
            .iter()
            .any(|line| format!("{line:?}").contains("message 7")));
        assert!(!lines
            .iter()
            .any(|line| format!("{line:?}").contains("message 6")));
    }

    #[test]
    fn renders_basic_layout() {
        let mut app = test_app("hello");
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn renders_command_palette_in_non_zero_viewport() {
        let mut app = test_app("/");
        app.messages.clear();
        app.command_palette.visible = true;
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 23, 68, 8)),
            },
        )
        .unwrap();

        terminal.draw(|frame| app.render(frame)).unwrap();
    }
}
