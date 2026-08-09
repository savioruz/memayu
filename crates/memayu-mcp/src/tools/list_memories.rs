//! `list_memories` tool — list recent memories.

use crate::types::ToolDefinition;
use crate::{McpError, MemoryBackend};
use serde_json::Value;
use std::collections::HashMap;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "list_memories",
        description: "List recent memories.",
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results (default 20)",
                    "default": 20
                }
            },
            "required": []
        }),
    }
}

pub async fn call(
    args: &HashMap<String, Value>,
    backend: &dyn MemoryBackend,
) -> Result<Value, McpError> {
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

    let user_id = args
        .get("user_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let memories = backend.list_memories(user_id, limit).await?;
    let text = if memories.is_empty() {
        "No memories stored yet.".into()
    } else {
        memories
            .iter()
            .map(|mem| format!("[{}] {}", mem.id, mem.content))
            .collect::<Vec<_>>()
            .join("\n")
    };

    Ok(serde_json::json!({
        "content": [{"type": "text", "text": text}]
    }))
}
