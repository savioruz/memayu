use async_trait::async_trait;
use memayu_config::{EmbedderBackend, ProviderConfig};
use memayu_core::{EmbedError, EmbedderProvider, ExtractionResult, LlmError, LlmProvider, Message};
use memayu_llm_client::HttpLlmProvider;
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
/// rows saved in the DB. The stored `backend` is DB-authoritative for the
/// embedder row (e.g. `local` Candle vs `remote` HTTP); the LLM is always
/// `remote`.
pub async fn load_registry(
    db: &crate::infrastructure::db::DbClient,
    fallback_llm: ProviderConfig,
    fallback_embedder: ProviderConfig,
) -> Result<ConfigRegistry, String> {
    let mut llm = fallback_llm;
    let mut embedder = fallback_embedder;
    for (provider, (backend, base_url, api_key, model)) in db.provider_configs().await? {
        let cfg = ProviderConfig {
            backend: if provider == "embedder" {
                backend.parse().unwrap_or(EmbedderBackend::Remote)
            } else {
                EmbedderBackend::Remote
            },
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

/// Read the DB-authoritative `extraction_mode`, falling back to `fallback`
/// when no row has been stored yet (pre-seeding installs).
pub async fn load_extraction_mode(
    db: &crate::infrastructure::db::DbClient,
    fallback: memayu_core::ExtractionMode,
) -> Result<memayu_core::ExtractionMode, String> {
    Ok(db
        .get_extraction_mode()
        .await?
        .and_then(|s| s.parse().ok())
        .unwrap_or(fallback))
}

/// Single shared write path for Category B settings (LLM + embedder provider
/// rows and the `extraction_mode` runtime setting). Called by the CLI wizard,
/// the TUI wizard, the web `/setup` page, and fresh-boot seeding.
pub async fn persist_provider_config(
    db: &crate::infrastructure::db::DbClient,
    llm: &ProviderConfig,
    embedder: &ProviderConfig,
    extraction_mode: memayu_core::ExtractionMode,
) -> Result<(), String> {
    // Normalize before persisting so a local embedder never carries a stale
    // base_url/api_key into the row (e.g. when setup prefills a value from a
    // prior remote config). The DB upsert normalizes again as a backstop, so
    // this is defense-in-depth at the shared write path.
    let llm = llm.clone().normalize();
    let embedder = embedder.clone().normalize();
    // Ensure tables exist. Idempotent for the already-initialized server DB;
    // required when a CLI/TUI wizard opens a fresh connection.
    db.init().await?;
    // Raw mode never calls an LLM, so it must not persist an LLM provider row
    // at all (not even a placeholder). The runtime reads `runtime_settings`
    // for the extraction mode and never infers it from the LLM row, so a raw
    // instance intentionally has no `llm` row.
    if extraction_mode != memayu_core::ExtractionMode::Raw {
        db.upsert_provider_config(
            "llm",
            "remote",
            &llm.base_url,
            llm.api_key.as_deref().unwrap_or(""),
            &llm.model,
        )
        .await?;
    }
    db.upsert_provider_config(
        "embedder",
        &embedder.backend.to_string(),
        &embedder.base_url,
        embedder.api_key.as_deref().unwrap_or(""),
        &embedder.model,
    )
    .await?;
    db.set_extraction_mode(&extraction_mode.to_string()).await?;
    Ok(())
}

/// Enforce config/DB precedence at boot.
///
/// - Empty `provider_config` table (fresh install): seed it from Category B
///   config, but only after that config validates. If it is invalid and no
///   admin account exists yet, this is an unconfigured first boot and we fail
///   fast with an actionable error naming `memayu setup` / `memayu setup --tui`.
/// - Non-empty table: the DB is authoritative; nothing is overwritten.
/// - Empty table + invalid config + existing admin (pre-`backend` schema
///   install): keep the DB authoritative-empty; the registry falls back to
///   defaults and `load_extraction_mode` falls back to the config default.
pub async fn ensure_provider_config(
    db: &crate::infrastructure::db::DbClient,
    config: &memayu_config::Config,
) -> Result<(), String> {
    let rows = db.provider_configs().await?;
    if !rows.is_empty() {
        return Ok(());
    }

    let issues = config.check();
    if !issues.is_empty() {
        let has_admin = !db.users_empty().await?;
        if !has_admin {
            return Err(format!(
                "No provider configuration in the database and the on-disk \
                 config is incomplete:\n  - {}\n\nRun `memayu setup` (interactive \
                 wizard) or `memayu setup --tui` to configure storage, embedding, \
                 and extraction. The choices are written to the database and the \
                 server will then boot normally.",
                issues.join("\n  - ")
            ));
        }
        // Admin exists (legacy DB): DB stays authoritative-empty; defaults apply.
        return Ok(());
    }

    persist_provider_config(db, &config.llm, &config.embedder, config.extraction_mode).await
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

    fn fresh(&self) -> Arc<dyn EmbedderProvider> {
        memayu_llm_client::build_embedder(&self.registry.embedder())
    }
}

#[async_trait]
impl EmbedderProvider for EmbedderConfigProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        self.fresh().embed(text).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::DbClient;
    use memayu_config::{
        Config, EmbedderBackend, ProviderConfig, ServerConfig, StorageBackend, StorageConfig,
    };

    async fn mem_db() -> DbClient {
        let storage = StorageConfig {
            backend: StorageBackend::Libsql,
            libsql_path: ":memory:".to_string(),
            database_url: None,
        };
        let db = DbClient::open(&storage).await.unwrap();
        db.init().await.unwrap();
        db
    }

    #[tokio::test]
    async fn persist_provider_config_local_embedder_clears_stale_fields() {
        let db = mem_db().await;
        let llm = ProviderConfig {
            backend: EmbedderBackend::Remote,
            base_url: "http://llm:11434".into(),
            api_key: Some("llm-key".into()),
            model: "llm-model".into(),
        };
        // A local backend arriving with a stale remote base_url/api_key (e.g.
        // prefilled by setup from a prior config) must not be persisted.
        let embedder = ProviderConfig {
            backend: EmbedderBackend::Local,
            base_url: "https://api.openai.com/v1".into(),
            api_key: Some("sk-stale".into()),
            model: "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2".into(),
        };
        persist_provider_config(&db, &llm, &embedder, memayu_core::ExtractionMode::Llm)
            .await
            .unwrap();

        let rows = db.provider_configs().await.unwrap();
        let (eb, eurl, ekey, emodel) = &rows["embedder"];
        assert_eq!(eb, "local");
        assert_eq!(
            eurl, "",
            "local embedder base_url must be cleared on persist"
        );
        assert_eq!(
            ekey, "",
            "local embedder api_key must be cleared on persist"
        );
        assert_eq!(
            emodel,
            "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"
        );

        // The LLM row keeps its values (always remote).
        let (_, lurl, lkey, lmodel) = &rows["llm"];
        assert_eq!(lurl, "http://llm:11434");
        assert_eq!(lkey, "llm-key");
        assert_eq!(lmodel, "llm-model");
    }

    /// A raw-mode `Config` (empty LLM, `Raw` extraction) used to simulate a
    /// headless first boot with `MEMAYU_EXTRACTION_MODE=raw`.
    fn raw_config() -> Config {
        Config {
            storage: StorageConfig {
                backend: StorageBackend::Libsql,
                libsql_path: ":memory:".into(),
                database_url: None,
            },
            llm: ProviderConfig {
                backend: EmbedderBackend::Remote,
                base_url: String::new(),
                api_key: None,
                model: String::new(),
            },
            embedder: ProviderConfig {
                backend: EmbedderBackend::Local,
                base_url: String::new(),
                api_key: None,
                model: "sentence-transformers/all-MiniLM-L6-v2".into(),
            },
            server: ServerConfig {
                bind_addr: "127.0.0.1".into(),
                port: 18080,
            },
            similarity_threshold: 0.65,
            extraction_mode: memayu_core::ExtractionMode::Raw,
            dimension: Some(384),
            api_url: None,
            api_key: None,
        }
    }

    fn llm_config() -> Config {
        Config {
            llm: ProviderConfig {
                backend: EmbedderBackend::Remote,
                base_url: "http://llm:11434".into(),
                api_key: Some("llm-key".into()),
                model: "gpt-4".into(),
            },
            extraction_mode: memayu_core::ExtractionMode::Llm,
            ..raw_config()
        }
    }

    #[tokio::test]
    async fn persist_provider_config_raw_writes_no_llm_row() {
        let db = mem_db().await;
        let llm = ProviderConfig {
            backend: EmbedderBackend::Remote,
            base_url: "https://api.openai.com/v1".into(),
            api_key: Some("sk-stale".into()),
            model: "gpt-4".into(),
        };
        let embedder = ProviderConfig {
            backend: EmbedderBackend::Local,
            base_url: String::new(),
            api_key: None,
            model: "sentence-transformers/all-MiniLM-L6-v2".into(),
        };
        persist_provider_config(&db, &llm, &embedder, memayu_core::ExtractionMode::Raw)
            .await
            .unwrap();

        let rows = db.provider_configs().await.unwrap();
        assert!(
            !rows.contains_key("llm"),
            "raw mode must not persist an llm provider row, got {rows:?}"
        );
        assert!(rows.contains_key("embedder"));
        assert_eq!(
            db.get_extraction_mode().await.unwrap().as_deref(),
            Some("raw")
        );
        // The registry must resolve raw (never llm) and expose no LLM config.
        let reg = load_registry(&db, raw_config().llm, raw_config().embedder)
            .await
            .unwrap();
        assert_eq!(reg.llm().base_url, "");
        assert_eq!(reg.llm().model, "");
    }

    #[tokio::test]
    async fn persist_provider_config_llm_writes_llm_row() {
        let db = mem_db().await;
        let llm = ProviderConfig {
            backend: EmbedderBackend::Remote,
            base_url: "https://llm.example.com/v1".into(),
            api_key: Some("sk-test".into()),
            model: "gpt-4".into(),
        };
        let embedder = ProviderConfig {
            backend: EmbedderBackend::Local,
            base_url: String::new(),
            api_key: None,
            model: "sentence-transformers/all-MiniLM-L6-v2".into(),
        };
        persist_provider_config(&db, &llm, &embedder, memayu_core::ExtractionMode::Llm)
            .await
            .unwrap();

        let rows = db.provider_configs().await.unwrap();
        assert!(rows.contains_key("llm"));
        let (_, lurl, lkey, lmodel) = &rows["llm"];
        assert_eq!(lurl, "https://llm.example.com/v1");
        assert_eq!(lkey, "sk-test");
        assert_eq!(lmodel, "gpt-4");
        assert_eq!(
            db.get_extraction_mode().await.unwrap().as_deref(),
            Some("llm")
        );
    }

    #[tokio::test]
    async fn ensure_provider_config_headless_raw_seeds_raw_no_llm() {
        let db = mem_db().await;
        ensure_provider_config(&db, &raw_config()).await.unwrap();
        let rows = db.provider_configs().await.unwrap();
        assert!(
            !rows.contains_key("llm"),
            "headless raw boot must not seed an llm row, got {rows:?}"
        );
        assert_eq!(
            db.get_extraction_mode().await.unwrap().as_deref(),
            Some("raw")
        );
        assert_eq!(
            load_extraction_mode(&db, memayu_core::ExtractionMode::Llm)
                .await
                .unwrap(),
            memayu_core::ExtractionMode::Raw,
            "raw mode must not be overridden by the llm fallback"
        );
    }

    #[tokio::test]
    async fn ensure_provider_config_headless_llm_seeds_llm() {
        let db = mem_db().await;
        ensure_provider_config(&db, &llm_config()).await.unwrap();
        let rows = db.provider_configs().await.unwrap();
        assert!(rows.contains_key("llm"));
        let (_, lurl, _, lmodel) = &rows["llm"];
        assert_eq!(lurl, "http://llm:11434");
        assert_eq!(lmodel, "gpt-4");
        assert_eq!(
            db.get_extraction_mode().await.unwrap().as_deref(),
            Some("llm")
        );
    }
}
