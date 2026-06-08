// ───────────────────────────────────────────────────────────────────────────
// 千寻 Tauri 桌面端 — Stage 4a 集成骨架 (04b sub-task #3)
//
// 范围 (sub-task #3: RuntimeApi 收口 + 5 个真 runtime command):
//   - 4 domain 子目录 (commands/{health, stronghold, runtime} + events + state)
//   - health domain:    本地 mock + 远程 daemon health 探活 (Stage 2, 不接 RuntimeState)
//   - stronghold domain: iota_stronghold 凭据加密 vault (Argon2 + ChaCha20, §11.3)
//   - runtime domain:   5 个真 command 走 RuntimeApi (list_sessions / send_message /
//                        create_plan / cancel_session / load_session)
//   - events:           emit 事件 schema 收口 (state_changed + session_event)
//   - state:            Tauri State 注入 (runtime.rs 真 build, RuntimeState::new + fallback)
//
// 设计原则 (maxu 要求: 不能单个大文件一直追加):
//   - lib.rs < 80 行薄壳 (装 plugin + setup + invoke_handler)
//   - 业务逻辑在 qianxun-runtime (path dep, RuntimeApi trait 收口)
//   - Tauri command 是 thin adapter (参数 + 返回 + emit event)
//   - 后续加新 command 时, lib.rs 只加 1 行 handler, 不会变成 600+ 行大文件
//
// 关联:
//   - docs/30_子项目规划/04b-tauri-runtime-integration.md (上位规划, sub-task #3)
//   - docs/30_子项目规划/04c-qianxun-runtime-extraction.md (sub-task #1, 已完成)
//   - ADR-0003 (合并 desktop + 2-mode 互斥)
// ───────────────────────────────────────────────────────────────────────────

mod commands;
mod events;
mod state;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .setup(|app| {
            // Stage 4a-3: 真 build RuntimeState (走 RuntimeState::new + fallback to new_for_test).
            // 失败 fallback 在 build() 内部已经处理, 永远不阻塞 desktop 启动.
            match state::runtime::build() {
                Ok(rt) => {
                    app.manage(rt);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to build RuntimeState, desktop continues without runtime");
                }
            }

            // Stage 2 兼容: setup 阶段立即发 'connected', 让前端能验证 IPC 桥接通.
            // 真实 health check + 状态机留 4a 后续 (与 daemon-stage2-sse-stream 对齐).
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                events::state_changed::emit(&handle, "connected");
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // health / stronghold (sub-task #2 已有)
            commands::health::check::health_check,
            commands::health::fetch::daemon_health_fetch,
            commands::stronghold::key::set_secret,
            commands::stronghold::key::get_secret,
            // runtime (sub-task #3 新增 5 个, 走 RuntimeApi trait 收口)
            commands::runtime::sessions::list_sessions,
            commands::runtime::send::send_message,
            commands::runtime::plans::create_plan,
            commands::runtime::plans::cancel_plan,
            commands::runtime::cancel::cancel_session,
            commands::runtime::load::load_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
