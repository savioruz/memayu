//! `delete_memory` tool — remove a memory by ID.

use crate::types::ToolDefinition;
use crate::{McpError, MemoryBackend};
use serde_json::Value;
use std::collections::HashMap;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "delete_memory",
        description: "Delete a memory by its ID.",
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "memory_id": {
                    "type": "string",
                    "description": "The ID of the memory to delete"
                }
            },
            "required": ["memory_id"]
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

    backend.delete_memory(memory_id).await?;
    Ok(serde_json::json!({
        "content": [{"type": "text", "text": format!("Memory {} deleted.", memory_id)}]
    }))
}
