use crate::agent::message::{ContentBlock, Message, UserMessageId};
use crate::provider::types::CompletionRequest;
use crate::types::AgentConfig;
use std::path::Path;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone)]
pub struct TokenBudget {
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

pub struct Conversation {
    system_prompt: Option<String>,
    messages: Vec<Message>,
    budget: TokenBudget,
}

impl Conversation {
    pub fn new(system_prompt: Option<String>) -> Self {
        Self {
            system_prompt,
            messages: Vec::new(),
            budget: TokenBudget {
                max_input_tokens: None,
                max_output_tokens: None,
            },
        }
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn messages_mut(&mut self) -> &mut Vec<Message> {
        &mut self.messages
    }

    pub fn push_user_message(&mut self, content: Vec<ContentBlock>) -> UserMessageId {
        let msg = Message::user(content);
        let id = msg.id().to_string();
        self.messages.push(msg);
        id
    }

    pub fn push_message(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    pub fn build_request(
        &self,
        tools: &[crate::tools::ToolDefinition],
        memory_context: &str,
        skills_catalog: &str,
        skill_injections: &str,
        agent_config: &AgentConfig,
    ) -> CompletionRequest {
        // 拼接: system_prompt → memory_context → skills_catalog → skill_injections
        let mut parts: Vec<&str> = Vec::new();
        if let Some(base) = &self.system_prompt {
            parts.push(base);
        }
        if !memory_context.is_empty() {
            parts.push(memory_context);
        }
        if !skills_catalog.is_empty() {
            parts.push(skills_catalog);
        }
        if !skill_injections.is_empty() {
            parts.push(skill_injections);
        }
        let system = if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        };

        CompletionRequest {
            system,
            messages: self.messages.clone(),
            tools: tools.to_vec(),
            tool_choice: crate::types::ToolChoice::Auto,
            max_tokens: agent_config.max_tokens,
            temperature: agent_config.temperature,
            thinking: agent_config.thinking.clone(),
            stop_sequences: vec![],
        }
    }

    pub async fn enforce_budget(&mut self, _tools: &[crate::tools::ToolDefinition]) {
        // Phase 1: 简单裁剪，保留最近消息
        if let Some(max_tokens) = self.budget.max_input_tokens {
            // 粗略估计：每字符约 0.25 token
            let mut total: u64 = 0;
            let mut keep_from = 0;
            for (i, msg) in self.messages.iter().enumerate().rev() {
                let text_len: u64 = serde_json::to_string(msg)
                    .map(|s| s.len() as u64)
                    .unwrap_or(0);
                total += text_len / 3;
                if total > max_tokens {
                    keep_from = i;
                    break;
                }
            }
            if keep_from > 0 && keep_from < self.messages.len() {
                self.messages.drain(0..keep_from);
            }
        }
    }

    pub fn budget(&self) -> &TokenBudget {
        &self.budget
    }

    pub fn set_budget(&mut self, max_input: Option<u64>, max_output: Option<u64>) {
        self.budget = TokenBudget {
            max_input_tokens: max_input,
            max_output_tokens: max_output,
        };
    }

    /// JSONL 格式保存到文件。
    /// 第一行: {"type":"system","prompt":"..."}
    /// 后续行: 每个 Message 一行 JSON
    pub async fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let mut file = tokio::fs::File::create(path).await?;
        let header = serde_json::json!({"type": "system", "prompt": self.system_prompt});
        let line = serde_json::to_string(&header).expect("header serialization");
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        for msg in &self.messages {
            let line = serde_json::to_string(msg).expect("message serialization");
            file.write_all(line.as_bytes()).await?;
            file.write_all(b"\n").await?;
        }
        Ok(())
    }

    /// 从 JSONL 文件加载会话。
    pub async fn load_from(path: &Path) -> std::io::Result<Self> {
        let content = tokio::fs::read_to_string(path).await?;
        let mut lines = content.lines();
        let mut system_prompt = None;
        let mut messages = Vec::new();

        if let Some(first) = lines.next() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(first) {
                if val.get("type").and_then(|v| v.as_str()) == Some("system") {
                    system_prompt = val.get("prompt").and_then(|v| v.as_str()).map(String::from);
                }
            }
        }

        for line in lines {
            if let Ok(msg) = serde_json::from_str::<Message>(line) {
                messages.push(msg);
            }
        }

        let mut conv = Self::new(system_prompt);
        conv.messages = messages;
        Ok(conv)
    }
}
