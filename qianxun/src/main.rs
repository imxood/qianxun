use clap::Parser;
use std::time::Duration;
use tracing_subscriber::fmt::time::FormatTime;

mod acp;
mod buf_writer;
mod cli;
mod daemon;
mod server;
mod tui;

struct LocalTimer;

impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(
            w,
            "{}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f")
        )
    }
}

/// 千寻 (Qianxun) — AI 编程助手 CLI
///
/// 支持独立 CLI 模式和 ACP 协议模式。
/// ACP 模式下通过 stdio 与 Zed 等编辑器通信。
#[derive(Parser)]
#[command(name = "qx", version, about)]
struct Cli {
    /// 以 ACP 协议模式运行（用于 Zed 集成）
    #[arg(long)]
    acp_mode: bool,

    /// 调试模式（输出详细日志）
    #[arg(short, long)]
    verbose: bool,

    /// 指定模型
    #[arg(short, long)]
    model: Option<String>,

    /// 指定 LLM provider 名称 (例如 "deepseek" / "MiniMax").
    /// 默认从 config.active_provider 读取, 留空则 "deepseek".
    #[arg(long)]
    provider: Option<String>,

    /// 配置文件路径（默认: ~/.qianxun/config.json）
    #[arg(long)]
    config: Option<String>,

    /// 生成默认配置文件模板并退出
    #[arg(long)]
    generate_config: bool,

    /// 工作区路径（默认从当前目录自动检测）
    #[arg(short, long)]
    workspace: Option<String>,

    /// 日志文件路径（指定后将日志写入文件而非 stderr）
    #[arg(long)]
    log_file: Option<String>,

    /// 启动时恢复最后一次会话
    #[arg(long)]
    resume: bool,

    /// 以 Daemon 模式运行（HTTP 服务常驻）
    #[arg(long)]
    daemon: bool,

    /// Daemon HTTP 端口（默认 23900）
    #[arg(long, default_value_t = 23900)]
    port: u16,

    /// 连接外部 Daemon 的 URL（例如 http://127.0.0.1:23900）
    /// 设置后 CLI 作为薄客户端连接 Daemon，不创建本地 AgentLoop
    #[arg(long)]
    daemon_url: Option<String>,

    /// 以 VPS Server 模式运行
    #[arg(long)]
    server: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let filter = if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::EnvFilter::from_default_env()
    } else if cli.verbose {
        tracing_subscriber::EnvFilter::new("debug,rustyline=info")
    } else {
        tracing_subscriber::EnvFilter::new("info")
    };

    if let Some(ref log_path) = cli.log_file {
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
        {
            Ok(log_file) => {
                tracing_subscriber::fmt()
                    .with_timer(LocalTimer)
                    .with_writer(buf_writer::TimedBufWriter::new(
                        log_file,
                        4096,
                        Duration::from_secs(1),
                    ))
                    .with_env_filter(filter)
                    .with_ansi(false)
                    .init();
            }
            Err(e) => {
                eprintln!("警告: 无法打开日志文件 {log_path}: {e}，回退到 stderr");
                tracing_subscriber::fmt()
                    .with_timer(LocalTimer)
                    .with_env_filter(filter)
                    .init();
            }
        }
    } else {
        tracing_subscriber::fmt()
            .with_timer(LocalTimer)
            .with_env_filter(filter)
            .init();
    }

    if cli.generate_config {
        match crate::cli::config::write_default_config() {
            Ok(path) => {
                println!("配置文件已生成: {}", path.display());
            }
            Err(e) => {
                eprintln!("错误: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    // 加载配置（双模式共享）
    let config_path = cli
        .config
        .as_ref()
        .map(std::path::PathBuf::from)
        .or_else(crate::cli::config::default_config_path);

    let resolved = if let Some(ref path) = config_path {
        match qianxun_core::config::Config::from_file(path) {
            Ok(raw) => {
                // env var 读取由 resolve() 内部按 provider 分发 (DEEPSEEK_API_KEY / ANTHROPIC_AUTH_TOKEN / 通用约定)
                raw.resolve(cli.model.clone(), cli.provider.clone())
            }
            Err(e) => {
                eprintln!("警告: 无法读取配置文件 {}: {e}", path.display());
                let mut cfg = qianxun_core::config::ResolvedConfig::default();
                if let Some(ref p) = cli.provider {
                    cfg.active_provider = p.clone();
                }
                // 用 resolve() 走 env 路径, 统一处理
                let env_resolved = qianxun_core::config::Config::default()
                    .resolve(cli.model.clone(), cli.provider.clone());
                // env 缺失时回退到原始 cfg (保持向后兼容)
                if env_resolved.active_provider_config().api_key.is_empty() {
                    cfg
                } else {
                    env_resolved
                }
            }
        }
    } else {
        // 无 config 文件 → 用默认 Config (空), 完全依赖 env vars
        qianxun_core::config::Config::default().resolve(cli.model.clone(), cli.provider.clone())
    };

    // 验证 active provider 的 api_key 非空
    if resolved.active_provider_config().api_key.is_empty() {
        let provider = &resolved.active_provider;
        let env_var = match provider.as_str() {
            "deepseek" => "DEEPSEEK_API_KEY",
            "MiniMax" => "ANTHROPIC_AUTH_TOKEN",
            other => {
                // 通用约定
                tracing::warn!(
                    "[main] no api_key for provider '{other}'; tried {}_API_KEY / {}_AUTH_TOKEN env vars and config.providers.{other}.api_key",
                    other.to_uppercase(),
                    other.to_uppercase()
                );
                "<PROVIDER>_API_KEY or <PROVIDER>_AUTH_TOKEN"
            }
        };
        eprintln!(
            "错误: provider '{provider}' 缺少 API key. 请设置 env var {env_var} 或在 config.json 的 providers.{provider}.api_key 填写."
        );
        std::process::exit(1);
    }

    if cli.server {
        tracing::info!("以 VPS Server 模式启动（端口 {}）", cli.port);
        server::run(cli.port).await?;
        return Ok(());
    }

    if cli.daemon {
        tracing::info!("以 Daemon 模式启动（端口 {}）", cli.port);
        daemon::run(cli.port).await?;
        return Ok(());
    }

    // 薄客户端模式：连接到远程 Daemon
    if let Some(ref daemon_url) = cli.daemon_url {
        tracing::info!("以薄客户端模式连接 Daemon: {daemon_url}");
        return run_thin_client(daemon_url).await;
    }

    if cli.acp_mode {
        tracing::info!("以 ACP 协议模式启动 (provider={})", resolved.active_provider);
        let provider = qianxun_core::provider::create_provider(
            &resolved.active_provider,
            &resolved.active_provider_config(),
        );
        crate::acp::run_acp_server(
            provider,
            resolved.agent.clone(),
            Some(resolved.compaction.clone()),
            resolved.budget.max_input_tokens,
            resolved.budget.max_output_tokens,
        )
        .await?;
    } else {
        tracing::info!("以独立 CLI 模式启动");

        // 检测项目根（.qianxun/ 向上查找）
        // 不改 cwd——Agent 的当前工作目录保持在用户启动 qx 的位置，方便相对路径引用。
        // 项目根仅用于：加载技能、MCP 配置、记忆存储、system prompt 上下文。
        let project_root = if let Some(ref w) = cli.workspace {
            Some(qianxun_core::workspace::project_root_from(
                std::path::Path::new(w),
            ))
        } else {
            std::env::current_dir()
                .ok()
                .as_ref()
                .and_then(|d| qianxun_core::workspace::find_project_root(d))
        };

        if let Some(ref pr) = project_root {
            tracing::info!("项目根: {}", pr.root.display());
        }

        let global_instructions = qianxun_core::workspace::read_global_agents_md();
        cli::run::run_repl(resolved, project_root, global_instructions).await?;
    }

    Ok(())
}

/// 薄客户端模式：通过 HTTP 连接 Daemon，不创建本地 AgentLoop。
async fn run_thin_client(daemon_url: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let health_url = format!("{daemon_url}/v1/system/health");

    // 验证 Daemon 连接
    let resp = client
        .get(&health_url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("无法连接 Daemon {daemon_url}: {e}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("Daemon 返回错误: {}", resp.status());
    }
    tracing::info!("Daemon 已连接: {daemon_url}");
    println!("已连接到 Daemon: {daemon_url}");
    println!("输入消息后按 Enter 发送（输入 /quit /exit 退出）\n");

    let mut input = String::new();
    loop {
        input.clear();
        if std::io::stdin().read_line(&mut input).is_err() {
            break;
        }
        let input = input.trim();
        match input {
            "/quit" | "/exit" => break,
            "" => continue,
            _ => {}
        }

        // 创建会话
        let session_url = format!("{daemon_url}/v1/chat/session");
        let session_resp = match client.post(&session_url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                println!("[Daemon] session error: {}", r.status());
                continue;
            }
            Err(e) => {
                println!("[Daemon] session error: {e}");
                continue;
            }
        };
        let sid: serde_json::Value = session_resp.json().await.unwrap_or_default();
        let session_id = sid["session_id"].as_str().unwrap_or("unknown").to_string();

        // 发送 prompt
        let prompt_url = format!("{daemon_url}/v1/chat/session/{session_id}/prompt");
        let body = serde_json::json!({"messages": [{"role": "user", "content": input}]});
        match client.post(&prompt_url).json(&body).send().await {
            Ok(r) => {
                let text = r.text().await.unwrap_or_default();
                println!("{}", text);
            }
            Err(e) => {
                println!("[Daemon] prompt error: {e}");
            }
        }
    }
    Ok(())
}
