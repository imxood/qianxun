pub mod router;
pub mod agent_host;

use std::sync::Arc;
use tokio::sync::watch;

/// Daemon 共享状态。
pub struct AppState {
    pub agent_host: agent_host::AgentLoopHost,
    pub shutdown_tx: watch::Sender<()>,
}

/// 启动 Daemon HTTP 服务。
pub async fn run(port: u16) -> anyhow::Result<()> {
    tracing::info!("Daemon starting on 127.0.0.1:{port}");

    let (shutdown_tx, mut shutdown_rx) = watch::channel(());

    let state = Arc::new(AppState {
        agent_host: agent_host::AgentLoopHost::new(10),
        shutdown_tx,
    });

    let app = router::build_router(state);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_rx.changed().await.ok();
        })
        .await?;

    tracing::info!("Daemon stopped");
    Ok(())
}
