//! 向后兼容的 `DeepSeekProvider` 类型别名。
//!
//! 历史实现已迁移至 [`AnthropicCompatProvider`](super::anthropic_compat::AnthropicCompatProvider),
//! 因为 DeepSeek / MiniMax-M3 / 其他 Anthropic 兼容服务共享同一协议。
//!
//! 保留此模块以避免破坏外部 `use qianxun_core::provider::deepseek::DeepSeekProvider` 引用。
//! 内部实现 = 通用 Anthropic 兼容 provider, 通过 `create_provider("deepseek", &cfg)` 实例化。

pub use super::anthropic_compat::AnthropicCompatProvider as DeepSeekProvider;
