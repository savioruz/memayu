use async_trait::async_trait;
use memayu_config::ProviderConfig;
use memayu_core::{EmbedError, EmbedderProvider, ExtractionResult, LlmError, LlmProvider, Message};
use memayu_llm_client::{HttpEmbedderProvider, HttpLlmProvider};
use std::sync::{Arc, RwLock};

/// Shared, mutable provider configuration. Written on dashboard save, read per call.
#[derive(Clone)]
pub struct ConfigRegistry {
    llm: Arc<RwLock<ProviderConfig>>,
    embedder: Arc<RwLock<ProviderConfig>>,
}

impl ConfigRegistry {
    pub fn new(llm: ProviderConfig, embedder: ProviderConfig) -> Self {
        Self {
            llm: Arc::new(RwLock::new(llm)),
            embedder: Arc::new(RwLock::new(embedder)),
        }
    }

    pub fn llm(&self) -> ProviderConfig {
        self.llm.read().unwrap().clone()
    }

    pub fn embedder(&self) -> ProviderConfig {
        self.embedder.read().unwrap().clone()
    }

    pub fn set_llm(&self, config: ProviderConfig) {
        *self.llm.write().unwrap() = config;
    }

    pub fn set_embedder(&self, config: ProviderConfig) {
        *self.embedder.write().unwrap() = config;
    }
}

/// Seed the registry from env (as config defaults), then override with any
/// rows saved in the DB.
pub async fn load_registry(
    db: &crate::infrastructure::db::DbClient,
    fallback_llm: ProviderConfig,
    fallback_embedder: ProviderConfig,
) -> Result<ConfigRegistry, String> {
    let mut llm = fallback_llm;
    let mut embedder = fallback_embedder;
    for (provider, (base_url, api_key, model)) in db.provider_configs().await? {
        let cfg = ProviderConfig {
            base_url,
            api_key: Some(api_key),
            model,
        };
        match provider.as_str() {
            "llm" => llm = cfg,
            "embedder" => embedder = cfg,
            _ => {}
        }
    }
    Ok(ConfigRegistry::new(llm, embedder))
}

/// A provider whose config is read lazily from the registry.
pub struct LlmConfigProvider {
    registry: ConfigRegistry,
}

impl LlmConfigProvider {
    pub fn new(registry: ConfigRegistry) -> Self {
        Self { registry }
    }

    fn fresh(&self) -> HttpLlmProvider {
        HttpLlmProvider::new(self.registry.llm())
    }
}

#[async_trait]
impl LlmProvider for LlmConfigProvider {
    async fn extract(&self, messages: &[Message]) -> Result<ExtractionResult, LlmError> {
        self.fresh().extract(messages).await
    }
}

pub struct EmbedderConfigProvider {
    registry: ConfigRegistry,
}

impl EmbedderConfigProvider {
    pub fn new(registry: ConfigRegistry) -> Self {
        Self { registry }
    }

    fn fresh(&self) -> HttpEmbedderProvider {
        HttpEmbedderProvider::new(self.registry.embedder())
    }
}

#[async_trait]
impl EmbedderProvider for EmbedderConfigProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        self.fresh().embed(text).await
    }
}
