use crate::agent::conversation::TokenBudget;
use crate::agent::context::window::TokenScope;
use crate::types::{AgentConfig, ThinkingConfig};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

// ─── Raw config (from JSON5 file) ─────────────────────────

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub providers: Option<HashMap<String, ProviderConfig>>,
    pub agent: Option<AgentDefaults>,
    pub budget: Option<BudgetConfig>,
    pub compaction: Option<CompactionConfig>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct AgentDefaults {
    pub max_turns: Option<u32>,
    pub max_retries: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct BudgetConfig {
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CompactScope {
    Total,
    #[default]
    BodyAfterPrefix,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct CompactionConfig {
    pub enabled: Option<bool>,
    pub model_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub snip_fresh_turns: Option<usize>,
    pub micro_compact_keep: Option<usize>,
    pub micro_compact_ttl_secs: Option<u64>,
    pub collapse_ratio: Option<f64>,
    pub block_ratio: Option<f64>,
    pub auto_compact_ratio: Option<f64>,
    pub circuit_breaker_limit: Option<u32>,
    pub scope: Option<String>,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            model_window: Some(1_000_000),
            max_output_tokens: None,
            snip_fresh_turns: Some(3),
            micro_compact_keep: Some(20),
            micro_compact_ttl_secs: Some(60),
            collapse_ratio: Some(0.90),
            block_ratio: Some(0.95),
            auto_compact_ratio: Some(0.85),
            circuit_breaker_limit: Some(3),
            scope: Some("body_after_prefix".into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedCompactionConfig {
    pub enabled: bool,
    pub model_window: u64,
    pub effective_window: u64,
    pub snip_fresh_turns: usize,
    pub micro_compact_keep: usize,
    pub micro_compact_ttl: std::time::Duration,
    pub collapse_ratio: f64,
    pub block_ratio: f64,
    pub auto_compact_ratio: f64,
    pub circuit_breaker_limit: u32,
    pub scope: TokenScope,
    pub max_output_tokens: u64,
}

impl Default for ResolvedCompactionConfig {
    fn default() -> Self {
        let model_window = 1_000_000u64;
        let max_output_tokens = 16384u64;
        let effective_window = model_window - max_output_tokens.min(20_000);
        Self {
            enabled: true,
            model_window,
            effective_window,
            snip_fresh_turns: 3,
            micro_compact_keep: 20,
            micro_compact_ttl: std::time::Duration::from_secs(60),
            collapse_ratio: 0.90,
            block_ratio: 0.95,
            auto_compact_ratio: 0.85,
            circuit_breaker_limit: 3,
            scope: TokenScope::BodyAfterPrefix,
            max_output_tokens,
        }
    }
}

// ─── Resolved config ──────────────────────────────────────

#[derive(Clone)]
pub struct ResolvedProviderConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u64>,
}

#[derive(Clone)]
pub struct ResolvedConfig {
    pub deepseek: ResolvedProviderConfig,
    pub agent: AgentConfig,
    pub budget: TokenBudget,
    pub compaction: ResolvedCompactionConfig,
}

// ─── Defaults ─────────────────────────────────────────────

impl Default for ResolvedConfig {
    fn default() -> Self {
        Self {
            deepseek: ResolvedProviderConfig {
                api_key: String::new(),
                model: "deepseek-v4-flash".into(),
                base_url: "https://api.deepseek.com/anthropic".into(),
                temperature: None,
                max_tokens: None,
            },
            agent: AgentConfig {
                max_turns: 50,
                max_retries: 3,
                max_tokens: Some(16384),
                temperature: None,
                thinking: ThinkingConfig::Disabled,
            },
            budget: TokenBudget {
                max_input_tokens: Some(100_000),
                max_output_tokens: Some(16384),
            },
            compaction: ResolvedCompactionConfig::default(),
        }
    }
}

// ─── Error ────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("failed to parse config file {path}: {source}")]
    Parse { path: String, source: json5::Error },
}

// ─── Implementation ───────────────────────────────────────

impl Config {
    /// Parse a JSON5 config file. Returns empty Config if file not found.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(e) => {
                return Err(ConfigError::Io {
                    path: path.display().to_string(),
                    source: e,
                });
            }
        };

        if content.trim().is_empty() {
            return Ok(Self::default());
        }

        json5::from_str(&content).map_err(|e| ConfigError::Parse {
            path: path.display().to_string(),
            source: e,
        })
    }

    /// Resolve config with env/CLI overrides.
    ///
    /// Priority: CLI args > Env vars > Config file > Built-in defaults
    pub fn resolve(
        self,
        env_api_key: Option<String>,
        cli_model: Option<String>,
    ) -> ResolvedConfig {
        let defaults = ResolvedConfig::default();
        let pcfg = self
            .providers
            .and_then(|mut m| m.remove("deepseek"))
            .unwrap_or_default();

        // Priority chain
        let api_key = env_api_key.or(pcfg.api_key).unwrap_or_default();
        let model = cli_model
            .or(pcfg.model)
            .unwrap_or_else(|| defaults.deepseek.model.clone());
        let base_url = pcfg
            .base_url
            .unwrap_or_else(|| defaults.deepseek.base_url.clone());
        let temperature = pcfg.temperature.or(defaults.deepseek.temperature);
        let provider_max_tokens = pcfg.max_tokens.or(defaults.agent.max_tokens);

        let max_turns = self
            .agent
            .as_ref()
            .and_then(|a| a.max_turns)
            .unwrap_or(defaults.agent.max_turns);
        let max_retries = self
            .agent
            .as_ref()
            .and_then(|a| a.max_retries)
            .unwrap_or(defaults.agent.max_retries);

        let max_input_tokens = self
            .budget
            .as_ref()
            .and_then(|b| b.max_input_tokens)
            .or(defaults.budget.max_input_tokens);
        let max_output_tokens = self
            .budget
            .as_ref()
            .and_then(|b| b.max_output_tokens)
            .or(defaults.budget.max_output_tokens);

        // Resolve compaction config
        let raw_compaction = self.compaction.unwrap_or_default();
        let model_window = raw_compaction.model_window.unwrap_or(defaults.compaction.model_window);
        let compaction_max_output = raw_compaction.max_output_tokens
            .or(max_output_tokens)
            .unwrap_or(defaults.compaction.max_output_tokens);
        let effective_window = model_window - compaction_max_output.min(20_000);
        let scope = match raw_compaction.scope.as_deref() {
            Some("total") => TokenScope::Total,
            _ => TokenScope::BodyAfterPrefix,
        };

        ResolvedConfig {
            deepseek: ResolvedProviderConfig {
                api_key,
                model,
                base_url,
                temperature,
                max_tokens: provider_max_tokens,
            },
            agent: AgentConfig {
                max_turns,
                max_retries,
                max_tokens: provider_max_tokens,
                temperature,
                thinking: ThinkingConfig::Disabled,
            },
            budget: TokenBudget {
                max_input_tokens,
                max_output_tokens,
            },
            compaction: ResolvedCompactionConfig {
                enabled: raw_compaction.enabled.unwrap_or(true),
                model_window,
                effective_window,
                snip_fresh_turns: raw_compaction.snip_fresh_turns.unwrap_or(defaults.compaction.snip_fresh_turns),
                micro_compact_keep: raw_compaction.micro_compact_keep.unwrap_or(defaults.compaction.micro_compact_keep),
                micro_compact_ttl: std::time::Duration::from_secs(
                    raw_compaction.micro_compact_ttl_secs.unwrap_or(60),
                ),
                collapse_ratio: raw_compaction.collapse_ratio.unwrap_or(defaults.compaction.collapse_ratio),
                block_ratio: raw_compaction.block_ratio.unwrap_or(defaults.compaction.block_ratio),
                auto_compact_ratio: raw_compaction.auto_compact_ratio.unwrap_or(defaults.compaction.auto_compact_ratio),
                circuit_breaker_limit: raw_compaction.circuit_breaker_limit.unwrap_or(defaults.compaction.circuit_breaker_limit),
                scope,
                max_output_tokens: compaction_max_output,
            },
        }
    }
}
