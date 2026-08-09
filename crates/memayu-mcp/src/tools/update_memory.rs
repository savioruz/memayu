//! `update_memory` tool — update an existing memory's content.

use crate::types::ToolDefinition;
use crate::{McpError, MemoryBackend};
use serde_json::Value;
use std::collections::HashMap;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "update_memory",
        description: "Update an existing memory's content by its ID.",
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "memory_id": {
                    "type": "string",
                    "description": "The ID of the memory to update"
                },
                "content": {
                    "type": "string",
                    "description": "The new content for the memory"
                }
            },
            "required": ["memory_id", "content"]
        }),
    }
}

pub async fn call(
    args: &HashMap<String, Value>,
    backend: &dyn MemoryBackend,
) -> Result<Value, McpError> {
    let memory_id = args
        .get("memory_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::Api("Missing 'memory_id' argument".into()))?;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::Api("Missing 'content' argument".into()))?;

    if content.trim().is_empty() {
        return Err(McpError::Api("content must not be empty".into()));
    }

    let mem = backend.update_memory(memory_id, content).await?;
    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": format!("Memory updated: [{}] {}", mem.id, mem.content)
        }]
    }))
}
