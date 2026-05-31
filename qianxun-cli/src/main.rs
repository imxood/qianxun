use clap::Parser;
use std::time::Duration;
use tracing_subscriber::fmt::time::FormatTime;

mod buf_writer;

struct LocalTimer;

impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"))
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
                    .with_writer(buf_writer::TimedBufWriter::new(log_file, 4096, Duration::from_secs(1)))
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
        match qianxun_cli::config::write_default_config() {
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
        .or_else(qianxun_cli::config::default_config_path);

    let resolved = if let Some(ref path) = config_path {
        match qianxun_core::config::Config::from_file(path) {
            Ok(raw) => {
                let env_key = std::env::var("DEEPSEEK_API_KEY").ok();
                raw.resolve(env_key, cli.model.clone())
            }
            Err(e) => {
                eprintln!("警告: 无法读取配置文件 {}: {e}", path.display());
                let env_key = std::env::var("DEEPSEEK_API_KEY")
                    .expect("DEEPSEEK_API_KEY environment variable or config file is required");
                let mut cfg = qianxun_core::config::ResolvedConfig::default();
                cfg.deepseek.api_key = env_key;
                if let Some(ref m) = cli.model {
                    cfg.deepseek.model = m.clone();
                }
                cfg
            }
        }
    } else {
        let env_key = std::env::var("DEEPSEEK_API_KEY")
            .expect("DEEPSEEK_API_KEY environment variable or config file is required");
        let mut cfg = qianxun_core::config::ResolvedConfig::default();
        cfg.deepseek.api_key = env_key;
        if let Some(ref m) = cli.model {
            cfg.deepseek.model = m.clone();
        }
        cfg
    };

    if cli.acp_mode {
        tracing::info!("以 ACP 协议模式启动");
        let provider = qianxun_core::provider::create_provider(&resolved.deepseek);
        qianxun_acp::run_acp_server(
            provider,
            resolved.agent.clone(),
            Some(resolved.compaction.clone()),
            resolved.budget.max_input_tokens,
            resolved.budget.max_output_tokens,
        )
        .await?;
    } else {
        tracing::info!("以独立 CLI 模式启动");

        // 检测工作区
        let workspace = if let Some(ref w) = cli.workspace {
            // 用户显式指定 -w，直接作为工作区根目录并切换 CWD
            let path = std::path::Path::new(w);
            if let Err(e) = std::env::set_current_dir(path) {
                tracing::warn!("无法切换工作目录到 {}: {e}", path.display());
            }
            Some(qianxun_core::workspace::workspace_from_root(path))
        } else {
            std::env::current_dir()
                .ok()
                .as_ref()
                .and_then(|d| qianxun_core::workspace::detect_workspace(d))
        };

        if let Some(ref ws) = workspace {
            tracing::info!("工作区已检测: {}", ws.root.display());
        }

        qianxun_cli::run_repl(&resolved, workspace, cli.resume).await?;
    }

    Ok(())
}
