//! Tool registry — each tool module exports `definition()` and `call()`.

mod add_memory;
mod delete_memory;
mod list_memories;
mod search_memory;
mod update_memory;

use crate::types::ToolDefinition;
use crate::{McpError, MemoryBackend};
use std::collections::HashMap;

/// Collect every tool's definition for `tools/list`.
pub fn all_definitions() -> Vec<ToolDefinition> {
    vec![
        add_memory::definition(),
        search_memory::definition(),
        list_memories::definition(),
        delete_memory::definition(),
        update_memory::definition(),
    ]
}

/// Dispatch a `tools/call` request to the matching tool handler.
pub async fn dispatch(
    name: &str,
    args: &HashMap<String, serde_json::Value>,
    backend: &dyn MemoryBackend,
) -> Result<serde_json::Value, McpError> {
    match name {
        "add_memory" => add_memory::call(args, backend).await,
        "search_memory" => search_memory::call(args, backend).await,
        "list_memories" => list_memories::call(args, backend).await,
        "delete_memory" => delete_memory::call(args, backend).await,
        "update_memory" => update_memory::call(args, backend).await,
        _ => Err(McpError::Api(format!("Unknown tool: {name}"))),
    }
}
