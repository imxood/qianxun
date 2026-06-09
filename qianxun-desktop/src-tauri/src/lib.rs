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
mod time;

use std::sync::Mutex;
use tauri::async_runtime::spawn;
use tauri::Manager;

pub fn run() {
    // 2026-06-09 L1: 初始化 tracing subscriber. 桌面端之前完全没注册,
    // 业务 tracing::info!/warn! 全被吞. 现在写到 stderr (跟 daemon 行为一致),
    // 用户可设 RUST_LOG=info,debug 控制粒度.
    // 2026-06-09 体验打磨: 用本地时间 (e.g. "2026-06-09 19:03:13.245") 替代 ISO 8601 UTC,
    // 方便用户直接对照日志跟问题现象 (本地时间).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,qianxun_runtime=debug")),
        )
        .with_target(true)
        .with_thread_ids(false)
        .with_timer(time::LocalTime)
        .compact()
        .init();

    let t_start = std::time::Instant::now();
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .setup(move |app| {
            // 2026-06-09: timing 日志同时输出 (a) 距 Tauri 进程启动的累计耗时 since_t0
            //              (b) 距上一跳的相对耗时 since_prev
            // 让用户一眼能区分 "Tauri 自身慢" vs "前端 Vite 编译慢" (后者不在 t_start 范围内)
            let log = |label: &str, since_t0_ms: u128, since_prev_ms: u128| {
                tracing::info!(since_t0_ms, since_prev_ms, "[boot] {label}");
            };
            let prev = std::time::Instant::now();
            log("T0 setup() entered", t_start.elapsed().as_millis(), prev.elapsed().as_millis());

            // 2026-06-09: 官方 splash 方案 (https://v2.tauri.app/learn/splashscreen/).
            // 注册 SetupState 给 set_complete command 用 — 跟踪前端 + 后端 ready 状态.
            app.manage(Mutex::new(SetupState {
                frontend_task: false,
                backend_task: false,
            }));

            // Stage 4a-3: 真 build RuntimeState (走 RuntimeState::new + fallback to new_for_test).
            // 失败 fallback 在 build() 内部已经处理, 永远不阻塞 desktop 启动.
            let t_build = std::time::Instant::now();
            match state::runtime::build() {
                Ok(rt) => {
                    log("T1 build() returned (skeleton OK, async init spawned)", t_start.elapsed().as_millis(), t_build.elapsed().as_millis());
                    app.manage(rt);
                }
                Err(e) => {
                    tracing::warn!(error = %e, since_t0_ms = t_start.elapsed().as_millis() as u64, since_prev_ms = t_build.elapsed().as_millis() as u64, "[boot] T1 build() failed, desktop continues without runtime");
                }
            }

            // 2026-06-09: 后端 set_complete 标记 'backend' (splash 关掉必要条件之一).
            // 后端 setup 不重, 立即标记 ready. 真正关 splash 等前端 + 后端都 ready.
            // 同步直接操作 Mutex, 不用 command (避免 app_handle 借用检查问题).
            {
                let app_handle = app.handle().clone();
                spawn(async move {
                    // 短暂延迟, 让 webview 加载 splashscreen.html (几乎瞬时, 50ms 足够)
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    let state = app_handle.state::<Mutex<SetupState>>();
                    let mut s = state.lock().unwrap();
                    s.backend_task = true;
                    let (frontend, backend) = (s.frontend_task, s.backend_task);
                    drop(s);
                    tracing::info!(frontend, backend, "[splash] backend task complete");
                    if frontend && backend {
                        if let Some(splash) = app_handle.get_webview_window("splashscreen") {
                            let _ = splash.close();
                        }
                        if let Some(main) = app_handle.get_webview_window("main") {
                            let _ = main.show();
                            let _ = main.set_focus();
                        }
                        tracing::info!("[splash] all tasks complete, splash closed, main shown");
                    }
                });
            }

            // Stage 2 兼容: setup 阶段立即发 'connected', 让前端能验证 IPC 桥接通.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                events::state_changed::emit(&handle, "connected");
                log("T2 'connected' event emitted (Tauri setup done, webview 接管)", t_start.elapsed().as_millis(), 500);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // 2026-06-09 加: splash 状态管理
            set_complete,
            // health / stronghold (sub-task #2 已有)
            commands::health::check::health_check,
            commands::health::fetch::daemon_health_fetch,
            commands::stronghold::key::set_secret,
            commands::stronghold::key::get_secret,
            // runtime (sub-task #3 新增 5 个, 走 RuntimeApi trait 收口)
            commands::runtime::sessions::list_sessions,
            commands::runtime::sessions::create_session,
            commands::runtime::sessions::delete_session,
            commands::runtime::sessions::pause_session,
            commands::runtime::sessions::resume_session,
            commands::runtime::sessions::update_active_provider,
            commands::runtime::send::send_message,
            commands::runtime::plans::create_plan,
            commands::runtime::plans::cancel_plan,
            commands::runtime::cancel::cancel_session,
            commands::runtime::load::load_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// 2026-06-09 加: Tauri 官方 splash 方案状态管理 (https://v2.tauri.app/learn/splashscreen/).
// 跟踪 frontend + backend ready 状态, 都完成时关 splash window + 显示 main.
struct SetupState {
	frontend_task: bool,
	backend_task: bool,
}

#[tauri::command]
async fn set_complete(
	app: tauri::AppHandle,
	state: tauri::State<'_, Mutex<SetupState>>,
	task: String,
) -> Result<(), String> {
	if task != "frontend" {
		return Err(format!("invalid task: {task} (expected 'frontend')"));
	}
	let mut s = state.lock().map_err(|e| e.to_string())?;
	s.frontend_task = true;
	let (frontend, backend) = (s.frontend_task, s.backend_task);
	drop(s);
	tracing::info!(frontend, backend, "[splash] frontend task complete");

	// 两个 task 都完成 → 关 splash, 显示 main
	if frontend && backend {
		if let Some(splash) = app.get_webview_window("splashscreen") {
			let _ = splash.close();
		}
		if let Some(main) = app.get_webview_window("main") {
			let _ = main.show();
			let _ = main.set_focus();
		}
		tracing::info!("[splash] all tasks complete, splash closed, main shown");
	}
	Ok(())
}
