use crate::agent::message::{ContentBlock, Message, UserMessageId};
use crate::provider::types::CompletionRequest;
use crate::types::{AgentConfig, TokenUsage};

#[derive(Debug, Clone)]
pub struct TokenBudget {
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

#[allow(dead_code)]
pub struct Conversation {
    system_prompt: Option<String>,
    messages: Vec<Message>,
    budget: TokenBudget,
    cache_breakpoint: Option<usize>,
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
            cache_breakpoint: None,
        }
    }

    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
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
        agent_config: &AgentConfig,
    ) -> CompletionRequest {
        let system = match (&self.system_prompt, memory_context.is_empty(), skills_catalog.is_empty()) {
            (Some(base), false, false) => Some(format!("{base}\n\n{memory_context}\n\n{skills_catalog}")),
            (Some(base), false, true) => Some(format!("{base}\n\n{memory_context}")),
            (Some(base), true, false) => Some(format!("{base}\n\n{skills_catalog}")),
            (Some(base), true, true) => Some(base.clone()),
            (None, false, _) if !memory_context.is_empty() => Some(memory_context.to_string()),
            (None, _, false) if !skills_catalog.is_empty() => Some(skills_catalog.to_string()),
            _ => None,
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

    /// 消耗的 token 估算
    pub fn estimated_tokens(&self) -> TokenUsage {
        let text: String = self
            .messages
            .iter()
            .flat_map(serde_json::to_string)
            .collect();
        let count = (text.len() / 3) as u64;
        TokenUsage {
            input: count,
            output: 0,
            cache_creation_input: None,
            cache_read_input: None,
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
}
