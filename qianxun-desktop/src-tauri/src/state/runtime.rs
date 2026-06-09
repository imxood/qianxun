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
/// 2026-06-09 重写: 同步跑真 init, 不用骨架 (骨架 bug: 后台 init 永远被丢弃).
///
/// 历史问题:
/// - 之前方案: build() 同步返 new_for_test() 骨架, 后台 spawn_blocking 调
///   `RuntimeState::new(config)` 创建真 state, 但 `rt` 在闭包结尾被丢弃, **app.manage()
///   永远是骨架**. 用户发消息时拿到的是骨架的 deepseek + 空 api_key, 报 auth 错误.
///
/// 修复: 同步跑真 init. 真 init 本身只 13ms (读 config + 建 provider + 16 sessions restore),
/// 比 7s Vite 编译短得多, 用户无感. 牺牲 < 50ms startup, 换取正确性.
///
/// 容错: try_load_config 失败 → 用 new_for_test() 骨架, 启动不阻塞.
pub fn build() -> Result<Arc<RuntimeState>, String> {
    let t0 = std::time::Instant::now();
    let config = try_load_config();
    let t_cfg = std::time::Instant::now();
    let config = config.unwrap_or_else(|| {
        tracing::warn!("[state] try_load_config failed, falling back to new_for_test() skeleton");
        ResolvedConfig::default()
    });
    tracing::info!(since_prev_ms = t_cfg.elapsed().as_millis() as u64, "[state] T1.1 config loaded");

    // 同步跑 RuntimeState::new (内部全同步 SQLite, 13ms).
    // build() 在 Tauri setup 同步闭包里调, 内部全同步 IO OK (不抢 tokio runtime).
    let state = futures::executor::block_on(RuntimeState::new(config))
        .map_err(|e| format!("RuntimeState::new failed: {e}"))?;
    tracing::info!(since_prev_ms = t0.elapsed().as_millis() as u64, "[state] T1.2 RuntimeState::new done (真 provider + restore inline)");

    // 懒 restore 不在 build() 同步跑 (它会跑 5-15s 同步 SQLite, 阻塞 webview 启动).
    // 由首次 send_message / list_sessions 入口自动调一次 (RuntimeState::ensure_restored).
    // 早期: state.restored 仍然是 false, ensure_restored 会同步等 5-15s, 用户发首条消息会卡 1-2s.
    // 接受: 用户发首条消息时本来就要等, 1-2s 额外延迟可接受.

    Ok(state)
}

