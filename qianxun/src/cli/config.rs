use std::path::PathBuf;

const TEMPLATE: &str = r#"// 千寻 (Qianxun) 全局配置文件
//
// 优先级（从高到低）:
//   CLI 参数 (--provider / --model) > 配置文件 > 内置默认值
//
// 切换 LLM provider:
//   1. 修改下方 "active_provider" 字段为 "deepseek" / "MiniMax" 或其他
//   2. 在 "providers" 区块添加对应 provider 的 api_key / model / base_url
//   3. 也可通过 CLI 临时覆盖:  qx --provider MiniMax
//
// 2026-06-09 改: API key 强制从本配置文件读, 不再支持环境变量
//   - 之前支持 DEEPSEEK_API_KEY / MINIMAX_API_KEY / ANTHROPIC_AUTH_TOKEN / 通用 <PROVIDER>_API_KEY
//   - 用户决策: 桌面端启动不继承 shell env, 单一配置源更可控
//   - 路径: 下方 providers.<name>.api_key 唯一来源
//
// 所有字段均为可选项，缺失 = 使用对应级别的默认值。

{
  // ── 当前激活的 Provider ─────────────────────────────
  // 留空 → "deepseek" (向后兼容)
  "active_provider": "deepseek",

  // ── Provider 配置 ─────────────────────────────────
  "providers": {
    // "deepseek": {
    //   // API 密钥 (必填, 唯一来源)
    //   // "api_key": "sk-...",
    //
    //   // 模型名（可选，可被 --model CLI 参数覆盖）
    //   // "model": "deepseek-v4-flash",
    //
    //   // API 基础地址
    //   // "base_url": "https://api.deepseek.com/anthropic",
    //
    //   // 生成温度（可选，null = API 默认值）
    //   // "temperature": 0.7,
    //
    //   // 单次响应最大 token（可选）
    //   // "max_tokens": 4096,
    // },

    // "MiniMax": {
    //   // API 密钥 (必填, 唯一来源)
    //   // "api_key": "eyJ...",
    //
    //   // 模型名（可选，默认 MiniMax-M3, 支持 1M 上下文 + thinking + tool_use + 图片）
    //   // "model": "MiniMax-M3",
    //
    //   // API 基础地址（Anthropic 兼容端点）
    //   // "base_url": "https://api.minimaxi.com/anthropic",
    // },
  },

  // ── Agent ─────────────────────────────────────────
  "agent": {
    // 每轮对话最大交互次数
    "max_turns": 50,
    // LLM API 调用重试次数
    "max_retries": 3,
  },

  // ── Token 预算 ─────────────────────────────────
  "budget": {
    // 对话窗口上限（超过时自动丢弃早期消息）
    "max_input_tokens": 100000,
    // 单次响应 token 上限
    "max_output_tokens": 4096,
  },

  // ── 上下文压缩 ───────────────────────────────
  "compaction": {
    // 启用上下文压缩（默认 true）
    // "enabled": true,
    // 模型窗口大小 token 数（DeepSeek / MiniMax-M3 = 1M）
    // "model_window": 1000000,
    // 裁剪前保留的最近轮次数
    // "snip_fresh_turns": 3,
    // 微压缩保留的最后消息数
    // "micro_compact_keep": 20,
    // 微压缩 TTL 秒数
    // "micro_compact_ttl_secs": 60,
    // 折叠比率（0.0-1.0）
    // "collapse_ratio": 0.90,
    // 阻塞比率（0.0-1.0）
    // "block_ratio": 0.95,
    // 自动压缩触发比率（0.0-1.0）
    // "auto_compact_ratio": 0.85,
    // 断路器限制
    // "circuit_breaker_limit": 3,
    // 追踪范围: "body_after_prefix" 或 "total"
    // "scope": "body_after_prefix",
  },
}
"#;

/// 将默认配置模板写入全局配置路径。
/// 文件已存在时返回错误，避免覆盖已有配置。
pub fn write_default_config() -> Result<PathBuf, String> {
    let path = default_config_path().ok_or_else(|| {
        "无法确定配置文件路径：未设置 USERPROFILE 或 HOME 环境变量".to_string()
    })?;

    if path.exists() {
        return Err(format!(
            "配置文件已存在: {}。如需重新生成，请先删除该文件。",
            path.display()
        ));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!("无法创建配置目录 {}: {e}", parent.display())
        })?;
    }

    std::fs::write(&path, TEMPLATE).map_err(|e| {
        format!("无法写入配置文件 {}: {e}", path.display())
    })?;

    Ok(path)
}

/// 检测当前平台的默认配置文件路径。
///
/// 所有平台统一使用 `~/.qianxun/config.json`
pub fn default_config_path() -> Option<PathBuf> {
    qianxun_core::workspace::qianxun_dir().map(|d| d.join("config.json"))
}
