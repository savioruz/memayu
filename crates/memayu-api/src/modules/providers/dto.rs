use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ProviderConfigRequest {
    pub llm: Option<memayu_config::ProviderConfig>,
    pub embedder: Option<memayu_config::ProviderConfig>,
}

#[derive(Debug, Serialize)]
pub struct ProviderConfigResponse {
    pub llm: memayu_config::ProviderConfig,
    pub embedder: memayu_config::ProviderConfig,
}
