use crate::pagination::{MemoryPage, MetadataFilter};
use crate::Memory;
use async_trait::async_trait;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StorageError {
    #[error("storage operation failed: {0}")]
    Other(String),
    #[error("dimension mismatch: provider produces {got}-dim, stored data uses {expected}-dim")]
    DimensionMismatch { expected: usize, got: usize },
    #[error("limit {limit} exceeds the maximum of {max}")]
    LimitExceeded { limit: usize, max: usize },
    #[error("invalid pagination cursor: {0}")]
    InvalidCursor(String),
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LlmError {
    #[error("LLM request failed: {0}")]
    Other(String),
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EmbedError {
    #[error("embedding failed: {0}")]
    Other(String),
}

#[async_trait]
pub trait StorageProvider: Send + Sync {
    async fn save_memory(&self, mem: &Memory) -> Result<(), StorageError>;
    async fn get_memory(&self, memory_id: &str) -> Result<Memory, StorageError>;
    async fn search_memory(
        &self,
        user_id: &str,
        vector: &[f32],
        limit: usize,
        filter: Option<&MetadataFilter>,
    ) -> Result<Vec<(Memory, f32)>, StorageError>;
    /// Full-text search over memory content, ranked natively (higher = better).
    ///
    /// This is the second signal for hybrid search. Backends are free to pick
    /// their engine (FTS5, tsvector) as long as scores sort descending.
    async fn search_fulltext(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
        filter: Option<&MetadataFilter>,
    ) -> Result<Vec<(Memory, f32)>, StorageError>;

    async fn list_memories(
        &self,
        user_id: &str,
        limit: usize,
        cursor: Option<&str>,
        filter: Option<&MetadataFilter>,
    ) -> Result<MemoryPage, StorageError>;
    async fn delete_memory(&self, memory_id: &str) -> Result<(), StorageError>;
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn extract(&self, messages: &[Message]) -> Result<ExtractionResult, LlmError>;
}

#[async_trait]
pub trait EmbedderProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError>;
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtractionResult {
    pub decision: ExtractionDecision,
    pub updated_memory_id: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionDecision {
    Add,
    Update,
}

/// Per-instance memory-ingestion strategy.
///
/// - [`ExtractionMode::Llm`] is the default: the LLM normalizes content and
///   decides ADD vs UPDATE (unchanged legacy behavior).
/// - [`ExtractionMode::Raw`] stores content verbatim and skips the LLM
///   entirely, using a high similarity threshold to skip exact duplicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionMode {
    #[default]
    Llm,
    Raw,
}

impl std::fmt::Display for ExtractionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Llm => write!(f, "llm"),
            Self::Raw => write!(f, "raw"),
        }
    }
}

impl std::str::FromStr for ExtractionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "llm" => Ok(Self::Llm),
            "raw" => Ok(Self::Raw),
            other => Err(format!(
                "unknown extraction mode \"{other}\"; expected \"llm\" or \"raw\""
            )),
        }
    }
}

impl ExtractionResult {
    pub fn add(content: impl Into<String>) -> Self {
        Self {
            decision: ExtractionDecision::Add,
            updated_memory_id: None,
            content: content.into(),
        }
    }

    pub fn update(memory_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            decision: ExtractionDecision::Update,
            updated_memory_id: Some(memory_id.into()),
            content: content.into(),
        }
    }
}

pub type Metadata = HashMap<String, String>;

#[cfg(test)]
mod tests {
    use super::*;

    // ── Message constructors ──

    #[test]
    fn message_system_sets_role() {
        let m = Message::system("hello");
        assert_eq!(m.role, "system");
        assert_eq!(m.content, "hello");
    }

    #[test]
    fn message_user_sets_role() {
        let m = Message::user("hi");
        assert_eq!(m.role, "user");
        assert_eq!(m.content, "hi");
    }

    // ── ExtractionResult constructors ──

    #[test]
    fn extraction_result_add() {
        let r = ExtractionResult::add("new fact");
        assert_eq!(r.decision, ExtractionDecision::Add);
        assert!(r.updated_memory_id.is_none());
        assert_eq!(r.content, "new fact");
    }

    #[test]
    fn extraction_result_update() {
        let r = ExtractionResult::update("mem-1", "updated fact");
        assert_eq!(r.decision, ExtractionDecision::Update);
        assert_eq!(r.updated_memory_id.as_deref(), Some("mem-1"));
        assert_eq!(r.content, "updated fact");
    }
}
