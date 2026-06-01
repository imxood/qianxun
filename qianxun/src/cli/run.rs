pub async fn run_repl(
    config: qianxun_core::config::ResolvedConfig,
    project_root: Option<qianxun_core::workspace::ProjectRoot>,
    global_instructions: Option<String>,
) -> anyhow::Result<()> {
    crate::tui::run(config, project_root, global_instructions).await
}
