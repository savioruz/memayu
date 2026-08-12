use chrono::{DateTime, Utc};
use memayu_core::Metadata;
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
}

fn default_limit() -> usize {
    5
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchMemoryResponse {
    pub results: Vec<SearchResult>,
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
    #[serde(default = "default_list_limit")]
    pub limit: usize,
}

fn default_list_limit() -> usize {
    100
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ListMemoryResponse {
    pub memories: Vec<ListedMemory>,
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
