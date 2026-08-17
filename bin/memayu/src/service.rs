//! Shared service construction for the TUI and local MCP frontends.
//!
//! This module deliberately avoids `memayu-api` so the default TUI build does
//! not pull in the Axum web stack.

use memayu_config::{Config, StorageBackend};
use memayu_core::{MemoryService, StorageProvider};
use memayu_llm_client::{build_embedder, HttpLlmProvider};
use std::sync::Arc;

/// Resolve the embedding dimension for storage *without* probing the embedder.
///
/// Order: an explicit dimension in the config (env `MEMAYU_EMBEDDER_DIM`, or the
/// `embedding_dim` written to the config file by `memayu setup`) wins; otherwise
/// the dimension already stored in the database schema is read back — a local
/// read, never an embedder call. Fails only when neither is available (a fresh,
/// unconfigured store), directing the user to `memayu setup`.
///
/// This is silent on purpose: the MCP frontend writes JSON-RPC to stdout, so no
/// logging may happen here.
pub async fn resolve_storage_dimension(
    config: &Config,
) -> Result<usize, Box<dyn std::error::Error>> {
    if let Some(dim) = config.dimension {
        return Ok(dim);
    }
    let stored = match config.storage.backend {
        StorageBackend::Libsql => {
            memayu_storage_libsql::LibsqlProvider::inspect(&config.storage.libsql_path)
                .await?
                .dimension
        }
        StorageBackend::Postgres => {
            memayu_storage_postgres::PostgresProvider::inspect(
                config
                    .storage
                    .database_url
                    .as_deref()
                    .ok_or("missing postgres url")?,
            )
            .await?
            .dimension
        }
    };
    stored.ok_or_else(|| {
        "no embedding dimension available for storage; run `memayu setup` or set MEMAYU_EMBEDDER_DIM"
            .into()
    })
}

/// Open a [`StorageProvider`] for the configured backend.
async fn build_storage(
    config: &Config,
    dimension: usize,
) -> Result<Arc<dyn StorageProvider>, Box<dyn std::error::Error>> {
    match config.storage.backend {
        StorageBackend::Libsql => Ok(Arc::new(
            memayu_storage_libsql::LibsqlProvider::open(&config.storage.libsql_path, dimension)
                .await?,
        )),
        StorageBackend::Postgres => Ok(Arc::new(
            memayu_storage_postgres::PostgresProvider::connect(
                config
                    .storage
                    .database_url
                    .as_deref()
                    .ok_or("missing postgres url")?,
                dimension,
            )
            .await?,
        )),
    }
}

/// Build a [`MemoryService`] backed by static providers from the config.
///
/// Returns the resolved embedding dimension alongside the service. Used by the
/// TUI and local MCP frontends, which have no runtime dashboard to reconfigure
/// providers.
pub async fn build_service(
    config: &Config,
) -> Result<(Arc<MemoryService>, usize), Box<dyn std::error::Error>> {
    let dimension = resolve_storage_dimension(config).await?;
    let storage = build_storage(config, dimension).await?;
    let service = Arc::new(
        MemoryService::new(
            storage,
            build_embedder(&config.embedder),
            Arc::new(HttpLlmProvider::new(config.llm.clone())),
        )
        .with_extraction_mode(config.extraction_mode),
    );
    Ok((service, dimension))
}
