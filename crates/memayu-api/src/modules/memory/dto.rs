use chrono::{DateTime, Utc};
use memayu_core::{Metadata, MetadataFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddMemoryRequest {
    pub content: String,
    #[serde(default)]
    pub metadata: Metadata,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AddMemoryResponse {
    pub status: String,
    pub memory_id: String,
    pub dimension: usize,
    pub metadata: Metadata,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchMemoryRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub metadata_filter: Option<MetadataFilter>,
}

fn default_limit() -> usize {
    5
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchMemoryResponse {
    pub memories: Vec<SearchResult>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResult {
    pub memory_id: String,
    pub content: String,
    pub score: f32,
    pub metadata: Metadata,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListQuery {
    /// Maximum number of memories to return. Defaults to 50. Hard maximum is
    /// 100; larger values are rejected with HTTP 400.
    #[serde(default = "default_list_limit")]
    #[schema(default = 50, maximum = 100)]
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(flatten)]
    #[schema(value_type = std::collections::HashMap<String, String>)]
    pub metadata_filter: MetadataFilterQuery,
}

#[derive(Debug, Clone, Default)]
pub struct MetadataFilterQuery(pub MetadataFilter);

impl<'de> serde::Deserialize<'de> for MetadataFilterQuery {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const PREFIX: &str = "metadata_filter.";
        let pairs: std::collections::HashMap<String, String> =
            std::collections::HashMap::deserialize(deserializer)?;
        let mut filter = MetadataFilter::new();
        for (k, v) in pairs {
            if let Some(key) = k.strip_prefix(PREFIX) {
                filter.insert(key.to_string(), v);
            }
        }
        Ok(MetadataFilterQuery(filter))
    }
}

fn default_list_limit() -> usize {
    50
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ListMemoryResponse {
    pub memories: Vec<ListedMemory>,
    /// Opaque cursor for the next page, or `null` when this is the last page.
    pub next_cursor: Option<String>,
    /// Total number of memories matching the current filter, independent of
    /// the current page window.
    pub total_data: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ListedMemory {
    pub memory_id: String,
    pub content: String,
    pub metadata: Metadata,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct UpdateMemoryRequest {
    pub content: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[allow(dead_code)]
pub struct UpdateMemoryResponse {
    pub status: String,
    pub memory_id: String,
    pub content: String,
    pub metadata: Metadata,
}
