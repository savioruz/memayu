use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct GenerateKeyResponse {
    pub key: String,
    pub id: String,
    pub label: String,
    pub key_prefix: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ListKeyResponse {
    pub id: String,
    pub label: String,
    pub key_prefix: String,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ListKeysResponse {
    pub keys: Vec<ListKeyResponse>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateKeyRequest {
    pub label: String,
}
