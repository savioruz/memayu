#[cfg(feature = "local-embedding")]
pub mod local_embedder;

mod embedder;
mod llm;
mod models;

pub use embedder::HttpEmbedderProvider;
pub use llm::HttpLlmProvider;
pub use models::{check_models, ModelsCheck};

use memayu_config::{EmbedderBackend, ProviderConfig};
use memayu_core::EmbedderProvider;
use std::sync::Arc;

/// Whether the configured embedder backend runs on-device (`Local`).
///
/// This is available regardless of the `local-embedding` feature so callers
/// (e.g. `memayu doctor`) can skip HTTP connectivity probes for local backends
/// even in a build that doesn't include Candle.
pub fn is_local_backend(cfg: &ProviderConfig) -> bool {
    matches!(cfg.backend, EmbedderBackend::Local)
}

/// Build an [`EmbedderProvider`] from a provider config, dispatching between
/// the HTTP (bring-your-own-key) and on-device local backends.
#[cfg(feature = "local-embedding")]
pub fn build_embedder(cfg: &ProviderConfig) -> Arc<dyn EmbedderProvider> {
    match cfg.backend {
        EmbedderBackend::Remote => Arc::new(HttpEmbedderProvider::new(cfg.clone())),
        EmbedderBackend::Local => {
            let model_id = if cfg.model.is_empty() {
                local_embedder::DEFAULT_MODEL_ID.to_string()
            } else {
                cfg.model.clone()
            };
            Arc::new(local_embedder::LocalEmbedder::new(model_id))
        }
    }
}

/// Fallback factory for builds without the `local-embedding` feature: the HTTP
/// provider is used for every backend.
#[cfg(not(feature = "local-embedding"))]
pub fn build_embedder(cfg: &ProviderConfig) -> Arc<dyn EmbedderProvider> {
    Arc::new(HttpEmbedderProvider::new(cfg.clone()))
}

// Re-export for callers that want the concrete local type.
#[cfg(feature = "local-embedding")]
pub use local_embedder::LocalEmbedder;
