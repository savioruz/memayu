mod error;

pub use error::ConfigError;

use std::collections::HashMap;
use std::path::PathBuf;

pub use memayu_core::ExtractionMode;

// ── Types ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageBackend {
    Libsql,
    Postgres,
}

impl std::fmt::Display for StorageBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageBackend::Libsql => write!(f, "libsql"),
            StorageBackend::Postgres => write!(f, "postgres"),
        }
    }
}

impl std::str::FromStr for StorageBackend {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "libsql" => Ok(StorageBackend::Libsql),
            "postgres" => Ok(StorageBackend::Postgres),
            other => Err(format!(
                "unknown storage backend \"{other}\"; expected \"libsql\" or \"postgres\""
            )),
        }
    }
}

/// Where the embedding model runs.
///
/// - [`EmbedderBackend::Remote`] is the default: an OpenAI-compatible remote
///   endpoint (the legacy BYOK behavior).
/// - [`EmbedderBackend::Local`] runs a Candle-based model entirely in-process
///   with no API key and no network call at inference time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbedderBackend {
    #[default]
    #[serde(alias = "http")]
    Remote,
    Local,
}

impl std::fmt::Display for EmbedderBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbedderBackend::Remote => write!(f, "remote"),
            EmbedderBackend::Local => write!(f, "local"),
        }
    }
}

impl std::str::FromStr for EmbedderBackend {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "remote" | "http" => Ok(EmbedderBackend::Remote),
            "local" => Ok(EmbedderBackend::Local),
            other => Err(format!(
                "unknown embedder backend \"{other}\"; expected \"remote\" or \"local\""
            )),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub backend: EmbedderBackend,
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_bind_addr() -> String {
    "127.0.0.1".into()
}

fn default_port() -> u16 {
    18080
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_storage_backend")]
    pub backend: StorageBackend,
    #[serde(default = "default_libsql_path")]
    pub libsql_path: String,
    #[serde(default)]
    pub database_url: Option<String>,
}

fn default_storage_backend() -> StorageBackend {
    StorageBackend::Libsql
}

fn default_libsql_path() -> String {
    "./memayu.db".into()
}

// ── TOML file representation ──

/// Mirrors the on-disk TOML layout with optional sections.
/// Every field is optional so partial configs work.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConfigFile {
    #[serde(default)]
    pub storage: Option<StorageConfigFile>,
    #[serde(default)]
    pub llm: Option<ProviderConfigFile>,
    #[serde(default)]
    pub embedder: Option<ProviderConfigFile>,
    #[serde(default)]
    pub server: Option<ServerConfigFile>,
    #[serde(default)]
    pub behavior: Option<BehaviorConfigFile>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StorageConfigFile {
    #[serde(default = "default_storage_backend_file")]
    pub backend: String,
    #[serde(default)]
    pub libsql_path: Option<String>,
    #[serde(default)]
    pub database_url: Option<String>,
}

fn default_storage_backend_file() -> String {
    "libsql".into()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderConfigFile {
    /// Optional embedder backend: "remote" (default) or "local". Ignored for the LLM.
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerConfigFile {
    #[serde(default)]
    pub bind_addr: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BehaviorConfigFile {
    #[serde(default)]
    pub similarity_threshold: Option<f32>,
    #[serde(default)]
    pub extraction_mode: Option<String>,
}

// ── Runtime Config ──

#[derive(Debug, Clone)]
pub struct Config {
    pub storage: StorageConfig,
    pub llm: ProviderConfig,
    pub embedder: ProviderConfig,
    pub server: ServerConfig,
    pub similarity_threshold: f32,
    pub extraction_mode: ExtractionMode,
    pub dimension: Option<usize>,
    /// Cloud-mode: API endpoint (MEMAYU_API_URL). When set, LLM/embedder/storage validation is skipped.
    pub api_url: Option<String>,
    pub api_key: Option<String>,
}

// ── Helpers ──

fn validate_url(var: &'static str, value: String) -> Result<String, ConfigError> {
    if value.starts_with("http://") || value.starts_with("https://") {
        Ok(value)
    } else {
        Err(ConfigError::Invalid {
            var,
            value,
            detail: "expected an http(s) URL".into(),
        })
    }
}

// ── XDG config path ──

/// Returns the canonical XDG config file path without creating directories.
/// Resolution order:
/// 1. `$MEMAYU_CONFIG` env var
/// 2. `$XDG_CONFIG_HOME/memayu/config.toml`
/// 3. `~/.config/memayu/config.toml`
pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("MEMAYU_CONFIG") {
        return PathBuf::from(p);
    }
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"));
    base.join("memayu").join("config.toml")
}

/// Returns the directory used to cache downloaded embedding models.
/// Resolution order:
/// 1. `$MEMAYU_MODEL_DIR`
/// 2. `$XDG_DATA_HOME/memayu/models`
/// 3. `~/.local/share/memayu/models`
///
/// The directory is created on first use by the local embedder.
pub fn model_dir() -> PathBuf {
    if let Ok(p) = std::env::var("MEMAYU_MODEL_DIR") {
        return PathBuf::from(p);
    }
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("~/.local/share"));
    base.join("memayu").join("models")
}

/// Try to read and parse the config file at the canonical path.
/// Returns `Ok(None)` if the file doesn't exist.
pub fn read_config_file(path: &std::path::Path) -> Result<Option<ConfigFile>, ConfigError> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::File {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    let cf: ConfigFile = toml::from_str(&raw).map_err(|e| ConfigError::Parse {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    Ok(Some(cf))
}

// ── Merge: file defaults → env overrides ──

impl Config {
    /// Load config: file first, then env overrides on top.
    /// If no config file exists, still tries env vars.
    pub fn load() -> Result<Self, ConfigError> {
        let env_map: HashMap<String, String> = std::env::vars().collect();
        let cf = read_config_file(&config_path())?;
        Self::merge(cf, &env_map)
    }

    /// Merge an optional ConfigFile with env overrides.
    pub fn merge(
        cf: Option<ConfigFile>,
        env: &HashMap<String, String>,
    ) -> Result<Self, ConfigError> {
        let api_url = env.get("MEMAYU_API_URL").cloned().filter(|s| !s.is_empty());

        // Cloud mode: skip provider validation
        if api_url.is_some() {
            return cloud_config(env);
        }

        let file = cf.unwrap_or_default();

        // ── behavior (resolved first: raw mode makes the LLM optional) ──
        let extraction_mode = extraction_mode(
            env,
            file.behavior
                .as_ref()
                .and_then(|b| b.extraction_mode.as_deref()),
        )?;

        // ── storage ──
        let backend: StorageBackend = env
            .get("MEMAYU_STORAGE_BACKEND")
            .map(|s| {
                s.parse().map_err(|e| ConfigError::Invalid {
                    var: "MEMAYU_STORAGE_BACKEND",
                    value: s.clone(),
                    detail: e,
                })
            })
            .transpose()?
            .unwrap_or_else(|| {
                file.storage
                    .as_ref()
                    .and_then(|s| s.backend.parse().ok())
                    .unwrap_or(StorageBackend::Libsql)
            });

        let postgres_url = match backend {
            StorageBackend::Libsql => None,
            StorageBackend::Postgres => Some(
                env.get("MEMAYU_DATABASE_URL")
                    .cloned()
                    .or_else(|| file.storage.as_ref().and_then(|s| s.database_url.clone()))
                    .unwrap_or_default(),
            ),
        };

        // ── LLM ──
        // Required in `llm` mode; optional in `raw` mode, which never calls it.
        let (llm_base_url, llm_api_key, llm_model) = if extraction_mode == ExtractionMode::Raw {
            (
                env_opt_or(
                    env,
                    file.llm.as_ref().and_then(|l| l.base_url.as_deref()),
                    "MEMAYU_LLM_BASE_URL",
                )
                .unwrap_or_default(),
                env_opt_or(
                    env,
                    file.llm.as_ref().and_then(|l| l.api_key.as_deref()),
                    "MEMAYU_LLM_API_KEY",
                ),
                env_opt_or(
                    env,
                    file.llm.as_ref().and_then(|l| l.model.as_deref()),
                    "MEMAYU_LLM_MODEL",
                )
                .unwrap_or_default(),
            )
        } else {
            (
                env_required_or(
                    env,
                    file.llm.as_ref().and_then(|l| l.base_url.as_deref()),
                    "MEMAYU_LLM_BASE_URL",
                    "llm.base_url",
                )?,
                env_opt_or(
                    env,
                    file.llm.as_ref().and_then(|l| l.api_key.as_deref()),
                    "MEMAYU_LLM_API_KEY",
                ),
                env_required_or(
                    env,
                    file.llm.as_ref().and_then(|l| l.model.as_deref()),
                    "MEMAYU_LLM_MODEL",
                    "llm.model",
                )?,
            )
        };

        // ── Embedder ──
        // Backend: "remote" (default, BYOK) or "local" (in-process Candle model).
        // A local backend needs no base_url or api_key.
        let emb_backend: EmbedderBackend = env
            .get("MEMAYU_EMBEDDER_BACKEND")
            .map(|s| {
                s.parse().map_err(|e| ConfigError::Invalid {
                    var: "MEMAYU_EMBEDDER_BACKEND",
                    value: s.clone(),
                    detail: e,
                })
            })
            .transpose()?
            .unwrap_or_else(|| {
                file.embedder
                    .as_ref()
                    .and_then(|l| l.backend.as_deref())
                    .and_then(|b| b.parse().ok())
                    .unwrap_or(EmbedderBackend::Remote)
            });

        let emb_base_url = if emb_backend == EmbedderBackend::Local {
            env_opt_or(
                env,
                file.embedder.as_ref().and_then(|l| l.base_url.as_deref()),
                "MEMAYU_EMBEDDER_BASE_URL",
            )
            .unwrap_or_default()
        } else {
            env_required_or(
                env,
                file.embedder.as_ref().and_then(|l| l.base_url.as_deref()),
                "MEMAYU_EMBEDDER_BASE_URL",
                "embedder.base_url",
            )?
        };
        let emb_api_key = env_opt_or(
            env,
            file.embedder.as_ref().and_then(|l| l.api_key.as_deref()),
            "MEMAYU_EMBEDDER_API_KEY",
        );
        let emb_model = env_required_or(
            env,
            file.embedder.as_ref().and_then(|l| l.model.as_deref()),
            "MEMAYU_EMBEDDER_MODEL",
            "embedder.model",
        )?;

        // ── Server ──
        let bind_addr = env
            .get("MEMAYU_BIND_ADDR")
            .cloned()
            .or_else(|| file.server.as_ref().and_then(|s| s.bind_addr.clone()))
            .unwrap_or_else(|| "127.0.0.1".into());
        let port: u16 = env
            .get("MEMAYU_PORT")
            .map(|p| {
                p.parse().map_err(|_| ConfigError::Invalid {
                    var: "MEMAYU_PORT",
                    value: p.clone(),
                    detail: "expected a u16 port number".into(),
                })
            })
            .transpose()?
            .or_else(|| file.server.as_ref().and_then(|s| s.port))
            .unwrap_or(18080);

        let sim = env
            .get("MEMAYU_SIMILARITY_THRESHOLD")
            .map(|v| {
                v.parse().map_err(|_| ConfigError::Invalid {
                    var: "MEMAYU_SIMILARITY_THRESHOLD",
                    value: v.clone(),
                    detail: "expected a float between 0.0 and 1.0".into(),
                })
            })
            .transpose()?
            .or_else(|| file.behavior.as_ref().and_then(|b| b.similarity_threshold))
            .unwrap_or(0.65);

        let dim = env
            .get("MEMAYU_EMBEDDER_DIM")
            .map(|v| {
                v.parse().map_err(|_| ConfigError::Invalid {
                    var: "MEMAYU_EMBEDDER_DIM",
                    value: v.clone(),
                    detail: "expected a positive integer".into(),
                })
            })
            .transpose()?;

        Ok(Self {
            storage: StorageConfig {
                backend,
                libsql_path: env
                    .get("MEMAYU_LIBSQL_PATH")
                    .cloned()
                    .or_else(|| file.storage.as_ref().and_then(|s| s.libsql_path.clone()))
                    .unwrap_or_else(|| "./memayu.db".into()),
                database_url: postgres_url,
            },
            llm: ProviderConfig {
                backend: EmbedderBackend::Remote,
                base_url: if llm_base_url.is_empty() {
                    llm_base_url
                } else {
                    validate_url("MEMAYU_LLM_BASE_URL", llm_base_url)?
                },
                api_key: llm_api_key,
                model: llm_model,
            },
            embedder: ProviderConfig {
                backend: emb_backend,
                base_url: if emb_backend == EmbedderBackend::Local {
                    emb_base_url
                } else {
                    validate_url("MEMAYU_EMBEDDER_BASE_URL", emb_base_url)?
                },
                api_key: emb_api_key,
                model: emb_model,
            },
            server: ServerConfig { bind_addr, port },
            similarity_threshold: sim,
            extraction_mode,
            dimension: dim,
            api_url: None,
            api_key: None,
        })
    }

    /// Validate that all required fields are present.
    /// Returns a list of human-readable errors — empty means valid.
    pub fn check(&self) -> Vec<String> {
        let mut msgs = Vec::new();
        if self.api_url.is_some() {
            return msgs; // cloud mode skips validation
        }
        if self.extraction_mode != ExtractionMode::Raw {
            if self.llm.base_url.is_empty() {
                msgs.push("llm.base_url: missing — set MEMAYU_LLM_BASE_URL".into());
            }
            if self.llm.model.is_empty() {
                msgs.push("llm.model: missing — set MEMAYU_LLM_MODEL".into());
            }
        }
        if self.embedder.backend == EmbedderBackend::Remote && self.embedder.base_url.is_empty() {
            msgs.push("embedder.base_url: missing — set MEMAYU_EMBEDDER_BASE_URL".into());
        }
        if self.embedder.model.is_empty() {
            msgs.push(
                "embedder.model: missing — set MEMAYU_EMBEDDER_MODEL (HF model id for local backend)"
                    .into(),
            );
        }
        if let StorageBackend::Postgres = self.storage.backend {
            if self
                .storage
                .database_url
                .as_ref()
                .is_none_or(|u| u.is_empty())
            {
                msgs.push("storage.database_url: missing — required for postgres backend".into());
            }
        }
        msgs
    }

    /// Pretty-print effective config with secrets redacted.
    pub fn show(&self) -> String {
        let redact = |s: &Option<String>| -> Option<String> {
            s.as_ref().map(|v| {
                if v.is_empty() {
                    "(empty)".to_string()
                } else {
                    "***".to_string()
                }
            })
        };
        format!(
            "[storage]\n\
             backend = \"{}\"\n\
             libsql_path = \"{}\"\n\
             database_url = \"{}\"\n\
             \n\
             [llm]\n\
             base_url = \"{}\"\n\
             api_key = \"{}\"\n\
             model = \"{}\"\n\
             \n\
             [embedder]\n\
             base_url = \"{}\"\n\
             api_key = \"{}\"\n\
             model = \"{}\"\n\
             \n\
             [server]\n\
             bind_addr = \"{}\"\n\
             port = {}\n\
             \n\
             [behavior]\n\
             similarity_threshold = {}\n\
             extraction_mode = \"{}\"\n\
             \n\
             {}{}\n",
            self.storage.backend,
            self.storage.libsql_path,
            self.storage.database_url.as_deref().unwrap_or(""),
            self.llm.base_url,
            redact(&self.llm.api_key).unwrap_or_else(|| "(none)".into()),
            self.llm.model,
            self.embedder.base_url,
            redact(&self.embedder.api_key).unwrap_or_else(|| "(none)".into()),
            self.embedder.model,
            self.server.bind_addr,
            self.server.port,
            self.similarity_threshold,
            self.extraction_mode,
            if let Some(d) = self.dimension {
                format!("embedding_dim = {d}\n")
            } else {
                String::new()
            },
            if let Some(url) = &self.api_url {
                format!("api_url = \"{url}\"\n")
            } else {
                String::new()
            },
        )
    }

    // ── for internal migration: still exposes from_env for tests ──

    #[cfg(test)]
    fn from_env(env: &HashMap<String, String>) -> Result<Self, ConfigError> {
        Self::merge(None, env)
    }
}

// ── Private helpers ──

fn cloud_config(env: &HashMap<String, String>) -> Result<Config, ConfigError> {
    let sim = env
        .get("MEMAYU_SIMILARITY_THRESHOLD")
        .map(|v| {
            v.parse().map_err(|_| ConfigError::Invalid {
                var: "MEMAYU_SIMILARITY_THRESHOLD",
                value: v.clone(),
                detail: "expected a float between 0.0 and 1.0".into(),
            })
        })
        .transpose()?
        .unwrap_or(0.65);
    Ok(Config {
        storage: StorageConfig {
            backend: StorageBackend::Libsql,
            libsql_path: String::new(),
            database_url: None,
        },
        llm: ProviderConfig {
            backend: EmbedderBackend::Remote,
            base_url: String::new(),
            api_key: None,
            model: String::new(),
        },
        embedder: ProviderConfig {
            backend: EmbedderBackend::Remote,
            base_url: String::new(),
            api_key: None,
            model: String::new(),
        },
        server: ServerConfig {
            bind_addr: "0.0.0.0".into(),
            port: 8080,
        },
        similarity_threshold: sim,
        extraction_mode: ExtractionMode::default(),
        dimension: None,
        api_url: env.get("MEMAYU_API_URL").cloned(),
        api_key: env.get("MEMAYU_API_KEY").cloned(),
    })
}

fn env_required_or(
    env: &HashMap<String, String>,
    file_val: Option<&str>,
    env_var: &'static str,
    field_label: &'static str,
) -> Result<String, ConfigError> {
    if let Some(v) = env.get(env_var).filter(|s| !s.is_empty()) {
        return Ok(v.clone());
    }
    if let Some(v) = file_val {
        if !v.is_empty() {
            return Ok(v.to_string());
        }
    }
    Err(ConfigError::MissingField {
        env_var,
        field: field_label,
    })
}

fn env_opt_or(
    env: &HashMap<String, String>,
    file_val: Option<&str>,
    env_var: &'static str,
) -> Option<String> {
    if let Some(v) = env.get(env_var).filter(|s| !s.is_empty()) {
        return Some(v.clone());
    }
    if let Some(v) = file_val {
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}

/// Resolve the extraction mode from env (`MEMAYU_EXTRACTION_MODE`) or the
/// config file (`behavior.extraction_mode`). Defaults to [`ExtractionMode::Llm`].
fn extraction_mode(
    env: &HashMap<String, String>,
    file_val: Option<&str>,
) -> Result<ExtractionMode, ConfigError> {
    if let Some(v) = env.get("MEMAYU_EXTRACTION_MODE").filter(|s| !s.is_empty()) {
        return v.parse().map_err(|e| ConfigError::Invalid {
            var: "MEMAYU_EXTRACTION_MODE",
            value: v.clone(),
            detail: e,
        });
    }
    if let Some(v) = file_val.filter(|s| !s.is_empty()) {
        return v.parse().map_err(|e| ConfigError::Invalid {
            var: "MEMAYU_EXTRACTION_MODE",
            value: v.to_string(),
            detail: e,
        });
    }
    Ok(ExtractionMode::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.to_string());
        }
        m
    }

    fn minimal_env() -> HashMap<String, String> {
        env_with(&[
            ("MEMAYU_LLM_BASE_URL", "https://api.deepseek.com/v1"),
            ("MEMAYU_LLM_API_KEY", "llm-key"),
            ("MEMAYU_LLM_MODEL", "deepseek-chat"),
            ("MEMAYU_EMBEDDER_BASE_URL", "https://api.openai.com/v1"),
            ("MEMAYU_EMBEDDER_API_KEY", "emb-key"),
            ("MEMAYU_EMBEDDER_MODEL", "text-embedding-3-small"),
        ])
    }

    // ── existing env-only tests ──

    #[test]
    fn defaults_applied() {
        let cfg = Config::from_env(&minimal_env()).unwrap();
        assert_eq!(cfg.storage.backend, StorageBackend::Libsql);
        assert_eq!(cfg.storage.libsql_path, "./memayu.db");
        assert_eq!(cfg.server.port, 18080);
        assert!((cfg.similarity_threshold - 0.65).abs() < 1e-6);
        assert_eq!(cfg.dimension, None);
    }

    #[test]
    fn missing_required_key_fails_with_specific_error() {
        let env = env_with(&[
            ("MEMAYU_LLM_BASE_URL", "https://x"),
            ("MEMAYU_EMBEDDER_BASE_URL", "https://x"),
            ("MEMAYU_EMBEDDER_MODEL", "m"),
        ]);
        let err = Config::from_env(&env).unwrap_err();
        assert!(matches!(err, ConfigError::MissingField { .. }));
    }

    #[test]
    fn invalid_backend_rejected() {
        let mut env = minimal_env();
        env.insert("MEMAYU_STORAGE_BACKEND".into(), "sqlite".into());
        let err = Config::from_env(&env).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }));
    }

    #[test]
    fn postgres_requires_database_url() {
        let mut env = minimal_env();
        env.insert("MEMAYU_STORAGE_BACKEND".into(), "postgres".into());
        // env-only: missing database_url → empty is fine (not a hard error in the new model)
        // Postgres with no URL just uses empty string
        let cfg = Config::from_env(&env).unwrap();
        assert_eq!(cfg.storage.backend, StorageBackend::Postgres);
    }

    #[test]
    fn postgres_loads_url() {
        let mut env = minimal_env();
        env.insert("MEMAYU_STORAGE_BACKEND".into(), "postgres".into());
        env.insert("MEMAYU_DATABASE_URL".into(), "postgres://u:p@h/db".into());
        env.insert("MEMAYU_PORT".into(), "9000".into());
        env.insert("MEMAYU_EMBEDDER_DIM".into(), "768".into());
        let cfg = Config::from_env(&env).unwrap();
        assert_eq!(cfg.storage.backend, StorageBackend::Postgres);
        assert_eq!(
            cfg.storage.database_url.as_deref(),
            Some("postgres://u:p@h/db")
        );
        assert_eq!(cfg.server.port, 9000);
        assert_eq!(cfg.dimension, Some(768));
    }

    #[test]
    fn cloud_mode_accepts_any_url_string() {
        let cfg = Config::from_env(&env_with(&[("MEMAYU_API_URL", "memayu")])).unwrap();
        assert_eq!(cfg.api_url.as_deref(), Some("memayu"));
    }

    #[test]
    fn cloud_mode_skips_llm_validation() {
        let cfg =
            Config::from_env(&env_with(&[("MEMAYU_API_URL", "https://api.example.com")])).unwrap();
        assert_eq!(cfg.api_url.as_deref(), Some("https://api.example.com"));
        assert_eq!(cfg.llm.model, "");
    }

    #[test]
    fn non_cloud_mode_still_requires_llm_vars() {
        let err = Config::from_env(&HashMap::new()).unwrap_err();
        assert!(matches!(err, ConfigError::MissingField { .. }));
    }

    #[test]
    fn cloud_mode_with_api_key() {
        let env = env_with(&[
            ("MEMAYU_API_URL", "https://api.example.com"),
            ("MEMAYU_API_KEY", "secret123"),
            ("MEMAYU_SIMILARITY_THRESHOLD", "0.9"),
        ]);
        let cfg = Config::from_env(&env).unwrap();
        assert_eq!(cfg.api_url.as_deref(), Some("https://api.example.com"));
        assert_eq!(cfg.api_key.as_deref(), Some("secret123"));
        assert!((cfg.similarity_threshold - 0.9).abs() < 1e-6);
    }

    // ── TOML file merge tests ──

    #[test]
    fn toml_overrides_defaults() {
        let toml = indoc::indoc! {r#"
            [storage]
            backend = "libsql"
            libsql_path = "/data/memayu.db"

            [llm]
            base_url = "https://llm.example.com/v1"
            api_key = "secret-llm"
            model = "gpt-4"

            [embedder]
            base_url = "https://emb.example.com/v1"
            api_key = "secret-emb"
            model = "text-embedding-3-large"

            [server]
            bind_addr = "0.0.0.0"
            port = 9000

            [behavior]
            similarity_threshold = 0.92
        "#};
        let cf: ConfigFile = toml::from_str(toml).unwrap();
        let cfg = Config::merge(Some(cf), &HashMap::new()).unwrap();
        assert_eq!(cfg.storage.libsql_path, "/data/memayu.db");
        assert_eq!(cfg.llm.base_url, "https://llm.example.com/v1");
        assert_eq!(cfg.llm.model, "gpt-4");
        assert_eq!(cfg.embedder.model, "text-embedding-3-large");
        assert_eq!(cfg.server.port, 9000);
        assert!((cfg.similarity_threshold - 0.92).abs() < 1e-6);
    }

    #[test]
    fn env_overrides_toml() {
        let toml = indoc::indoc! {r#"
            [llm]
            base_url = "https://file.example.com/v1"
            model = "file-model"

            [embedder]
            base_url = "https://file-emb.example.com/v1"
            model = "file-emb-model"
        "#};
        let cf: ConfigFile = toml::from_str(toml).unwrap();
        let env = env_with(&[
            ("MEMAYU_LLM_BASE_URL", "https://env.example.com/v1"),
            ("MEMAYU_LLM_MODEL", "env-model"),
            ("MEMAYU_EMBEDDER_BASE_URL", "https://env-emb.example.com/v1"),
            ("MEMAYU_EMBEDDER_MODEL", "env-emb-model"),
        ]);
        let cfg = Config::merge(Some(cf), &env).unwrap();
        assert_eq!(cfg.llm.base_url, "https://env.example.com/v1");
        assert_eq!(cfg.llm.model, "env-model");
        assert_eq!(cfg.embedder.base_url, "https://env-emb.example.com/v1");
        assert_eq!(cfg.embedder.model, "env-emb-model");
    }

    #[test]
    fn toml_missing_sections_default() {
        let toml = indoc::indoc! {r#"
            [llm]
            base_url = "https://llm.example.com/v1"
            model = "gpt-4"

            [embedder]
            base_url = "https://emb.example.com/v1"
            model = "text-embedding-3-large"
        "#};
        let cf: ConfigFile = toml::from_str(toml).unwrap();
        let cfg = Config::merge(Some(cf), &HashMap::new()).unwrap();
        assert_eq!(cfg.storage.backend, StorageBackend::Libsql);
        assert_eq!(cfg.server.port, 18080);
        assert!((cfg.similarity_threshold - 0.65).abs() < 1e-6);
    }

    #[test]
    fn check_reports_missing_fields() {
        let cfg = Config {
            storage: StorageConfig {
                backend: StorageBackend::Libsql,
                libsql_path: String::new(),
                database_url: None,
            },
            llm: ProviderConfig {
                backend: EmbedderBackend::Remote,
                base_url: String::new(),
                api_key: None,
                model: String::new(),
            },
            embedder: ProviderConfig {
                backend: EmbedderBackend::Remote,
                base_url: String::new(),
                api_key: None,
                model: String::new(),
            },
            server: ServerConfig {
                bind_addr: "127.0.0.1".into(),
                port: 18080,
            },
            similarity_threshold: 0.65,
            extraction_mode: ExtractionMode::Llm,
            dimension: None,
            api_url: None,
            api_key: None,
        };
        let issues = cfg.check();
        assert!(!issues.is_empty());
        assert!(issues.iter().any(|m| m.contains("llm.base_url")));
        assert!(issues.iter().any(|m| m.contains("llm.model")));
        assert!(issues.iter().any(|m| m.contains("embedder.base_url")));
        assert!(issues.iter().any(|m| m.contains("embedder.model")));
    }

    #[test]
    fn check_valid_config_empty() {
        let cfg = Config::from_env(&minimal_env()).unwrap();
        assert!(cfg.check().is_empty());
    }

    #[test]
    fn show_redacts_secrets() {
        let cfg = Config::from_env(&minimal_env()).unwrap();
        let out = cfg.show();
        assert!(!out.contains("llm-key"));
        assert!(out.contains("***"));
    }

    #[test]
    fn config_path_respects_env_var() {
        std::env::set_var("MEMAYU_CONFIG", "/tmp/test-config.toml");
        assert_eq!(config_path(), PathBuf::from("/tmp/test-config.toml"));
        std::env::remove_var("MEMAYU_CONFIG");
    }

    // ── extraction_mode ──

    #[test]
    fn extraction_mode_defaults_to_llm() {
        let cfg = Config::from_env(&minimal_env()).unwrap();
        assert_eq!(cfg.extraction_mode, ExtractionMode::Llm);
    }

    #[test]
    fn extraction_mode_env_raw() {
        let mut env = minimal_env();
        env.insert("MEMAYU_EXTRACTION_MODE".into(), "raw".into());
        let cfg = Config::from_env(&env).unwrap();
        assert_eq!(cfg.extraction_mode, ExtractionMode::Raw);
    }

    #[test]
    fn raw_mode_does_not_require_llm_config() {
        // Only embedder is configured: raw mode must accept a fully absent LLM.
        let env = env_with(&[
            ("MEMAYU_EXTRACTION_MODE", "raw"),
            ("MEMAYU_EMBEDDER_BASE_URL", "https://api.openai.com/v1"),
            ("MEMAYU_EMBEDDER_API_KEY", "emb-key"),
            ("MEMAYU_EMBEDDER_MODEL", "text-embedding-3-small"),
        ]);
        let cfg = Config::from_env(&env).unwrap();
        assert_eq!(cfg.extraction_mode, ExtractionMode::Raw);
        assert!(cfg.llm.base_url.is_empty());
        assert!(cfg.llm.model.is_empty());
        assert!(cfg.check().is_empty(), "raw mode must validate cleanly");
    }

    #[test]
    fn llm_mode_still_requires_llm_config() {
        let env = env_with(&[
            ("MEMAYU_EMBEDDER_BASE_URL", "https://api.openai.com/v1"),
            ("MEMAYU_EMBEDDER_MODEL", "text-embedding-3-small"),
        ]);
        let err = Config::from_env(&env).unwrap_err();
        assert!(matches!(err, ConfigError::MissingField { .. }));
    }

    #[test]
    fn extraction_mode_env_llm_explicit() {
        let mut env = minimal_env();
        env.insert("MEMAYU_EXTRACTION_MODE".into(), "llm".into());
        let cfg = Config::from_env(&env).unwrap();
        assert_eq!(cfg.extraction_mode, ExtractionMode::Llm);
    }

    #[test]
    fn extraction_mode_invalid_rejected() {
        let mut env = minimal_env();
        env.insert("MEMAYU_EXTRACTION_MODE".into(), "hybrid".into());
        let err = Config::from_env(&env).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }));
    }

    #[test]
    fn extraction_mode_from_toml() {
        let toml = indoc::indoc! {r#"
            [llm]
            base_url = "https://llm.example.com/v1"
            model = "gpt-4"

            [embedder]
            base_url = "https://emb.example.com/v1"
            model = "text-embedding-3-large"

            [behavior]
            extraction_mode = "raw"
        "#};
        let cf: ConfigFile = toml::from_str(toml).unwrap();
        let cfg = Config::merge(Some(cf), &HashMap::new()).unwrap();
        assert_eq!(cfg.extraction_mode, ExtractionMode::Raw);
    }

    #[test]
    fn env_extraction_mode_overrides_toml() {
        let toml = indoc::indoc! {r#"
            [llm]
            base_url = "https://llm.example.com/v1"
            model = "gpt-4"

            [embedder]
            base_url = "https://emb.example.com/v1"
            model = "text-embedding-3-large"

            [behavior]
            extraction_mode = "raw"
        "#};
        let cf: ConfigFile = toml::from_str(toml).unwrap();
        let env = env_with(&[("MEMAYU_EXTRACTION_MODE", "llm")]);
        let cfg = Config::merge(Some(cf), &env).unwrap();
        assert_eq!(cfg.extraction_mode, ExtractionMode::Llm);
    }

    #[test]
    fn show_includes_extraction_mode() {
        let cfg = Config::from_env(&minimal_env()).unwrap();
        let out = cfg.show();
        assert!(out.contains("extraction_mode = \"llm\""));
    }
}
