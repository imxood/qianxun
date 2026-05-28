use serde_json::Value;

pub type UserMessageId = String;
pub type AssistantMessageId = String;
pub type ToolCallId = String;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContentBlock {
    pub r#type: String,
    pub text: Option<String>,
    pub tool_use_id: Option<ToolCallId>,
    pub tool_name: Option<String>,
    pub input: Option<Value>,
    pub is_error: Option<bool>,
    pub thinking: Option<String>,
    pub signature: Option<String>,
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            r#type: "text".into(),
            text: Some(text.into()),
            tool_use_id: None,
            tool_name: None,
            input: None,
            is_error: None,
            thinking: None,
            signature: None,
        }
    }

    pub fn tool_result(id: ToolCallId, content: impl Into<String>, is_error: bool) -> Self {
        Self {
            r#type: "tool_result".into(),
            text: Some(content.into()),
            tool_use_id: Some(id),
            tool_name: None,
            input: None,
            is_error: Some(is_error),
            thinking: None,
            signature: None,
        }
    }

    pub fn tool_use(id: ToolCallId, name: impl Into<String>, input: Value) -> Self {
        Self {
            r#type: "tool_use".into(),
            text: None,
            tool_use_id: Some(id),
            tool_name: Some(name.into()),
            input: Some(input),
            is_error: None,
            thinking: None,
            signature: None,
        }
    }

    pub fn thinking(text: impl Into<String>, signature: Option<String>) -> Self {
        Self {
            r#type: "thinking".into(),
            text: Some(text.into()),
            tool_use_id: None,
            tool_name: None,
            input: None,
            is_error: None,
            thinking: None,
            signature,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Message {
    User {
        id: UserMessageId,
        content: Vec<ContentBlock>,
    },
    Assistant {
        id: AssistantMessageId,
        content: Vec<ContentBlock>,
    },
}

impl Message {
    pub fn user(content: Vec<ContentBlock>) -> Self {
        Self::User {
            id: uuid::Uuid::new_v4().to_string(),
            content,
        }
    }

    pub fn assistant(content: Vec<ContentBlock>) -> Self {
        Self::Assistant {
            id: uuid::Uuid::new_v4().to_string(),
            content,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Message::User { id, .. } | Message::Assistant { id, .. } => id,
        }
    }

    pub fn content(&self) -> &[ContentBlock] {
        match self {
            Message::User { content, .. } | Message::Assistant { content, .. } => content,
        }
    }

    pub fn role(&self) -> &str {
        match self {
            Message::User { .. } => "user",
            Message::Assistant { .. } => "assistant",
        }
    }

    pub fn content_mut(&mut self) -> &mut Vec<ContentBlock> {
        match self {
            Message::User { content, .. } | Message::Assistant { content, .. } => content,
        }
    }

    /// Returns true if this is a user message containing only tool_result blocks.
    pub fn is_tool_result_only(&self) -> bool {
        match self {
            Message::User { content, .. } => {
                !content.is_empty() && content.iter().all(|b| b.r#type == "tool_result")
            }
            Message::Assistant { .. } => false,
        }
    }
}
