use crate::agent::conversation::TokenBudget;
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
                max_tokens: Some(4096),
                temperature: None,
                thinking: ThinkingConfig::Disabled,
            },
            budget: TokenBudget {
                max_input_tokens: Some(100_000),
                max_output_tokens: Some(4096),
            },
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
        }
    }
}
