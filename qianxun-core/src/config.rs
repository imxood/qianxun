use crate::agent::conversation::TokenBudget;
use crate::agent::context::window::TokenScope;
use crate::types::{AgentConfig, AgentPattern, PlanAndExecuteConfig, ReflectiveConfig, WorkflowConfig, ThinkingConfig};
use serde::{Deserialize, Serialize};
use json_comments::StripComments;
use std::collections::HashMap;
use std::path::Path;

// ─── Raw config (from JSON file with comments) ────────────

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Config {
    /// 当前激活的 provider 名称. 留空则默认 "deepseek".
    /// 优先级: CLI --provider > env QIANXUN_ACTIVE_PROVIDER > 本字段 > "deepseek"
    pub active_provider: Option<String>,
    pub providers: Option<HashMap<String, ProviderConfig>>,
    pub agent: Option<AgentDefaults>,
    pub budget: Option<BudgetConfig>,
    pub compaction: Option<CompactionConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct AgentDefaults {
    pub max_turns: Option<u32>,
    pub max_retries: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
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

#[derive(Debug, Deserialize, Serialize)]
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
    /// 向后兼容: 始终等于 active provider 的 config (如果 active 是 deepseek, 仍是 deepseek).
    /// 旧代码用 `resolved.deepseek` 直接取 config, 仍能工作.
    pub deepseek: ResolvedProviderConfig,
    /// 当前激活的 provider 名称. 例如 "deepseek" / "MiniMax" / 自定义.
    pub active_provider: String,
    /// 全部 provider 解析后的 ResolvedProviderConfig, 按名称索引.
    pub providers: HashMap<String, ResolvedProviderConfig>,
    pub agent: AgentConfig,
    pub budget: TokenBudget,
    pub compaction: ResolvedCompactionConfig,
}

impl ResolvedConfig {
    /// 获取当前激活 provider 的 config.
    /// 若 active_provider 不在 providers HashMap 中 (罕见), 回退到 deepseek 字段.
    pub fn active_provider_config(&self) -> ResolvedProviderConfig {
        self.providers
            .get(&self.active_provider)
            .cloned()
            .unwrap_or_else(|| self.deepseek.clone())
    }
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
            active_provider: "deepseek".into(),
            providers: HashMap::new(),
            agent: AgentConfig {
                max_turns: 50,
                max_retries: 3,
                max_tokens: Some(16384),
                temperature: None,
                thinking: ThinkingConfig::Disabled,
                pattern: AgentPattern::default(),
                plan_and_execute: PlanAndExecuteConfig::default(),
                reflective: ReflectiveConfig::default(),
                workflow: WorkflowConfig::default(),
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
    Parse { path: String, source: serde_json::Error },
}

// ─── Implementation ───────────────────────────────────────

impl Config {
    /// Parse a JSON config file (supports `//` and `/* */` comments).
    /// Returns empty Config if file not found.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
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

        let reader = StripComments::new(file);
        serde_json::from_reader(reader).map_err(|e| ConfigError::Parse {
            path: path.display().to_string(),
            source: e,
        })
    }

    /// 原子写 config 文件 (写 tmp → rename, 防半写文件).
    /// 2026-06-09 加: 桌面端 Provider 设置 UI 写配置.
    ///
    /// 行为:
    /// 1. 序列化 Config 为 JSON (缩进 2 空格, 含原注释风格 — 通过 StripComments writer 不必要)
    /// 2. 写到 `<path>.tmp` 同目录
    /// 3. `std::fs::rename` 原子替换原文件
    /// 4. 失败时清理 tmp
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        use std::io::Write;
        let path = path.as_ref();
        let tmp = path.with_extension("json.tmp");

        // 1. 序列化
        let json = serde_json::to_string_pretty(self).map_err(|e| {
            ConfigError::Parse {
                path: path.display().to_string(),
                source: e,
            }
        })?;

        // 2. 写 tmp
        {
            let mut f = std::fs::File::create(&tmp).map_err(|e| ConfigError::Io {
                path: tmp.display().to_string(),
                source: e,
            })?;
            f.write_all(json.as_bytes()).map_err(|e| ConfigError::Io {
                path: tmp.display().to_string(),
                source: e,
            })?;
            f.sync_all().map_err(|e| ConfigError::Io {
                path: tmp.display().to_string(),
                source: e,
            })?;
        }

        // 3. 原子 rename
        if let Err(e) = std::fs::rename(&tmp, path) {
            // 清理 tmp
            let _ = std::fs::remove_file(&tmp);
            return Err(ConfigError::Io {
                path: path.display().to_string(),
                source: e,
            });
        }
        Ok(())
    }

    /// Resolve config with env/CLI overrides.
    ///
    /// 优先级 (从高到低):
    ///   1. CLI `--provider` / `--model`
    ///   2. Env `QIANXUN_ACTIVE_PROVIDER` (仅 active_provider)
    ///   3. 配置文件 `active_provider` / `providers.<name>.*`
    ///   4. 内置默认值 (`"deepseek"`)
    ///
    /// 每个 provider 的 API key 查找顺序 (在 `resolve()` 内部, 不依赖外部传参):
    ///   1. 预设的硬编码 env var (例如 MiniMax → `ANTHROPIC_AUTH_TOKEN`)
    ///   2. 通用约定 `<PROVIDER>_API_KEY`
    ///   3. 通用约定 `<PROVIDER>_AUTH_TOKEN`
    ///   4. 配置文件 `providers.<name>.api_key`
    pub fn resolve(
        self,
        cli_model: Option<String>,
        cli_provider: Option<String>,
    ) -> ResolvedConfig {
        let defaults = ResolvedConfig::default();

        // ── 1. 解析 active_provider ──
        let active_provider = cli_provider
            .or_else(|| std::env::var("QIANXUN_ACTIVE_PROVIDER").ok())
            .or_else(|| self.active_provider.clone())
            .unwrap_or_else(|| "deepseek".to_string());

        // ── 2. 解析所有 raw providers → ResolvedProviderConfig ──
        let mut all_resolved: HashMap<String, ResolvedProviderConfig> = HashMap::new();
        if let Some(raw_providers) = self.providers {
            for (name, raw_pcfg) in raw_providers {
                let env_key = env_api_key_for(&name);
                let api_key = env_key
                    .or_else(|| raw_pcfg.api_key.clone())
                    .unwrap_or_default();

                let base_url = raw_pcfg
                    .base_url
                    .clone()
                    .unwrap_or_else(|| default_base_url_for(&name));

                let model = raw_pcfg
                    .model
                    .clone()
                    .unwrap_or_else(|| default_model_for(&name));

                all_resolved.insert(
                    name,
                    ResolvedProviderConfig {
                        api_key,
                        model,
                        base_url,
                        temperature: raw_pcfg.temperature,
                        max_tokens: raw_pcfg.max_tokens,
                    },
                );
            }
        }

        // ── 3. 保证激活的 provider 至少有一个 config (即使 config 文件没定义) ──
        if !all_resolved.contains_key(&active_provider) {
            let default_cfg = ResolvedProviderConfig {
                api_key: env_api_key_for(&active_provider).unwrap_or_default(),
                model: default_model_for(&active_provider),
                base_url: default_base_url_for(&active_provider),
                temperature: None,
                max_tokens: None,
            };
            all_resolved.insert(active_provider.clone(), default_cfg);
        }

        // ── 4. 取出 active config ──
        let active_cfg = all_resolved.get(&active_provider).cloned().unwrap();

        // CLI --model 覆盖 active provider 的 model
        let active_cfg = ResolvedProviderConfig {
            model: cli_model.unwrap_or_else(|| active_cfg.model.clone()),
            ..active_cfg
        };
        // 回写到 HashMap
        all_resolved.insert(active_provider.clone(), active_cfg.clone());

        // ── 5. deepseek 字段向后兼容 ──
        // 始终等于 active provider 的 config (如果 deepseek 在 HashMap 中, 用 deepseek 的; 否则用 active)
        let deepseek_legacy = all_resolved
            .get("deepseek")
            .cloned()
            .unwrap_or_else(|| active_cfg.clone());

        // ── 6. Agent / Budget / Compaction 解析 ──
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

        let raw_compaction = self.compaction.unwrap_or_default();
        let model_window = raw_compaction.model_window.unwrap_or(defaults.compaction.model_window);
        let compaction_max_output = raw_compaction
            .max_output_tokens
            .or(max_output_tokens)
            .unwrap_or(defaults.compaction.max_output_tokens);
        let effective_window = model_window - compaction_max_output.min(20_000);
        let scope = match raw_compaction.scope.as_deref() {
            Some("total") => TokenScope::Total,
            _ => TokenScope::BodyAfterPrefix,
        };

        ResolvedConfig {
            deepseek: deepseek_legacy,
            active_provider,
            providers: all_resolved,
            agent: AgentConfig {
                max_turns,
                max_retries,
                max_tokens: active_cfg.max_tokens.or(max_output_tokens),
                temperature: active_cfg.temperature,
                thinking: ThinkingConfig::Disabled,
                pattern: AgentPattern::default(),
                plan_and_execute: PlanAndExecuteConfig::default(),
                reflective: ReflectiveConfig::default(),
                workflow: WorkflowConfig::default(),
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

// ─── Env / Default helpers ─────────────────────────────────

/// 按 provider 名称查找 API key 环境变量.
fn env_api_key_for(provider_name: &str) -> Option<String> {
    // 1. 预设的硬编码 env var
    let specific: &[&str] = match provider_name {
        "deepseek" => &["DEEPSEEK_API_KEY"],
        "MiniMax" => &["ANTHROPIC_AUTH_TOKEN"],
        _ => &[],
    };
    for var in specific {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    // 2. 通用约定: <PROVIDER>_API_KEY
    let generic = format!("{}_API_KEY", provider_name.to_uppercase());
    if let Ok(v) = std::env::var(&generic) {
        if !v.is_empty() {
            return Some(v);
        }
    }
    // 3. Anthropic 风格: <PROVIDER>_AUTH_TOKEN
    let anthropic = format!("{}_AUTH_TOKEN", provider_name.to_uppercase());
    if let Ok(v) = std::env::var(&anthropic) {
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

fn default_base_url_for(provider_name: &str) -> String {
    match provider_name {
        "deepseek" => "https://api.deepseek.com/anthropic".into(),
        "MiniMax" => "https://api.minimaxi.com/anthropic".into(),
        "anthropic" => "https://api.anthropic.com".into(),
        other => {
            tracing::warn!(
                "[config] unknown provider '{other}', set `base_url` explicitly in config"
            );
            String::new()
        }
    }
}

fn default_model_for(provider_name: &str) -> String {
    match provider_name {
        "deepseek" => "deepseek-v4-flash".into(),
        "MiniMax" => "MiniMax-M3".into(),
        "anthropic" => "claude-sonnet-4-5".into(),
        other => {
            tracing::warn!(
                "[config] unknown provider '{other}', set `model` explicitly in config"
            );
            other.to_string()
        }
    }
}
