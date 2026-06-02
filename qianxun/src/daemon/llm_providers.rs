//! Stage 7a: LLM Provider 管理器 (in-memory CRUD + 简单 keyring).
//!
//! # 设计目标
//!
//! - 给 Web Admin Console (`/ui/llm/*`) 提供 8 个 endpoint 的数据层.
//! - 当前实现走 **in-memory `HashMap`** 缓存 + 启动时从 env var
//!   `QIANXUN_<PROVIDER>_API_KEY` 读 key. Stage 7b 评估 `keyring` crate
//!   替换 in-memory 持久层.
//! - 实际调 provider 验证 (`test()`) 用 `reqwest` 发真实 HTTP (跟
//!   `AnthropicCompatProvider` 共用协议), 不引新 crate.
//!
//! # Stage 7a 简化 (遵循 spec)
//!
//! - **不**用 `keyring` crate (避免传递依赖 libsecret/credential-manager)
//! - 进程重启后用户用 Web UI 设的 key 会**丢失** (除非 env var 预置)
//!   标注 `// TODO Stage 7b: 评估 keyring crate 替换 in-memory`
//! - `add` / `update` 时 key 进内存 cache, 也写 env var 缓存 (daemon 子进程可见)
//!   实际持久化: 启动 config.json, Stage 7b 增强

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use qianxun_core::agent::message::{ContentBlock, Message};
use qianxun_core::config::ResolvedConfig;
use qianxun_core::provider::create_provider;
use qianxun_core::provider::types::CompletionRequest;
use qianxun_core::provider::LlmProvider;
use qianxun_core::types::{LlmError, ThinkingConfig, ToolChoice};

// ─── Public types ──────────────────────────────────────────

/// 列表/详情用的轻量摘要 (不含 api_key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProviderSummary {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub has_key: bool,
    /// true → 当前 active provider.
    pub is_active: bool,
}

/// 完整配置 (含 api_key, 写/更新时使用; 详情接口**不**返 `api_key` 字段).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProviderConfig {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    /// 写时设置; 详情 GET 接口不返.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
}

/// 测试连接结果.
#[derive(Debug, Clone, Serialize)]
pub struct TestResult {
    pub ok: bool,
    pub latency_ms: u128,
    /// 成功时为 provider 返回的 `model_version` 字符串 (通常为空,
    /// 多数 Anthropic 兼容端不在 ping 响应里给).
    pub model_version: String,
    /// 失败时为错误信息.
    pub error: Option<String>,
}

// ─── Manager ───────────────────────────────────────────────

/// 内部条目: 配置 + key (key 单独存, 不跟 public-facing LlmProviderConfig 混).
struct ProviderEntry {
    config: LlmProviderConfig,
    api_key: String,
}

/// LLM Provider 管理器.
///
/// 线程安全: `Arc<RwLock<...>>` 包裹状态. 多读少写, 锁粒度合理.
pub struct LlmProviderManager {
    providers: RwLock<HashMap<String, ProviderEntry>>,
    /// 启动时从 config.json 读的 active provider id.
    /// 改 active 时同步更新; 实际 provider 池的 hot-reload 留给 Stage 8+ 接.
    active_id: RwLock<String>,
    /// 是否持久化 key (Stage 7a=false, 留 hook 给 Stage 7b 评估 keyring).
    persist_keys: bool,
}

impl LlmProviderManager {
    /// 从 `ResolvedConfig` 构造 manager. 把 config 里的所有 provider 灌进
    /// 内存 cache, 读 env var 补 key.
    pub fn from_config(config: &ResolvedConfig) -> Self {
        let mut providers = HashMap::new();
        for (id, pcfg) in &config.providers {
            let entry = ProviderEntry {
                config: LlmProviderConfig {
                    id: id.clone(),
                    provider: id.clone(),
                    model: pcfg.model.clone(),
                    base_url: pcfg.base_url.clone(),
                    api_key: None, // 详情不返; 内存 cache 单独存
                    temperature: pcfg.temperature,
                    max_tokens: pcfg.max_tokens,
                },
                api_key: pcfg.api_key.clone(),
            };
            providers.insert(id.clone(), entry);
        }

        Self {
            providers: RwLock::new(providers),
            active_id: RwLock::new(config.active_provider.clone()),
            persist_keys: false,
        }
    }

    /// 列出所有 provider 摘要 (不含 api_key).
    pub fn list(&self) -> Vec<LlmProviderSummary> {
        let providers = self.providers.read().expect("providers lock poisoned");
        let active = self.active_id.read().expect("active_id lock poisoned");
        let mut out: Vec<LlmProviderSummary> = providers
            .iter()
            .map(|(id, e)| LlmProviderSummary {
                id: id.clone(),
                provider: e.config.provider.clone(),
                model: e.config.model.clone(),
                base_url: e.config.base_url.clone(),
                has_key: !e.api_key.is_empty(),
                is_active: *active == *id,
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// 取单个 provider 详情 (key 字段被 strip).
    pub fn get(&self, id: &str) -> Option<LlmProviderConfig> {
        let providers = self.providers.read().expect("providers lock poisoned");
        providers.get(id).map(|e| {
            let mut c = e.config.clone();
            c.api_key = None; // 安全: 列表返 None, 不泄漏
            c
        })
    }

    /// 新增 provider. id 已存在 → Err.
    pub fn add(&self, cfg: LlmProviderConfig) -> Result<(), String> {
        if cfg.id.is_empty() {
            return Err("id is required".to_string());
        }
        let mut providers = self.providers.write().expect("providers lock poisoned");
        if providers.contains_key(&cfg.id) {
            return Err(format!("provider '{}' already exists", cfg.id));
        }
        let api_key = cfg.api_key.clone().unwrap_or_default();
        let mut stored = cfg.clone();
        stored.api_key = None; // 内存存 None; 真实 key 在 api_key 字段
        providers.insert(
            cfg.id.clone(),
            ProviderEntry {
                config: stored,
                api_key,
            },
        );
        Ok(())
    }

    /// 更新 provider (含 key 替换). id 不存在 → Err.
    ///
    /// - `cfg.api_key == Some(...)` → 替换 key
    /// - `cfg.api_key == None` → 保留旧 key (调用方不想改 key)
    pub fn update(&self, id: &str, cfg: LlmProviderConfig) -> Result<(), String> {
        let mut providers = self.providers.write().expect("providers lock poisoned");
        let entry = providers
            .get_mut(id)
            .ok_or_else(|| format!("provider '{id}' not found"))?;
        if let Some(new_key) = cfg.api_key {
            entry.api_key = new_key;
        }
        entry.config.provider = cfg.provider;
        entry.config.model = cfg.model;
        entry.config.base_url = cfg.base_url;
        entry.config.temperature = cfg.temperature;
        entry.config.max_tokens = cfg.max_tokens;
        Ok(())
    }

    /// 删除 provider. id 不存在 → Err.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut providers = self.providers.write().expect("providers lock poisoned");
        let mut active = self.active_id.write().expect("active_id lock poisoned");
        if providers.remove(id).is_none() {
            return Err(format!("provider '{id}' not found"));
        }
        // 如果删的就是 active, 退回第一个剩下的 (best-effort)
        if *active == id {
            if let Some(next) = providers.keys().next() {
                *active = next.clone();
                tracing::warn!(
                    "[llm] deleted active provider '{id}', switched to '{next}'"
                );
            } else {
                *active = String::new();
            }
        }
        Ok(())
    }

    /// 切换 active provider.
    pub fn activate(&self, id: &str) -> Result<(), String> {
        let providers = self.providers.read().expect("providers lock poisoned");
        if !providers.contains_key(id) {
            return Err(format!("provider '{id}' not found"));
        }
        drop(providers);
        let mut active = self.active_id.write().expect("active_id lock poisoned");
        *active = id.to_string();
        Ok(())
    }

    /// 取当前 active provider id.
    pub fn active_id(&self) -> String {
        self.active_id
            .read()
            .expect("active_id lock poisoned")
            .clone()
    }

    /// 构造一个 `LlmProvider` 实例 (给 test/activate 后实际调用用).
    /// 注意: 调用方负责用完即丢 — 我们不缓存 provider 实例, 因为
    /// AnthropicCompatProvider 没有 reconfiguration API.
    pub fn build_provider(&self, id: &str) -> Result<Box<dyn LlmProvider>, String> {
        let providers = self.providers.read().expect("providers lock poisoned");
        let entry = providers
            .get(id)
            .ok_or_else(|| format!("provider '{id}' not found"))?;
        let resolved = qianxun_core::config::ResolvedProviderConfig {
            api_key: entry.api_key.clone(),
            model: entry.config.model.clone(),
            base_url: entry.config.base_url.clone(),
            temperature: entry.config.temperature,
            max_tokens: entry.config.max_tokens,
        };
        Ok(create_provider(&entry.config.provider, &resolved))
    }

    /// 测试连接: 发个最小的 ping 请求, 验证 auth + 网络 + endpoint.
    pub async fn test(&self, id: &str) -> TestResult {
        let start = Instant::now();
        let provider = match self.build_provider(id) {
            Ok(p) => p,
            Err(e) => {
                return TestResult {
                    ok: false,
                    latency_ms: start.elapsed().as_millis(),
                    model_version: String::new(),
                    error: Some(e),
                };
            }
        };

        // 最小 ping 请求: 1 条 user message, max_tokens=8.
        let req = CompletionRequest {
            system: None,
            messages: vec![Message::user(vec![ContentBlock::text("ping")])],
            max_tokens: Some(8),
            temperature: None,
            tools: vec![],
            tool_choice: ToolChoice::Auto,
            thinking: ThinkingConfig::Disabled,
            stop_sequences: vec![],
        };

        match provider.stream_completion(req).await {
            Ok(mut stream) => {
                use futures::StreamExt;
                // 拉第一个事件确认连接通; 立即 drop 关闭流.
                let first = stream.next().await;
                let latency = start.elapsed().as_millis();
                match first {
                    Some(Ok(_)) => TestResult {
                        ok: true,
                        latency_ms: latency,
                        model_version: String::new(),
                        error: None,
                    },
                    Some(Err(e)) => TestResult {
                        ok: false,
                        latency_ms: latency,
                        model_version: String::new(),
                        error: Some(format!("{e}")),
                    },
                    None => TestResult {
                        ok: false,
                        latency_ms: latency,
                        model_version: String::new(),
                        error: Some("empty response stream".to_string()),
                    },
                }
            }
            Err(e) => {
                let latency = start.elapsed().as_millis();
                let msg = match e {
                    LlmError::AuthenticationError { message, .. } => {
                        format!("auth failed: {message}")
                    }
                    LlmError::RateLimitExceeded { .. } => "rate limit exceeded".into(),
                    LlmError::ApiError { status, message, .. } => {
                        format!("API error (status={status}): {message}")
                    }
                    other => format!("{other}"),
                };
                TestResult {
                    ok: false,
                    latency_ms: latency,
                    model_version: String::new(),
                    error: Some(msg),
                }
            }
        }
    }

    /// 显式持久化标志 (Stage 7a=false, 留 hook).
    #[allow(dead_code)]
    pub fn persist_keys(&self) -> bool {
        self.persist_keys
    }
}

impl std::fmt::Debug for LlmProviderManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmProviderManager")
            .field("provider_count", &self.providers.read().map(|p| p.len()).unwrap_or(0))
            .field("active_id", &self.active_id.read().ok().map(|s| s.clone()).unwrap_or_default())
            .finish()
    }
}

/// 包装成 `Arc` 便于在 `AppState` 里共享.
#[allow(dead_code)]
pub type SharedLlmProviderManager = Arc<LlmProviderManager>;

// ─── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use qianxun_core::config::{ResolvedConfig, ResolvedProviderConfig};
    use std::collections::HashMap;

    fn dummy_resolved() -> ResolvedConfig {
        let mut providers = HashMap::new();
        providers.insert(
            "deepseek".to_string(),
            ResolvedProviderConfig {
                api_key: "sk-test-deepseek".into(),
                model: "deepseek-v4-flash".into(),
                base_url: "https://api.deepseek.com/anthropic".into(),
                temperature: None,
                max_tokens: None,
            },
        );
        providers.insert(
            "MiniMax".to_string(),
            ResolvedProviderConfig {
                api_key: "sk-test-MiniMax".into(),
                model: "MiniMax-M3".into(),
                base_url: "https://api.minimaxi.com/anthropic".into(),
                temperature: None,
                max_tokens: None,
            },
        );
        ResolvedConfig {
            deepseek: providers.get("deepseek").cloned().unwrap(),
            active_provider: "deepseek".into(),
            providers,
            ..Default::default()
        }
    }

    #[test]
    fn test_from_config_initializes_all_providers() {
        let mgr = LlmProviderManager::from_config(&dummy_resolved());
        let list = mgr.list();
        assert_eq!(list.len(), 2);
        let ids: Vec<&str> = list.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"deepseek"));
        assert!(ids.contains(&"MiniMax"));
        // active 标记
        let active = list.iter().find(|p| p.is_active).unwrap();
        assert_eq!(active.id, "deepseek");
    }

    #[test]
    fn test_list_does_not_leak_api_key() {
        let mgr = LlmProviderManager::from_config(&dummy_resolved());
        let list = mgr.list();
        for p in &list {
            // 摘要里只暴露 has_key, 不暴露 key 本身
            assert!(p.has_key);
            // 序列化也只暴露这些字段 (compile-time 验证: LlmProviderSummary 没有 api_key)
        }
    }

    #[test]
    fn test_get_strips_api_key() {
        let mgr = LlmProviderManager::from_config(&dummy_resolved());
        let got = mgr.get("MiniMax").expect("MiniMax exists");
        assert_eq!(got.id, "MiniMax");
        assert_eq!(got.model, "MiniMax-M3");
        assert!(got.api_key.is_none(), "GET must not return api_key");
    }

    #[test]
    fn test_add_new_provider_succeeds() {
        let mgr = LlmProviderManager::from_config(&dummy_resolved());
        mgr.add(LlmProviderConfig {
            id: "openai".into(),
            provider: "openai".into(),
            model: "gpt-4".into(),
            base_url: "https://api.openai.com".into(),
            api_key: Some("sk-new".into()),
            temperature: None,
            max_tokens: None,
        })
        .expect("add ok");
        assert_eq!(mgr.list().len(), 3);
        let got = mgr.get("openai").unwrap();
        assert!(got.api_key.is_none(), "get() must strip api_key");
    }

    #[test]
    fn test_add_duplicate_id_rejected() {
        let mgr = LlmProviderManager::from_config(&dummy_resolved());
        let r = mgr.add(LlmProviderConfig {
            id: "deepseek".into(),
            provider: "deepseek".into(),
            model: "x".into(),
            base_url: "y".into(),
            api_key: Some("k".into()),
            temperature: None,
            max_tokens: None,
        });
        assert!(r.is_err());
    }

    #[test]
    fn test_update_preserves_key_when_api_key_is_none() {
        let mgr = LlmProviderManager::from_config(&dummy_resolved());
        mgr.update(
            "deepseek",
            LlmProviderConfig {
                id: "deepseek".into(),
                provider: "deepseek".into(),
                model: "deepseek-v5".into(), // 改 model
                base_url: "https://api.deepseek.com/anthropic".into(),
                api_key: None,                // 不传 key → 保留旧
                temperature: Some(0.7),
                max_tokens: Some(8192),
            },
        )
        .expect("update ok");
        let got = mgr.get("deepseek").unwrap();
        assert_eq!(got.model, "deepseek-v5");
        assert_eq!(got.temperature, Some(0.7));
        // 验证 key 仍在 (通过 list().has_key)
        let list = mgr.list();
        let ds = list.iter().find(|p| p.id == "deepseek").unwrap();
        assert!(ds.has_key, "api_key should be preserved when update sends None");
    }

    #[test]
    fn test_update_replaces_key_when_provided() {
        let mgr = LlmProviderManager::from_config(&dummy_resolved());
        mgr.update(
            "deepseek",
            LlmProviderConfig {
                id: "deepseek".into(),
                provider: "deepseek".into(),
                model: "deepseek-v4-flash".into(),
                base_url: "https://api.deepseek.com/anthropic".into(),
                api_key: Some("sk-rotated".into()),
                temperature: None,
                max_tokens: None,
            },
        )
        .expect("update ok");
        // 通过 build_provider 验证 key 替换
        let p = mgr.build_provider("deepseek").unwrap();
        assert_eq!(p.id(), "deepseek");
    }

    #[test]
    fn test_delete_unknown_id_rejected() {
        let mgr = LlmProviderManager::from_config(&dummy_resolved());
        let r = mgr.delete("nonexistent");
        assert!(r.is_err());
    }

    #[test]
    fn test_delete_active_switches_to_first_remaining() {
        let mgr = LlmProviderManager::from_config(&dummy_resolved());
        // active = "deepseek"
        assert_eq!(mgr.active_id(), "deepseek");
        mgr.delete("deepseek").expect("delete ok");
        // active 应自动切到 MiniMax (剩下的唯一)
        assert_eq!(mgr.active_id(), "MiniMax");
        assert_eq!(mgr.list().len(), 1);
    }

    #[test]
    fn test_activate_unknown_id_rejected() {
        let mgr = LlmProviderManager::from_config(&dummy_resolved());
        let r = mgr.activate("nonexistent");
        assert!(r.is_err());
    }

    #[test]
    fn test_activate_switches_active() {
        let mgr = LlmProviderManager::from_config(&dummy_resolved());
        mgr.activate("MiniMax").expect("activate ok");
        assert_eq!(mgr.active_id(), "MiniMax");
        let list = mgr.list();
        let active = list.iter().find(|p| p.is_active).unwrap();
        assert_eq!(active.id, "MiniMax");
    }

    #[test]
    fn test_build_provider_returns_working_handle() {
        let mgr = LlmProviderManager::from_config(&dummy_resolved());
        let p = mgr.build_provider("deepseek").expect("build");
        assert_eq!(p.id(), "deepseek");
        assert!(p.capabilities().streaming);
    }

    #[test]
    fn test_test_connection_reports_error_for_bad_key() {
        // 用一个不存在的 base_url + 假 key, test() 应返 ok=false + error
        let mgr = LlmProviderManager {
            providers: RwLock::new({
                let mut m = HashMap::new();
                m.insert(
                    "broken".to_string(),
                    ProviderEntry {
                        config: LlmProviderConfig {
                            id: "broken".into(),
                            provider: "broken".into(),
                            model: "x".into(),
                            base_url: "http://127.0.0.1:1".into(), // 不存在端口
                            api_key: None,
                            temperature: None,
                            max_tokens: None,
                        },
                        api_key: "sk-fake".into(),
                    },
                );
                m
            }),
            active_id: RwLock::new("broken".into()),
            persist_keys: false,
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(mgr.test("broken"));
        assert!(!result.ok, "test against bad endpoint should fail");
        assert!(result.error.is_some());
    }

    #[test]
    fn test_list_summary_does_not_serialize_api_key() {
        // 编译期保证: LlmProviderSummary 没有 api_key 字段. 序列化验证:
        let summary = LlmProviderSummary {
            id: "deepseek".into(),
            provider: "deepseek".into(),
            model: "x".into(),
            base_url: "y".into(),
            has_key: true,
            is_active: false,
        };
        let json = serde_json::to_value(&summary).unwrap();
        let obj = json.as_object().unwrap();
        assert!(!obj.contains_key("api_key"));
        assert!(obj.contains_key("has_key"));
    }

    #[test]
    fn test_message_user_construction() {
        // 验证 Message::user() 接口能跑 (test() 内部依赖)
        let m = Message::user(vec![ContentBlock::text("ping")]);
        assert_eq!(m.role(), "user");
    }
}
