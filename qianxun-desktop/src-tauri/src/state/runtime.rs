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
//   - 同步 build() 包 tauri::async_runtime::block_on (Tauri setup 是 sync 闭包)

use std::sync::Arc;

use qianxun_core::config::{Config, ResolvedConfig};
use qianxun_runtime::RuntimeState;

/// 加载 ~/.qianxun/config.json, 失败返 None (不抛错, 走 fallback).
///
/// 用 Config::from_file 跟 qianxun binary 走同一条路径 (含注释解析),
/// 然后 resolve() 跑 env 覆盖 + provider 合并, 返 ResolvedConfig.
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
/// 成功 → 返 Arc<RuntimeState> 真业务 (provider/tools/memory/skills/store/agent_host 全部就位)
/// 失败 → fallback 返 new_for_test (in-memory, desktop 仍可启动, 设置面板可用)
/// 全失败 → 返 Err (lib.rs setup 收到后打 warn, 进程不 panic)
///
/// 内部用 tauri::async_runtime::block_on 跑 async build() (Tauri 2.x async_runtime 是 tokio re-export).
pub fn build() -> Result<Arc<RuntimeState>, String> {
    tauri::async_runtime::block_on(async {
        let config = try_load_config().unwrap_or_default();
        match RuntimeState::new(config).await {
            Ok(rt) => {
                tracing::info!("[state] RuntimeState initialized from config");
                Ok(rt)
            }
            Err(e) => {
                tracing::warn!(
                    "[state] RuntimeState::new failed: {e}, falling back to in-memory test state"
                );
                Ok(RuntimeState::new_for_test())
            }
        }
    })
}
