use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ProviderConfigRequest {
    pub llm: Option<memayu_config::ProviderConfig>,
    pub embedder: Option<memayu_config::ProviderConfig>,
    /// Optional `extraction_mode` ("llm" | "raw") to persist to the DB.
    #[serde(default)]
    pub extraction_mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProviderConfigResponse {
    pub llm: memayu_config::ProviderConfig,
    pub embedder: memayu_config::ProviderConfig,
}
