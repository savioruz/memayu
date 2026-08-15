use async_trait::async_trait;
use chrono::Utc;
use memayu_core::{Memory, MemoryService, Metadata, MetadataFilter};
use std::collections::HashMap;
use std::sync::Arc;

/// Unified error type for MCP tool operations.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("api error: {0}")]
    Api(String),
}

impl From<memayu_core::CoreError> for McpError {
    fn from(e: memayu_core::CoreError) -> Self {
        McpError::Api(e.to_string())
    }
}

/// API responses are wrapped in a `{ "result": <body> }` envelope.
#[derive(serde::Deserialize)]
struct Envelope<T> {
    result: T,
}

/// Abstract backend — implemented by in-process `MemoryService` or a remote HTTP client.
#[async_trait]
pub trait MemoryBackend: Send + Sync {
    async fn add_memory(&self, user_id: &str, content: &str) -> Result<Memory, McpError>;
    async fn search_memory(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
        metadata_filter: Option<MetadataFilter>,
    ) -> Result<Vec<(Memory, f32)>, McpError>;
    async fn list_memories(&self, user_id: &str, limit: usize) -> Result<Vec<Memory>, McpError>;
    async fn delete_memory(&self, memory_id: &str) -> Result<(), McpError>;
    async fn update_memory(&self, memory_id: &str, content: &str) -> Result<Memory, McpError>;
}

// ── Concrete Backend ──

pub enum Backend {
    /// In-process frontend talking to [`MemoryService`] directly. The tool
    /// payloads still carry a `user_id` (defaulting to `"default"` for
    /// compatibility), but in self-hosted mode the instance has a single admin
    /// account, so the local backend ignores that value and always resolves to
    /// `account_id` (#32).
    Local {
        service: Arc<MemoryService>,
        account_id: String,
    },
    Cloud {
        base_url: String,
        api_key: Option<String>,
        client: reqwest::Client,
    },
}

#[async_trait]
impl MemoryBackend for Backend {
    async fn add_memory(&self, user_id: &str, content: &str) -> Result<Memory, McpError> {
        match self {
            Backend::Local {
                service,
                account_id,
            } => Ok(service
                .add_memory(account_id, content, &Metadata::default())
                .await?),
            Backend::Cloud {
                base_url,
                api_key,
                client,
            } => {
                let url = format!("{}/api/memories/add", base_url.trim_end_matches('/'));
                let body = serde_json::json!({"content": content, "metadata": {}});
                let mut req = client.post(&url).json(&body);
                if let Some(key) = api_key {
                    req = req.header("x-api-key", key);
                }
                let resp = req.send().await.map_err(|e| McpError::Api(e.to_string()))?;
                if !resp.status().is_success() {
                    let s = resp.status();
                    let t = resp.text().await.unwrap_or_default();
                    return Err(McpError::Api(format!("HTTP {s}: {t}")));
                }
                #[derive(serde::Deserialize)]
                struct R {
                    memory_id: String,
                }
                let data: Envelope<R> = resp
                    .json()
                    .await
                    .map_err(|e| McpError::Api(e.to_string()))?;
                Ok(Memory {
                    id: data.result.memory_id,
                    user_id: user_id.into(),
                    content: content.into(),
                    vector: vec![],
                    metadata: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                })
            }
        }
    }

    async fn search_memory(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
        metadata_filter: Option<MetadataFilter>,
    ) -> Result<Vec<(Memory, f32)>, McpError> {
        match self {
            Backend::Local {
                service,
                account_id,
            } => Ok(service
                .search_memory_filtered(account_id, query, limit, metadata_filter.as_ref())
                .await?),
            Backend::Cloud {
                base_url,
                api_key,
                client,
            } => {
                let url = format!("{}/api/memories/search", base_url.trim_end_matches('/'));
                let mut body = serde_json::json!({"query": query, "limit": limit});
                if let Some(f) = metadata_filter {
                    body["metadata_filter"] = serde_json::to_value(f)
                        .map_err(|e| McpError::Api(format!("serialize filter: {e}")))?;
                }
                let mut req = client.post(&url).json(&body);
                if let Some(key) = api_key {
                    req = req.header("x-api-key", key);
                }
                let resp = req.send().await.map_err(|e| McpError::Api(e.to_string()))?;
                if !resp.status().is_success() {
                    let s = resp.status();
                    let t = resp.text().await.unwrap_or_default();
                    return Err(McpError::Api(format!("HTTP {s}: {t}")));
                }
                #[derive(serde::Deserialize)]
                struct R {
                    memories: Vec<Ri>,
                }
                #[derive(serde::Deserialize)]
                struct Ri {
                    memory_id: String,
                    content: String,
                    score: f32,
                    #[serde(default)]
                    created_at: Option<chrono::DateTime<Utc>>,
                }
                let data: Envelope<R> = resp
                    .json()
                    .await
                    .map_err(|e| McpError::Api(e.to_string()))?;
                Ok(data
                    .result
                    .memories
                    .into_iter()
                    .map(|r| {
                        (
                            Memory {
                                id: r.memory_id,
                                user_id: user_id.into(),
                                content: r.content,
                                vector: vec![],
                                metadata: HashMap::new(),
                                created_at: r.created_at.unwrap_or_else(Utc::now),
                                updated_at: Utc::now(),
                            },
                            r.score,
                        )
                    })
                    .collect())
            }
        }
    }

    async fn list_memories(&self, user_id: &str, limit: usize) -> Result<Vec<Memory>, McpError> {
        match self {
            Backend::Local {
                service,
                account_id,
            } => Ok(service.list_memories(account_id, limit).await?),
            Backend::Cloud {
                base_url,
                api_key,
                client,
            } => {
                let url = format!(
                    "{}/api/memories/list?limit={limit}",
                    base_url.trim_end_matches('/')
                );
                let mut req = client.get(&url);
                if let Some(key) = api_key {
                    req = req.header("x-api-key", key);
                }
                let resp = req.send().await.map_err(|e| McpError::Api(e.to_string()))?;
                if !resp.status().is_success() {
                    let s = resp.status();
                    let t = resp.text().await.unwrap_or_default();
                    return Err(McpError::Api(format!("HTTP {s}: {t}")));
                }
                #[derive(serde::Deserialize)]
                struct R {
                    memories: Vec<Ri>,
                }
                #[derive(serde::Deserialize)]
                struct Ri {
                    memory_id: String,
                    content: String,
                    created_at: chrono::DateTime<Utc>,
                    updated_at: chrono::DateTime<Utc>,
                }
                let data: Envelope<R> = resp
                    .json()
                    .await
                    .map_err(|e| McpError::Api(e.to_string()))?;
                Ok(data
                    .result
                    .memories
                    .into_iter()
                    .map(|m| Memory {
                        id: m.memory_id,
                        user_id: user_id.into(),
                        content: m.content,
                        vector: vec![],
                        metadata: HashMap::new(),
                        created_at: m.created_at,
                        updated_at: m.updated_at,
                    })
                    .collect())
            }
        }
    }

    async fn delete_memory(&self, memory_id: &str) -> Result<(), McpError> {
        match self {
            Backend::Local { service, .. } => {
                service.delete_memory(memory_id).await?;
                Ok(())
            }
            Backend::Cloud {
                base_url,
                api_key,
                client,
            } => {
                let url = format!(
                    "{}/api/memories/{}",
                    base_url.trim_end_matches('/'),
                    memory_id
                );
                let mut req = client.delete(&url);
                if let Some(key) = api_key {
                    req = req.header("x-api-key", key);
                }
                let resp = req.send().await.map_err(|e| McpError::Api(e.to_string()))?;
                if !resp.status().is_success() {
                    let s = resp.status();
                    let t = resp.text().await.unwrap_or_default();
                    return Err(McpError::Api(format!("HTTP {s}: {t}")));
                }
                Ok(())
            }
        }
    }

    async fn update_memory(&self, memory_id: &str, content: &str) -> Result<Memory, McpError> {
        match self {
            Backend::Local { service, .. } => Ok(service.update_memory(memory_id, content).await?),
            Backend::Cloud {
                base_url,
                api_key,
                client,
            } => {
                let url = format!(
                    "{}/api/memories/{}",
                    base_url.trim_end_matches('/'),
                    memory_id
                );
                let body = serde_json::json!({"content": content});
                let mut req = client.patch(&url).json(&body);
                if let Some(key) = api_key {
                    req = req.header("x-api-key", key);
                }
                let resp = req.send().await.map_err(|e| McpError::Api(e.to_string()))?;
                if !resp.status().is_success() {
                    let s = resp.status();
                    let t = resp.text().await.unwrap_or_default();
                    return Err(McpError::Api(format!("HTTP {s}: {t}")));
                }
                #[derive(serde::Deserialize)]
                struct R {
                    memory_id: String,
                    content: String,
                }
                let data: Envelope<R> = resp
                    .json()
                    .await
                    .map_err(|e| McpError::Api(e.to_string()))?;
                Ok(Memory {
                    id: data.result.memory_id,
                    user_id: String::new(),
                    content: data.result.content,
                    vector: vec![],
                    metadata: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                })
            }
        }
    }
}
