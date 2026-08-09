//! `add_memory` tool — store a new memory.

use crate::types::ToolDefinition;
use crate::{McpError, MemoryBackend};
use serde_json::Value;
use std::collections::HashMap;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "add_memory",
        description:
            "Store a new memory or update existing ones. The system will deduplicate and merge similar memories.",
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The memory content to store"
                }
            },
            "required": ["content"]
        }),
    }
}

pub async fn call(
    args: &HashMap<String, Value>,
    backend: &dyn MemoryBackend,
) -> Result<Value, McpError> {
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::Api("Missing 'content' argument".into()))?;

    let user_id = args
        .get("user_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let mem = backend.add_memory(user_id, content).await?;
    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": format!("Memory stored: {} (id: {})", mem.content, mem.id)
        }]
    }))
}
