//! `add_memory` tool — store a new memory (single or batch).

use crate::types::ToolDefinition;
use crate::{McpError, MemoryBackend};
use serde_json::Value;
use std::collections::HashMap;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "add_memory",
        description:
            "Store new memories or update existing ones. The system will deduplicate and merge similar memories. Pass a single `content` string, or a `memories` array to store several in one call.",
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The memory content to store (single add)"
                },
                "memories": {
                    "type": "array",
                    "description": "Batch of memories to store in one call",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "The memory content to store"
                            },
                            "metadata": {
                                "type": "object",
                                "description": "Optional key/value metadata"
                            }
                        },
                        "required": ["content"]
                    }
                }
            }
        }),
    }
}

pub async fn call(
    args: &HashMap<String, Value>,
    backend: &dyn MemoryBackend,
) -> Result<Value, McpError> {
    let user_id = args
        .get("user_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    // Batch path: `memories` is an array of {content, metadata}. A failure on
    // one item does not abort the rest; successes and failures are summarized.
    if let Some(items) = args.get("memories").and_then(|v| v.as_array()) {
        let mut added = 0usize;
        let mut ids = Vec::new();
        let mut errors = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let content = item
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if content.is_empty() {
                errors.push(format!("item {i}: content is required"));
                continue;
            }
            match backend.add_memory(user_id, content).await {
                Ok(mem) => {
                    added += 1;
                    ids.push(mem.id);
                }
                Err(e) => errors.push(format!("item {i}: {e}")),
            }
        }
        let summary = if errors.is_empty() {
            format!("Stored {added} memories (ids: {})", ids.join(", "))
        } else {
            format!(
                "Stored {added} memories (ids: {}); failures: {}",
                ids.join(", "),
                errors.join("; ")
            )
        };
        return Ok(serde_json::json!({
            "content": [{ "type": "text", "text": summary }]
        }));
    }

    // Single path: `content` string (backward compatible).
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::Api("Missing 'content' or 'memories' argument".into()))?;

    let mem = backend.add_memory(user_id, content).await?;
    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": format!("Memory stored: {} (id: {})", mem.content, mem.id)
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use memayu_core::{Memory, MetadataFilter};

    /// Backend that stores each memory verbatim with a deterministic id.
    struct MockBackend;

    #[async_trait]
    impl MemoryBackend for MockBackend {
        async fn add_memory(&self, user_id: &str, content: &str) -> Result<Memory, McpError> {
            Ok(Memory {
                id: format!("id-{content}"),
                user_id: user_id.to_string(),
                content: content.to_string(),
                vector: vec![],
                metadata: HashMap::new(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        }
        async fn search_memory(
            &self,
            _user_id: &str,
            _query: &str,
            _limit: usize,
            _metadata_filter: Option<MetadataFilter>,
        ) -> Result<Vec<(Memory, f32)>, McpError> {
            Ok(vec![])
        }
        async fn list_memories(
            &self,
            _user_id: &str,
            _limit: usize,
        ) -> Result<Vec<Memory>, McpError> {
            Ok(vec![])
        }
        async fn delete_memory(&self, _memory_id: &str) -> Result<(), McpError> {
            Ok(())
        }
        async fn update_memory(
            &self,
            _memory_id: &str,
            _content: &str,
        ) -> Result<Memory, McpError> {
            unreachable!()
        }
    }

    fn args(map: serde_json::Map<String, Value>) -> HashMap<String, Value> {
        HashMap::from_iter(map)
    }

    #[tokio::test]
    async fn single_content_is_backward_compatible() {
        let args = args(
            serde_json::json!({ "content": "hello" })
                .as_object()
                .unwrap()
                .clone(),
        );
        let out = call(&args, &MockBackend).await.unwrap();
        let text = out["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("id-hello"), "text: {text}");
    }

    #[tokio::test]
    async fn batch_stores_all_items() {
        let args = args(
            serde_json::json!({ "memories": [
                {"content":"a"},
                {"content":"b"},
                {"content":"c"}
            ] })
            .as_object()
            .unwrap()
            .clone(),
        );
        let out = call(&args, &MockBackend).await.unwrap();
        let text = out["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Stored 3 memories"), "text: {text}");
        assert!(text.contains("id-a"), "text: {text}");
        assert!(text.contains("id-b"), "text: {text}");
        assert!(text.contains("id-c"), "text: {text}");
    }

    #[tokio::test]
    async fn batch_reports_per_item_failures() {
        let args = args(
            serde_json::json!({ "memories": [
                {"content":"ok"},
                {"content":""},
                {"content":"ok2"}
            ] })
            .as_object()
            .unwrap()
            .clone(),
        );
        let out = call(&args, &MockBackend).await.unwrap();
        let text = out["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Stored 2 memories"), "text: {text}");
        assert!(text.contains("item 1: content is required"), "text: {text}");
    }
}
