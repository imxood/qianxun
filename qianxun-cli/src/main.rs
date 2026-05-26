use clap::Parser;

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

    /// 配置文件路径（默认: ~/.qianxun/config.json5）
    #[arg(long)]
    config: Option<String>,

    /// 生成默认配置文件模板并退出
    #[arg(long)]
    generate_config: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::EnvFilter::from_default_env()
    } else {
        tracing_subscriber::EnvFilter::new("info")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    let cli = Cli::parse();

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

    if cli.acp_mode {
        tracing::info!("以 ACP 协议模式启动");
        // Phase 2: ACP server start
        eprintln!("ACP 模式将在 Phase 2 中实现");
    } else {
        tracing::info!("以独立 CLI 模式启动");

        // 加载配置
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

        qianxun_cli::run_repl(cli.verbose, &resolved).await?;
    }

    Ok(())
}
