use std::path::PathBuf;

const TEMPLATE: &str = r#"// 千寻 (Qianxun) 全局配置文件
//
// 优先级（从高到低）:
//   CLI 参数 (--model) > 环境变量 (DEEPSEEK_API_KEY) > 配置文件 > 内置默认值
//
// 所有字段均为可选项，缺失 = 使用对应级别的默认值。

{
  // ── Provider ──────────────────────────────────────────────
  "providers": {
    // "deepseek": {
    //   // API 密钥（可选，默认走 DEEPSEEK_API_KEY 环境变量）
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
  },

  // ── Agent ─────────────────────────────────────────────────
  "agent": {
    // 每轮对话最大交互次数
    "max_turns": 50,
    // LLM API 调用重试次数
    "max_retries": 3,
  },

  // ── Token 预算 ─────────────────────────────────────────
  "budget": {
    // 对话窗口上限（超过时自动丢弃早期消息）
    "max_input_tokens": 100000,
    // 单次响应 token 上限
    "max_output_tokens": 4096,
  },

  // ── 上下文压缩 ─────────────────────────────────────
  "compaction": {
    // 启用上下文压缩（默认 true）
    // "enabled": true,
    // 模型窗口大小 token 数（DeepSeek = 1M）
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
