//! LLM 通道（docs/17 §7.4）：只走 qx-llm 桥——读 DSH 的 llm-pi-ai 配置与
//! 凭据，经 OpenAI 兼容协议直连上游。DSH 未运行/未配置时返回可分类错误，
//! 由出刊层转成 `llm-unavailable` 进度态（UI 红字提示，不静默降级）。
//!
//! 配置链（已实测解析）：
//! - `<dsh_home>/profiles/web/cordis.patch.yml` → `llm-pi-ai.config.providers`：
//!   provider → { models[], apiKeyEnv, baseURL? }；
//! - `<dsh_home>/.credentials.yaml` → `refs[apiKeyEnv]` → key；
//! - endpoint：provider 显式 `baseURL` 优先（如 qwen35-local），否则查内置映射
//!   （提取自 DSH pi-ai catalog providers/data/*.json，见 PROVIDER_ENDPOINTS）。
//!
//! M2a 只实现 openai-completions 协议；anthropic-messages 协议（kimi/minimax）M2b。

use std::collections::BTreeMap;

use serde::Deserialize;

use super::super::error::Error;

/// 单次补全请求（非流式；出刊场景无需逐 token）。
pub struct CompleteArgs {
    /// 本次使用的模型（triage/synthesis 用 flash，rewrite 用主模型）。
    pub model: String,
    pub system: String,
    pub user: String,
    /// 输出预算；thinking 已禁用时不被思考消耗。
    pub max_tokens: u32,
}

/// 解析后的可用通道：provider + 模型组 + endpoint + key。
#[derive(Debug, Clone)]
pub struct LlmChannel {
    pub provider: String,
    /// flash 级模型（triage/synthesis：分类任务，便宜优先）。
    pub model: String,
    /// 写作模型（rewrite：优先非 flash 主模型，质量优先；无则同 model）。
    pub writing_model: String,
    pub base_url: String,
    pub api_key: String,
    /// GLM 系思考模型：思考 token 计入 max_tokens，调用时显式禁用
    /// （已实测不传则 rewrite 2048 预算被思考耗尽 → content 空）。
    pub thinking_disabled: bool,
}

/// 通道不可用的分类原因（转用户可见提示）。
#[derive(Debug, Clone)]
pub enum LlmUnavailable {
    /// cordis.patch.yml 里没有 llm-pi-ai providers。
    NoProvider,
    /// provider 存在但凭据里没有 key。
    NoApiKey { provider: String },
    /// provider 全是 M2a 未支持的协议（anthropic-messages 等）。
    UnsupportedProtocol { provider: String },
    /// 配置文件读取失败。
    ConfigRead(String),
}

impl std::fmt::Display for LlmUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoProvider => write!(f, "DSH 未配置 LLM 提供方（cordis.patch.yml 无 llm-pi-ai）"),
            Self::NoApiKey { provider } => {
                write!(f, "提供方 {provider} 缺少 API key（.credentials.yaml）")
            }
            Self::UnsupportedProtocol { provider } => {
                write!(f, "提供方 {provider} 的协议暂不支持（仅 OpenAI 兼容）")
            }
            Self::ConfigRead(cause) => write!(f, "DSH LLM 配置读取失败：{cause}"),
        }
    }
}

/// 内置 provider → OpenAI 兼容 endpoint（提取自 DSH pi-ai catalog
/// providers/data/*.json，2026-10 版本）。显式 baseURL 优先于此表。
fn provider_endpoint(provider: &str) -> Option<&'static str> {
    match provider {
        "zai-coding-cn" => Some("https://open.bigmodel.cn/api/coding/paas/v4"),
        "deepseek" => Some("https://api.deepseek.com"),
        "openai" => Some("https://api.openai.com/v1"),
        _ => None,
    }
}

/// cordis.patch.yml 里 llm-pi-ai 的最小解析结构（只取需要字段，其余忽略）。
/// cordis.patch.yml 顶层是数组，节点两种形态（实测 2026-10）：
/// `{insert: [entry…]}` 与直接 `{id, name?, config?}`——untagged 双形。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PatchNode {
    Insert { insert: Vec<PatchEntry> },
    Entry(PatchEntry),
}

#[derive(Debug, Deserialize)]
struct PatchEntry {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    config: Option<serde_yaml::Value>,
}

#[derive(Debug, Deserialize)]
struct PiAiConfig {
    #[serde(default)]
    providers: BTreeMap<String, ProviderCfg>,
}

#[derive(Debug, Deserialize)]
struct ProviderCfg {
    #[serde(default)]
    models: Vec<ModelCfg>,
    /// yaml 键名是 apiKeyEnv（camelCase，无全局 rename）——靠 alias 兼容。
    #[serde(alias = "apiKeyEnv", default)]
    api_key_env: Option<String>,
    /// yaml 键名是 baseURL（大写 URL）——serde 靠 alias 兼容。
    #[serde(alias = "baseURL", default)]
    base_url: Option<String>,
    /// pi-ai 协议名：openai-completions / anthropic-messages / …
    #[serde(default)]
    api: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelCfg {
    id: String,
}

/// credentials.yaml 的最小解析：refs[ENV] = secret。
#[derive(Debug, Deserialize)]
struct CredentialsFile {
    #[serde(default)]
    refs: BTreeMap<String, String>,
}

/// flash 级偏好排序：含这些子串的模型 id 排前（便宜、快，加工足够）。
fn flash_preference(model_id: &str) -> usize {
    let lower = model_id.to_lowercase();
    if lower.contains("flash") {
        0
    } else if lower.contains("mini") || lower.contains("lite") || lower.contains("turbo") {
        1
    } else {
        2
    }
}

/// 从 cordis.patch.yml + credentials.yaml 发现可用通道。
/// 返回 Err(LlmUnavailable) 由出刊层转为进度态。
pub fn discover_channel(dsh_home: &std::path::Path) -> Result<LlmChannel, LlmUnavailable> {
    let patch_path = dsh_home
        .join("profiles")
        .join("web")
        .join("cordis.patch.yml");
    let patch_text = std::fs::read_to_string(&patch_path).map_err(|cause| {
        LlmUnavailable::ConfigRead(format!("{}：{cause}", patch_path.display()))
    })?;
    let nodes: Vec<PatchNode> = serde_yaml::from_str(&patch_text).map_err(|cause| {
        LlmUnavailable::ConfigRead(format!("cordis.patch.yml 解析失败：{cause}"))
    })?;

    // 定位 llm-pi-ai 条目（两种节点形态都查；id 或 name 匹配）。
    let pi_cfg = nodes
        .iter()
        .flat_map(|node| match node {
            PatchNode::Insert { insert } => insert.iter().collect::<Vec<_>>(),
            PatchNode::Entry(entry) => vec![entry],
        })
        .find(|entry| {
            entry.id.as_deref() == Some("llm-pi-ai")
                || entry.name.as_deref() == Some("@deepseek-ai/dsh-llm-pi-ai")
        })
        .and_then(|entry| entry.config.as_ref());
    let Some(config_value) = pi_cfg else {
        return Err(LlmUnavailable::NoProvider);
    };
    let config: PiAiConfig = serde_yaml::from_value(config_value.clone())
        .map_err(|cause| LlmUnavailable::ConfigRead(format!("llm-pi-ai 配置解析失败：{cause}")))?;

    // 凭据（可选文件：本地 provider 可无 key）。
    let credentials = std::fs::read_to_string(dsh_home.join(".credentials.yaml"))
        .ok()
        .and_then(|text| serde_yaml::from_str::<CredentialsFile>(&text).ok());

    // 收集全部 openai-completions 候选，两轮选择：
    // 第一轮只要「有 key 的云端」（加工质量优先），全空再落本地端点。
    struct Candidate {
        provider: String,
        model: String,
        writing_model: String,
        base_url: String,
        api_key: String,
        cloud: bool,
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    for (provider, cfg) in &config.providers {
        let protocol = cfg
            .api
            .clone()
            .unwrap_or_else(|| "openai-completions".to_owned());
        if protocol != "openai-completions" {
            continue; // anthropic-messages 等 M2b 接入。
        }
        // 模型分层：flash 最优者做分类任务；非 flash 最优者做写作（无则退同 flash）。
        let mut sorted: Vec<String> = cfg.models.iter().map(|m| m.id.clone()).collect();
        sorted.sort_by_key(|id| (flash_preference(id), id.clone()));
        let Some(model) = sorted.first().cloned() else {
            continue;
        };
        let writing_model = sorted
            .iter()
            .find(|id| flash_preference(id) > 0)
            .cloned()
            .unwrap_or_else(|| model.clone());
        let base_url = match (&cfg.base_url, provider_endpoint(provider)) {
            (Some(explicit), _) => explicit.clone(),
            (None, Some(builtin)) => builtin.to_owned(),
            (None, None) => continue,
        };
        let api_key = cfg
            .api_key_env
            .as_ref()
            .and_then(|env| credentials.as_ref().and_then(|creds| creds.refs.get(env)))
            .cloned()
            .unwrap_or_default();
        let cloud = base_url.starts_with("https://");
        if cloud && api_key.is_empty() {
            continue; // 云端没有 key：本轮直接出局。
        }
        candidates.push(Candidate {
            provider: provider.clone(),
            model,
            writing_model,
            base_url,
            api_key,
            cloud,
        });
    }
    candidates
        .iter()
        .find(|c| c.cloud)
        .or_else(|| candidates.first())
        .map(|c| LlmChannel {
            provider: c.provider.clone(),
            model: c.model.clone(),
            writing_model: c.writing_model.clone(),
            base_url: c.base_url.clone(),
            api_key: c.api_key.clone(),
            // GLM 系（zai）为思考模型：调用时显式禁 thinking。
            thinking_disabled: c.provider.starts_with("zai"),
        })
        .ok_or_else(|| {
            // 有 provider 但全部不可用：给出更精确的原因。
            if let Some((provider, cfg)) = config.providers.iter().next() {
                let protocol = cfg
                    .api
                    .clone()
                    .unwrap_or_else(|| "openai-completions".to_owned());
                if protocol != "openai-completions" {
                    return LlmUnavailable::UnsupportedProtocol {
                        provider: provider.clone(),
                    };
                }
                if cfg.api_key_env.is_some() {
                    return LlmUnavailable::NoApiKey {
                        provider: provider.clone(),
                    };
                }
            }
            LlmUnavailable::NoProvider
        })
}

/// OpenAI 兼容 chat/completions 非流式调用（内置重试 1 次：
/// 429/5xx/超时/空 content/length 截断——这些瞬态失败不应丢篇）。
pub async fn complete(
    client: reqwest::Client,
    channel: &LlmChannel,
    args: CompleteArgs,
) -> Result<String, Error> {
    let mut last_err: Option<Error> = None;
    for attempt in 0..2 {
        if attempt > 0 {
            // 退避 12s：429 限流的恢复窗口（实测 5s 不够，三主题并发会连环 429）。
            crate::logging::log("info", "LLM 调用失败，12s 后重试");
            tokio::time::sleep(std::time::Duration::from_secs(12)).await;
        }
        match complete_once(&client, channel, &args).await {
            Ok(content) => return Ok(content),
            Err(cause) => last_err = Some(cause),
        }
    }
    Err(last_err.unwrap_or_else(|| Error::Intel("LLM 调用失败".to_owned())))
}

/// 单次调用。返回 content；usage 计入日志（费用可观测）。
async fn complete_once(
    client: &reqwest::Client,
    channel: &LlmChannel,
    args: &CompleteArgs,
) -> Result<String, Error> {
    let url = format!(
        "{}/chat/completions",
        channel.base_url.trim_end_matches('/')
    );
    let mut payload = serde_json::json!({
        "model": args.model,
        "messages": [
            {"role": "system", "content": args.system},
            {"role": "user", "content": args.user}
        ],
        "max_tokens": args.max_tokens,
        "temperature": 0.4
    });
    if channel.thinking_disabled {
        // GLM 系：思考 token 计入 max_tokens，显式关闭（docs/17 §7.5.1-1）。
        payload["thinking"] = serde_json::json!({"type": "disabled"});
    }
    let resp = client
        .post(&url)
        .bearer_auth(&channel.api_key)
        .json(&payload)
        .send()
        .await
        .and_then(|resp| resp.error_for_status())
        .map_err(|cause| Error::Intel(format!("LLM 请求失败：{cause}")))?;
    let text = resp
        .text()
        .await
        .map_err(|cause| Error::Intel(format!("LLM 响应读取失败：{cause}")))?;
    let parsed = parse_completion(&text);
    if let Some(usage) = parsed
        .value
        .as_ref()
        .and_then(|v| v.get("usage"))
        .and_then(|u| u.get("total_tokens"))
        .and_then(|t| t.as_u64())
    {
        crate::logging::log(
            "info",
            &format!("LLM 用量：{} total_tokens（{}）", usage, args.model),
        );
    }
    match parsed.content {
        Some(content) => Ok(content),
        // length 截断/空 content：可重试的瞬态形态。
        None => Err(Error::Intel(format!(
            "LLM 响应不可用（finish_reason={}）：{}",
            parsed.finish_reason.unwrap_or_else(|| "unknown".to_owned()),
            truncate(&text, 160)
        ))),
    }
}

/// chat/completions 响应解析结果。
struct ParsedCompletion {
    content: Option<String>,
    finish_reason: Option<String>,
    value: Option<serde_json::Value>,
}

/// 解析响应：取首个 choice 的 content 与 finish_reason（宽松，便于测试）。
fn parse_completion(text: &str) -> ParsedCompletion {
    let value: Option<serde_json::Value> = serde_json::from_str(text).ok();
    let choice = value
        .as_ref()
        .and_then(|v| v.get("choices"))
        .and_then(|c| c.get(0));
    let content = choice
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_owned);
    let finish_reason = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(|f| f.as_str())
        .map(str::to_owned);
    ParsedCompletion {
        content,
        finish_reason,
        value,
    }
}

/// 从模型输出提取 JSON（三级降级，docs/17 §7.5.1-6）：
/// ① 整段 parse；② ```json 围栏；③ 首个 `{`/`[` 起始的括号平衡扫描
/// （字符串字面量内的括号不计数——修「说明文字含示例 JSON」误伤）。
pub fn extract_json(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    // ① 整段就是 JSON。
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(value);
    }
    // ② 围栏优先。
    for fence in ["```json", "```"] {
        if let Some(rest) = trimmed.strip_prefix(fence) {
            if let Some(end) = rest.find("```") {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(rest[..end].trim()) {
                    return Some(value);
                }
            }
        }
    }
    // ③ 平衡扫描：从首个起始符开始，栈计数，字符串感知，取首个完整值。
    let bytes = trimmed.as_bytes();
    let start = trimmed.find(['{', '['])?;
    let mut depth: usize = 0;
    let mut in_string = false;
    let mut escaped = false;
    let open = bytes[start];
    let close = if open == b'{' { b'}' } else { b']' };
    for (offset, &b) in bytes[start..].iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    // 闭合符必须与起始符配对。
                    if b != close {
                        return None;
                    }
                    return serde_json::from_str(&trimmed[start..=start + offset]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

fn truncate(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        text
    } else {
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_patch(dir: &std::path::Path, patch: &str) -> std::path::PathBuf {
        let web = dir.join("profiles").join("web");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("cordis.patch.yml"), patch).unwrap();
        dir.to_path_buf()
    }

    // 顶层为列表（实测 2026-10）：混排 insert 节点与直接条目；
    // llm-pi-ai 是直接条目（不在 insert 内）。
    const PATCH_CLOUD: &str = r#"
- insert:
    - id: qx-bridge
      name: qx-bridge
      config:
        vault: "C:/tmp/vault"
- id: web
  config:
    searchProvider: qx-websearch
- id: llm-pi-ai
  name: "@deepseek-ai/dsh-llm-pi-ai"
  config:
    providers:
      zai-coding-cn:
        reasoning: max
        models:
          - id: glm-5.3
          - id: glm-5.3-flash
        apiKeyEnv: ZAI_CODING_CN_API_KEY
      qwen35-local:
        api: openai-completions
        baseURL: http://127.0.0.1:17230/v1
        models:
          - id: Qwen3.5-9B-Q6_K
        apiKeyEnv: QWEN35_LOCAL_API_KEY
"#;

    #[test]
    fn 云端provider缺key时跳过并取本地() {
        let dir = write_patch(
            &std::env::temp_dir().join(format!("qx-llm-{}", std::process::id())),
            PATCH_CLOUD,
        );
        // 无 credentials：zai 跳过（缺 key），qwen35-local 本地 endpoint 允许空 key。
        let channel = discover_channel(&dir).expect("本地 provider 应可用");
        assert_eq!(channel.provider, "qwen35-local");
        assert_eq!(channel.base_url, "http://127.0.0.1:17230/v1");
        assert_eq!(channel.model, "Qwen3.5-9B-Q6_K");
    }

    #[test]
    fn 有key时选云端并偏好flash() {
        let dir = write_patch(
            &std::env::temp_dir().join(format!("qx-llm-k-{}", std::process::id())),
            PATCH_CLOUD,
        );
        std::fs::write(
            dir.join(".credentials.yaml"),
            "refs:\n  ZAI_CODING_CN_API_KEY: sk-test\n",
        )
        .unwrap();
        let channel = discover_channel(&dir).expect("应选中 zai");
        assert_eq!(channel.provider, "zai-coding-cn");
        // 模型分层：分类用 flash，写作用主模型（GLM 系禁 thinking）。
        assert_eq!(channel.model, "glm-5.3-flash");
        assert_eq!(channel.writing_model, "glm-5.3");
        assert!(channel.thinking_disabled);
        assert_eq!(channel.api_key, "sk-test");
    }

    #[test]
    fn 无llm条目报不可用() {
        let dir = write_patch(
            &std::env::temp_dir().join(format!("qx-llm-n-{}", std::process::id())),
            "- id: other\n  config: {}\n",
        );
        match discover_channel(&dir) {
            Err(LlmUnavailable::NoProvider) => {}
            other => panic!("应为 NoProvider，实际 {other:?}"),
        }
    }

    #[test]
    fn json提取容忍包裹() {
        let value = extract_json("好的，如下：\n```json\n{\"a\":1}\n```");
        assert_eq!(value.unwrap()["a"], 1);
        let array = extract_json("[{\"id\":\"x\"}]");
        assert_eq!(array.unwrap()[0]["id"], "x");
    }

    #[test]
    fn json提取平衡扫描字符串感知() {
        // 字符串字面量内的 } 不破坏平衡；闲话后首个完整 JSON 值可提取
        // （旧首尾扫描会跨越两个 JSON 段导致解析失败）。
        let text = r#"说明 {"示例":"不是结果"} 真正结果 [{"id":"a"}]"#;
        let value = extract_json(text).expect("应提取到首个平衡段");
        assert_eq!(value["示例"], "不是结果");
        let text2 = r#"结果：{"note":"含 } 符号"}"#;
        assert_eq!(extract_json(text2).unwrap()["note"], "含 } 符号");
    }

    #[test]
    fn 响应解析含finish原因与空content() {
        let ok = parse_completion(
            r#"{"choices":[{"message":{"content":" 正文 "},"finish_reason":"stop"}],"usage":{"total_tokens":123}}"#,
        );
        assert_eq!(ok.content.as_deref(), Some("正文"));
        assert_eq!(ok.finish_reason.as_deref(), Some("stop"));
        assert_eq!(
            ok.value.unwrap()["usage"]["total_tokens"],
            serde_json::json!(123)
        );
        // 空内容（思考耗尽预算的典型形态）：content=None 但 finish_reason 可见。
        let empty = parse_completion(
            r#"{"choices":[{"message":{"content":""},"finish_reason":"length"}]}"#,
        );
        assert!(empty.content.is_none());
        assert_eq!(empty.finish_reason.as_deref(), Some("length"));
    }

    #[test]
    fn completion解析取content() {
        let parsed = parse_completion(r#"{"choices":[{"message":{"content":"  你好 "}}]}"#);
        assert_eq!(parsed.content.as_deref(), Some("你好"));
        assert!(parse_completion(r#"{"choices":[]}"#).content.is_none());
    }
}
