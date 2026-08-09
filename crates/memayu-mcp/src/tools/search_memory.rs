//! `search_memory` tool — semantic similarity search.

use crate::types::ToolDefinition;
use crate::{McpError, MemoryBackend};
use serde_json::Value;
use std::collections::HashMap;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "search_memory",
        description: "Search memories by semantic similarity.",
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results (default 10)",
                    "default": 10
                }
            },
            "required": ["query"]
        }),
    }
}

pub async fn call(
    args: &HashMap<String, Value>,
    backend: &dyn MemoryBackend,
) -> Result<Value, McpError> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::Api("Missing 'query' argument".into()))?;

    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

    let user_id = args
        .get("user_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let results = backend.search_memory(user_id, query, limit).await?;
    let text = if results.is_empty() {
        "No memories found.".into()
    } else {
        results
            .iter()
            .map(|(mem, score)| format!("[{}] {} (score: {:.2})", mem.id, mem.content, score))
            .collect::<Vec<_>>()
            .join("\n")
    };

    Ok(serde_json::json!({
        "content": [{"type": "text", "text": text}]
    }))
}
