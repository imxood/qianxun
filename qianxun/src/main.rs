use clap::Parser;
use std::time::Duration;
use tracing_subscriber::fmt::time::FormatTime;

mod acp;
mod buf_writer;
mod cli;
mod client;
mod runtime;
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

    /// 强制内嵌模式 (不连接 daemon, 进程内嵌 AgentLoop).
    /// 默认行为 (不传此 flag): 启动时探测本地 daemon, 有则走 thin client,
    /// 无则回退到内嵌模式 (向后兼容 Stage 3 行为).
    #[arg(long)]
    standalone: bool,

    /// Thin client 模式调本地 daemon 时附带的 Bearer token (Stage 6b).
    ///
    /// 配套 daemon `auth_middleware` (HS256 JWT) 使用, 缺省时从
    /// env var `QIANXUN_CLIENT_TOKEN` 自动读. token 与启动 daemon 时
    /// 设置的 `QIANXUN_JWT_SECRET` 用同一个密钥签发.
    ///
    /// 例: `--client-token eyJhbGciOiJIUzI1NiIs...` 或
    /// `QIANXUN_CLIENT_TOKEN=eyJ... qx --daemon-url http://...`
    ///
    /// 注: clap 在本项目未启用 `env` feature, env var 解析在下面手动做
    /// (与 `--config` 走 env 的处理方式一致).
    #[arg(long)]
    client_token: Option<String>,

    /// Stage 7a: Web Admin Console 静态文件 dist 路径 (SvelteKit 产物).
    /// 优先级: CLI > env `QIANXUN_UI_DIST` > 默认 (`<exe 同级>/ui/` release
    /// 或 `<workspace>/qianxun/src/daemon/ui/dist/` debug). 路径不存在时
    /// daemon 仍启动, 但 `/_ui/*` 返 503 + 提示 `pnpm build`.
    #[arg(long)]
    ui_dist: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut cli = Cli::parse();

    // 解析 --client-token: CLI flag 优先, 回退到 env var `QIANXUN_CLIENT_TOKEN`.
    // (clap env feature 未启用, 手动读 env. 跟 `--config` 走 env 的处理一致.)
    if cli.client_token.is_none() {
        if let Ok(s) = std::env::var("QIANXUN_CLIENT_TOKEN") {
            if !s.is_empty() {
                cli.client_token = Some(s);
            }
        }
    }

    // 解析 --ui-dist 路径: CLI > env > 默认
    // (跟 `--client-token` 走 env 的处理方式一致)
    let ui_dist: Option<std::path::PathBuf> = cli
        .ui_dist
        .as_ref()
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var("QIANXUN_UI_DIST").ok().map(std::path::PathBuf::from))
        .or_else(default_ui_dist_path);

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
        // Stage 10a: 加载 (或首启动创建) admin credential.
        // 文件路径: ~/.qianxun/admin.cred (JSON: password_hash + token_secret).
        // 首启动会生成随机 password 并打印到 stderr.
        let cred_path = qianxun_core::workspace::qianxun_dir()
            .ok_or_else(|| anyhow::anyhow!("cannot determine ~/.qianxun home dir"))?
            .join("admin.cred");
        let admin = match runtime::auth::AdminCredential::load_or_create(&cred_path) {
            Ok(a) => std::sync::Arc::new(a),
            Err(e) => {
                eprintln!("错误: 加载 admin credential 失败 ({}): {e}", cred_path.display());
                std::process::exit(1);
            }
        };
        tracing::info!(
            "[runtime] admin credential loaded (path={}, hash_len={}B, secret_len={}B)",
            cred_path.display(),
            admin.password_hash().len(),
            admin.token_secret().len()
        );
        // 兼容: 如果用户设了 QIANXUN_JWT_SECRET env var, 提示一下已不再使用.
        if std::env::var("QIANXUN_JWT_SECRET").is_ok() {
            tracing::warn!(
                "[runtime] QIANXUN_JWT_SECRET env var is set but ignored — \
                 token secret now stored in admin.cred file. \
                 Run `qx --daemon` once to bootstrap; subsequent boots will use the file."
            );
        }
        runtime::run(cli.port, resolved, ui_dist, admin).await?;
        return Ok(());
    }

    // 显式指定 daemon URL → 薄客户端模式 (优先于 --standalone)
    if let Some(ref daemon_url) = cli.daemon_url {
        tracing::info!("以薄客户端模式连接 Daemon: {daemon_url}");
        return client::run_thin_repl(daemon_url, cli.client_token.as_deref()).await;
    }

    // ── Stage 4: 默认探测 daemon ──
    // 探测时机: 用户没传 --standalone 也没传 --daemon-url 时.
    // 有 daemon → thin client; 无 daemon → 回退 standalone (向后兼容).
    let use_thin = !cli.standalone && client::detect_local_daemon().await.is_some();
    if use_thin {
        let daemon_url = client::detect_local_daemon().await.expect("just probed");
        tracing::info!("[main] 检测到本地 Daemon: {daemon_url}, 走 thin client");
        if cli.acp_mode {
            // ACP thin path 暂未实现 (Stage 4a+): 提示并回退 standalone.
            eprintln!(
                "[main] ACP thin-client 模式尚未实现, 暂以 standalone 模式运行 (daemon={daemon_url})"
            );
        } else {
            return client::run_thin_repl(&daemon_url, cli.client_token.as_deref()).await;
        }
    } else if !cli.standalone {
        tracing::info!("[main] 未检测到本地 Daemon, 回退 standalone 模式");
    }

    if cli.acp_mode {
        tracing::info!("以 ACP 协议模式启动 (provider={})", resolved.active_provider);
        // Phase 4a 收尾: ACP 入口走 qianxun-runtime 统一 RuntimeState.
        // 跟 desktop/daemon/TUI 共享同一份 RuntimeState 初始化 (单点维护).
        let state = qianxun_runtime::RuntimeState::new(resolved).await?;
        crate::acp::run_acp_server(state).await?;
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

/// Stage 7a: 默认 UI dist 路径查找.
///
/// 优先级:
/// 1. `<exe 同级>/ui/` (release 打包时)
/// 2. `<workspace>/qianxun/src/daemon/ui/dist/` (dev)
/// 3. 都没有 → None (daemon 启动时打 "Web UI disabled")
fn default_ui_dist_path() -> Option<std::path::PathBuf> {
    // 1. exe 同级
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("ui");
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    // 2. workspace dev 路径
    if let Ok(cwd) = std::env::current_dir() {
        // 从 cwd 向上找 qianxun-core 标志 (workspace root), 然后用 qianxun/src/daemon/ui/dist
        if let Some(root) = find_workspace_root(&cwd) {
            let candidate = root
                .join("qianxun")
                .join("src")
                .join("daemon")
                .join("ui")
                .join("dist");
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 从 start_dir 向上找 Cargo.toml 含 `[workspace]` 的目录 (workspace root).
fn find_workspace_root(start_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut cur: Option<&std::path::Path> = Some(start_dir);
    while let Some(dir) = cur {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file() {
            if let Ok(s) = std::fs::read_to_string(&manifest) {
                if s.contains("[workspace]") {
                    return Some(dir.to_path_buf());
                }
            }
        }
        cur = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_workspace_root_finds_workspace_marker() {
        // cargo test 跑在 workspace 根 (qianxun/Cargo.toml 含 [workspace])
        let cwd = std::env::current_dir().unwrap();
        let root = find_workspace_root(&cwd);
        assert!(root.is_some(), "should find a workspace root from cwd");
        let root = root.unwrap();
        assert!(root.join("Cargo.toml").is_file());
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("[workspace]"));
    }

    #[test]
    fn test_default_ui_dist_path_does_not_panic() {
        // 不论是否存在, 都不应 panic; 返回 Option.
        let _ = default_ui_dist_path();
    }
}

