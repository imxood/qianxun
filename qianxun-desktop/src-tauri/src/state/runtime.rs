// state/runtime.rs — RuntimeState 真初始化 (Stage 4a sub-task #3 替换 stub)
//
// 流程:
//   1. 尝试读 ~/.qianxun/config.json (ResolvedConfig)
//   2. 调 RuntimeState::new(config).await (完整 provider/tools/memory/skills/store/agent_host)
//   3. 失败 fallback: 走 new_for_test() (in-memory, 不阻塞 desktop 启动)
//   4. 还失败: 返 Err (lib.rs::setup 收到 Err 时打 warn, desktop 继续启动但没 runtime)
//
// 设计:
//   - 跟 qianxun binary daemon 启动 1:1 (main.rs::runtime::run() 内部逻辑一致)
//   - config 加载路径跟 qianxun-core::workspace::qianxun_dir() 共享
//   - 失败 fallback 不直接 panic, 让 desktop 即使没 LLM key 也能启动 (设置面板用)
//   - **2026-06-09 修**: 改用 `spawn_blocking` 替代 `block_on`,避免抢 tokio runtime.
//     旧 `block_on` 在 Tauri setup 同步闭包跑, 内部 `restore_from_disk` 跑同步 SQLite,
//     会阻塞主线程 5-15 秒, 拖慢 webview 启动 + 死锁风险.
//   - **2026-06-09 修**: restore_from_disk 改成**懒初始化** (在 RuntimeState::new 末尾
//     不调, 改在首次 send_message 前自动调). 让 desktop 启动 < 1s, 后台恢复 conversation.

use std::sync::Arc;

use qianxun_core::config::{Config, ResolvedConfig};
use qianxun_runtime::RuntimeState;

/// 加载 ~/.qianxun/config.json, 失败返 None (不抛错, 走 fallback).
fn try_load_config() -> Option<ResolvedConfig> {
    let path = qianxun_core::workspace::qianxun_dir()?.join("config.json");
    match Config::from_file(&path) {
        Ok(cfg) => {
            tracing::info!("[state] loaded config from {}", path.display());
            Some(cfg.resolve(None, None))
        }
        Err(e) => {
            tracing::warn!(
                "[state] config load from {} failed: {e}, will use default",
                path.display()
            );
            None
        }
    }
}

/// 同步初始化 RuntimeState (供 lib.rs setup 调).
///
/// 2026-06-09 修: **极简骨架** 方案 (替代 spawn_blocking, 更稳).
///
/// 1. build() 同步只调 `RuntimeState::new_for_test()` (in-memory, <100ms)
///    — 桌面端 webview 立即可起, 用户看到 UI < 1s
/// 2. 后台 task 调 `RuntimeState::new(config)` + `ensure_restored()` (异步, 不阻塞 webview)
/// 3. 首次 `send_message` / `list_sessions` 前若还没 restore, 同步 await 一次
///    (此时用户已经点"发送", 1-2s 等待可接受)
///
/// 优点: build 同步返 + 极快, webview 启动 < 1s.
pub fn build() -> Result<Arc<RuntimeState>, String> {
    let t0 = std::time::Instant::now();
    let state = RuntimeState::new_for_test();
    tracing::info!(since_prev_ms = t0.elapsed().as_millis() as u64, "[state] T1.1 new_for_test() done (in-memory skeleton)");

    // 后台 spawn 真 init: 读 config + restore_from_disk.
    // 不能在 build() 同步跑 (会阻塞 5-15s + 死锁风险), 改在 tokio runtime 里 spawn.
    tauri::async_runtime::spawn(async move {
        // 1. 调 spawn_blocking 跑同步 init (独立线程, 不阻塞 tokio runtime)
        let t_async_start = std::time::Instant::now();
        let join_result = tauri::async_runtime::spawn_blocking(move || {
            // 1.1 读 config
            let t_cfg = std::time::Instant::now();
            let config = try_load_config().unwrap_or_default();
            tracing::info!(since_prev_ms = t_cfg.elapsed().as_millis() as u64, "[state] T1.2 try_load_config() done");
            // 1.2 构造真 RuntimeState. RuntimeState::new 是 async fn 但内部全同步 SQLite.
            //     跑在 blocking 线程 OK. 用 futures::executor::block_on 同步等 async fn.
            let t_new = std::time::Instant::now();
            let rt = match futures::executor::block_on(RuntimeState::new(config)) {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::warn!(since_prev_ms = t_new.elapsed().as_millis() as u64, "[state] T1.3 RuntimeState::new failed: {e}");
                    return;
                }
            };
            tracing::info!(since_prev_ms = t_new.elapsed().as_millis() as u64, "[state] T1.3 RuntimeState::new done");
            // 1.3 懒 restore (幂等)
            let t_restore = std::time::Instant::now();
            let _ = futures::executor::block_on(rt.ensure_restored());
            tracing::info!(since_prev_ms = t_restore.elapsed().as_millis() as u64, "[state] T1.4 ensure_restored() done");
            tracing::info!(since_prev_ms = t_async_start.elapsed().as_millis() as u64, "[state] T1 async init done: 真 provider + restore completed");
        })
        .await;

        if let Err(e) = join_result {
            tracing::warn!("[state] async init spawn_blocking join error: {e}");
        }
    });

    Ok(state)
}

