//! `search_memory` tool — semantic similarity search.

use crate::types::ToolDefinition;
use crate::{McpError, MemoryBackend};
use memayu_core::MetadataFilter;
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
                },
                "metadata_filter": {
                    "type": "object",
                    "additionalProperties": {
                        "type": "string"
                    },
                    "description": "Exact key=value metadata predicates; only memories matching every pair are returned"
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

    // Optional exact-match metadata filter: a JSON object of key=value pairs.
    let metadata_filter: Option<MetadataFilter> = args
        .get("metadata_filter")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        });
    let metadata_filter = metadata_filter.filter(|m| !m.is_empty());

    let user_id = args
        .get("user_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let results = backend
        .search_memory(user_id, query, limit, metadata_filter)
        .await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use memayu_core::Memory;

    /// Records the filter passed to `search_memory` so the tool's parsing and
    /// forwarding behaviour can be asserted without a real backend.
    struct RecordingBackend {
        last_filter: std::sync::Mutex<Option<MetadataFilter>>,
    }

    #[async_trait]
    impl MemoryBackend for RecordingBackend {
        async fn add_memory(&self, _user_id: &str, _content: &str) -> Result<Memory, McpError> {
            unimplemented!()
        }
        async fn search_memory(
            &self,
            _user_id: &str,
            _query: &str,
            _limit: usize,
            metadata_filter: Option<MetadataFilter>,
        ) -> Result<Vec<(Memory, f32)>, McpError> {
            *self.last_filter.lock().unwrap() = metadata_filter;
            Ok(vec![])
        }
        async fn list_memories(
            &self,
            _user_id: &str,
            _limit: usize,
        ) -> Result<Vec<Memory>, McpError> {
            unimplemented!()
        }
        async fn delete_memory(&self, _memory_id: &str) -> Result<(), McpError> {
            unimplemented!()
        }
        async fn update_memory(
            &self,
            _memory_id: &str,
            _content: &str,
        ) -> Result<Memory, McpError> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn forwards_metadata_filter() {
        let backend = RecordingBackend {
            last_filter: std::sync::Mutex::new(None),
        };
        let args: HashMap<String, Value> = serde_json::from_value(serde_json::json!({
            "query": "test",
            "metadata_filter": {"source": "telegram", "room": "alpha"}
        }))
        .unwrap();

        call(&args, &backend).await.unwrap();

        let filter = backend.last_filter.lock().unwrap().clone().unwrap();
        assert_eq!(filter["source"], "telegram");
        assert_eq!(filter["room"], "alpha");
    }

    #[tokio::test]
    async fn omits_filter_when_absent() {
        let backend = RecordingBackend {
            last_filter: std::sync::Mutex::new(Some(Default::default())),
        };
        let args: HashMap<String, Value> =
            serde_json::from_value(serde_json::json!({"query": "test"})).unwrap();

        call(&args, &backend).await.unwrap();

        assert!(backend.last_filter.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn drops_empty_filter_object() {
        let backend = RecordingBackend {
            last_filter: std::sync::Mutex::new(Some(Default::default())),
        };
        let args: HashMap<String, Value> = serde_json::from_value(serde_json::json!({
            "query": "test",
            "metadata_filter": {}
        }))
        .unwrap();

        call(&args, &backend).await.unwrap();

        assert!(backend.last_filter.lock().unwrap().is_none());
    }
}
