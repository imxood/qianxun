use crate::agent::message::{ContentBlock, Message};
use std::collections::HashSet;

/// Collect all tool_use IDs that have a matching tool_result in the conversation.
pub fn satisfied_tool_ids(messages: &[Message]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for msg in messages {
        for block in msg.content() {
            if block.r#type == "tool_result" {
                if let Some(ref id) = block.tool_use_id {
                    ids.insert(id.clone());
                }
            }
        }
    }
    ids
}

/// For every tool_use without a matching tool_result, insert a synthetic user
/// message containing `ContentBlock::tool_result(id, "aborted", false)` immediately
/// after the assistant message that declared the tool_use.
///
/// Iterates in reverse order so insertions don't shift untraversed indices.
pub fn ensure_call_outputs_present(messages: &mut Vec<Message>) {
    let mut satisfied = satisfied_tool_ids(messages);

    // Reverse index iteration to keep indices stable
    let mut i = messages.len();
    while i > 0 {
        i -= 1;
        if let Message::Assistant { content, .. } = &messages[i] {
            let orphans: Vec<String> = content
                .iter()
                .filter_map(|b| {
                    if b.r#type == "tool_use" {
                        b.tool_use_id.as_ref().filter(|id| !satisfied.contains(id.as_str()))
                    } else {
                        None
                    }
                })
                .cloned()
                .collect();

            if !orphans.is_empty() {
                let result_blocks: Vec<ContentBlock> = orphans
                    .iter()
                    .map(|id| ContentBlock::tool_result(id.clone(), "aborted", false))
                    .collect();
                let synthetic = Message::user(result_blocks);
                messages.insert(i + 1, synthetic);

                // Mark as satisfied so earlier passes don't re-insert
                for id in &orphans {
                    satisfied.insert(id.clone());
                }
            }
        }
    }
}

/// Remove tool_result blocks whose tool_use_id does not appear in any tool_use block.
/// Also remove empty user messages that result from removing all their blocks.
pub fn remove_orphan_outputs(messages: &mut Vec<Message>) {
    // Build set of all tool_use IDs
    let tool_use_ids: HashSet<String> = messages
        .iter()
        .flat_map(|msg| msg.content().iter())
        .filter_map(|b| {
            if b.r#type == "tool_use" {
                b.tool_use_id.clone()
            } else {
                None
            }
        })
        .collect();

    // Remove orphaned tool_result blocks
    for msg in messages.iter_mut() {
        if let Message::User { content, .. } = msg {
            content.retain(|block| {
                if block.r#type == "tool_result" {
                    // Keep only if tool_use_id exists in the set
                    block.tool_use_id.as_ref().is_some_and(|id| tool_use_ids.contains(id.as_str()))
                } else {
                    true // Keep non-tool_result blocks
                }
            });
        }
    }

    // Remove empty user messages (but keep assistant messages even if empty)
    messages.retain(|msg| match msg {
        Message::User { content, .. } => !content.is_empty(),
        Message::Assistant { .. } => true,
    });
}

/// Full normalize: remove_orphan_outputs() then ensure_call_outputs_present().
/// Always called before build_request, regardless of compression.
pub fn normalize_messages(messages: &mut Vec<Message>) {
    remove_orphan_outputs(messages);
    ensure_call_outputs_present(messages);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::message::Message;

    #[test]
    fn test_satisfied_tool_ids() {
        let msgs = vec![
            Message::assistant(vec![ContentBlock::tool_use("a".into(), "tool_a", serde_json::json!({}))]),
            Message::user(vec![ContentBlock::tool_result("a".into(), "ok", false)]),
            Message::assistant(vec![ContentBlock::tool_use("b".into(), "tool_b", serde_json::json!({}))]),
        ];
        let satisfied = satisfied_tool_ids(&msgs);
        assert!(satisfied.contains("a"));
        assert!(!satisfied.contains("b"));
    }

    #[test]
    fn test_ensure_call_outputs_present_inserts_aborted() {
        let mut msgs = vec![
            Message::assistant(vec![ContentBlock::tool_use("orphan_1".into(), "tool_x", serde_json::json!({}))]),
            Message::assistant(vec![ContentBlock::text("final")]),
        ];
        ensure_call_outputs_present(&mut msgs);
        assert_eq!(msgs.len(), 3, "should insert one synthetic user message");
        if let Message::User { content, .. } = &msgs[1] {
            assert_eq!(content.len(), 1);
            assert_eq!(content[0].r#type, "tool_result");
            assert_eq!(content[0].tool_use_id.as_deref(), Some("orphan_1"));
            assert_eq!(content[0].text.as_deref(), Some("aborted"));
        } else {
            panic!("expected User message at index 1");
        }
    }

    #[test]
    fn test_ensure_call_outputs_present_skips_satisfied() {
        let mut msgs = vec![
            Message::assistant(vec![ContentBlock::tool_use("a".into(), "tool_a", serde_json::json!({}))]),
            Message::user(vec![ContentBlock::tool_result("a".into(), "ok", false)]),
        ];
        ensure_call_outputs_present(&mut msgs);
        assert_eq!(msgs.len(), 2, "no insertion needed");
    }

    #[test]
    fn test_remove_orphan_outputs_removes_unmatched() {
        let mut msgs = vec![
            Message::user(vec![ContentBlock::tool_result("orphan".into(), "spurious", false)]),
            Message::assistant(vec![ContentBlock::text("hello")]),
        ];
        remove_orphan_outputs(&mut msgs);
        assert_eq!(msgs.len(), 1, "orphan-only user msg should be removed");
    }

    #[test]
    fn test_remove_orphan_outputs_mixed_blocks() {
        let mut msgs = vec![
            Message::user(vec![
                ContentBlock::text("some text"),
                ContentBlock::tool_result("orphan".into(), "spurious", false),
            ]),
            Message::assistant(vec![ContentBlock::text("response")]),
        ];
        remove_orphan_outputs(&mut msgs);
        assert_eq!(msgs.len(), 2, "user msg should be kept due to text block");
        if let Message::User { content, .. } = &msgs[0] {
            assert_eq!(content.len(), 1);
            assert_eq!(content[0].r#type, "text");
        }
    }

    #[test]
    fn test_normalize_roundtrip() {
        // Valid conversation: normalize is a no-op on structure
        let mut msgs = vec![
            Message::assistant(vec![ContentBlock::tool_use("a".into(), "tool_a", serde_json::json!({}))]),
            Message::user(vec![ContentBlock::tool_result("a".into(), "ok", false)]),
        ];
        let original_count = msgs.len();
        normalize_messages(&mut msgs);
        assert_eq!(msgs.len(), original_count);
    }

    #[test]
    fn test_multiple_orphans_in_one_assistant() {
        let mut msgs = vec![
            Message::assistant(vec![
                ContentBlock::tool_use("o1".into(), "tool_a", serde_json::json!({})),
                ContentBlock::tool_use("o2".into(), "tool_b", serde_json::json!({})),
            ]),
        ];
        ensure_call_outputs_present(&mut msgs);
        assert_eq!(msgs.len(), 2);
        if let Message::User { content, .. } = &msgs[1] {
            assert_eq!(content.len(), 2);
            assert_eq!(content[0].tool_use_id.as_deref(), Some("o1"));
            assert_eq!(content[1].tool_use_id.as_deref(), Some("o2"));
        }
    }
}
