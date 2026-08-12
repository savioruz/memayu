mod error;

pub use error::ConfigError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBackend {
    Libsql,
    Postgres,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub backend: StorageBackend,
    pub libsql_path: String,
    pub postgres_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub storage: StorageConfig,
    pub llm: ProviderConfig,
    pub embedder: ProviderConfig,
    pub server: ServerConfig,
    pub similarity_threshold: f32,
    pub dimension: Option<usize>,
    /// Cloud-mode: API endpoint (MEMAYU_API_URL). When set, LLM/embedder/storage validation is skipped.
    pub api_url: Option<String>,
    pub api_key: Option<String>,
}

fn env_required(
    env: &std::collections::HashMap<String, String>,
    var: &'static str,
) -> Result<String, ConfigError> {
    env.get(var)
        .filter(|s| !s.is_empty())
        .cloned()
        .ok_or(ConfigError::Missing(var))
}

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

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        Self::from_env(&std::env::vars().collect())
    }

    fn from_env(env: &std::collections::HashMap<String, String>) -> Result<Self, ConfigError> {
        let api_url = env.get("MEMAYU_API_URL").cloned().filter(|s| !s.is_empty());
        let api_key = env.get("MEMAYU_API_KEY").cloned().filter(|s| !s.is_empty());

        // Cloud mode: skip LLM/embedder/storage validation.
        // The `serve` subcommand must reject cloud-only config at dispatch time.
        if api_url.is_some() {
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
            return Ok(Self {
                storage: StorageConfig {
                    backend: StorageBackend::Libsql,
                    libsql_path: String::new(),
                    postgres_url: None,
                },
                llm: ProviderConfig {
                    base_url: String::new(),
                    api_key: None,
                    model: String::new(),
                },
                embedder: ProviderConfig {
                    base_url: String::new(),
                    api_key: None,
                    model: String::new(),
                },
                server: ServerConfig {
                    bind_addr: "0.0.0.0".into(),
                    port: 8080,
                },
                similarity_threshold: sim,
                dimension: None,
                api_url,
                api_key,
            });
        }

        let backend = match env.get("MEMAYU_STORAGE_BACKEND").map(String::as_str) {
            None | Some("libsql") => StorageBackend::Libsql,
            Some("postgres") => StorageBackend::Postgres,
            Some(other) => {
                return Err(ConfigError::Invalid {
                    var: "MEMAYU_STORAGE_BACKEND",
                    value: other.to_string(),
                    detail: "expected \"libsql\" or \"postgres\"".into(),
                })
            }
        };

        let postgres_url = match backend {
            StorageBackend::Libsql => None,
            StorageBackend::Postgres => Some(env_required(env, "MEMAYU_DATABASE_URL")?),
        };

        Ok(Self {
            storage: StorageConfig {
                backend,
                libsql_path: env
                    .get("MEMAYU_LIBSQL_PATH")
                    .cloned()
                    .unwrap_or_else(|| "memayu.db".to_string()),
                postgres_url,
            },
            llm: ProviderConfig {
                base_url: validate_url(
                    "MEMAYU_LLM_BASE_URL",
                    env_required(env, "MEMAYU_LLM_BASE_URL")?,
                )?,
                api_key: env
                    .get("MEMAYU_LLM_API_KEY")
                    .cloned()
                    .filter(|s| !s.is_empty()),
                model: env_required(env, "MEMAYU_LLM_MODEL")?,
            },
            embedder: ProviderConfig {
                base_url: validate_url(
                    "MEMAYU_EMBEDDER_BASE_URL",
                    env_required(env, "MEMAYU_EMBEDDER_BASE_URL")?,
                )?,
                api_key: env
                    .get("MEMAYU_EMBEDDER_API_KEY")
                    .cloned()
                    .filter(|s| !s.is_empty()),
                model: env_required(env, "MEMAYU_EMBEDDER_MODEL")?,
            },
            server: ServerConfig {
                bind_addr: env
                    .get("MEMAYU_BIND_ADDR")
                    .cloned()
                    .unwrap_or_else(|| "0.0.0.0".to_string()),
                port: env
                    .get("MEMAYU_PORT")
                    .map(|p| {
                        p.parse().map_err(|_| ConfigError::Invalid {
                            var: "MEMAYU_PORT",
                            value: p.clone(),
                            detail: "expected a u16 port number".into(),
                        })
                    })
                    .transpose()?
                    .unwrap_or(8080),
            },
            similarity_threshold: env
                .get("MEMAYU_SIMILARITY_THRESHOLD")
                .map(|v| {
                    v.parse().map_err(|_| ConfigError::Invalid {
                        var: "MEMAYU_SIMILARITY_THRESHOLD",
                        value: v.clone(),
                        detail: "expected a float between 0.0 and 1.0".into(),
                    })
                })
                .transpose()?
                .unwrap_or(0.55),
            dimension: env
                .get("MEMAYU_EMBEDDING_DIM")
                .map(|v| {
                    v.parse().map_err(|_| ConfigError::Invalid {
                        var: "MEMAYU_EMBEDDING_DIM",
                        value: v.clone(),
                        detail: "expected a positive integer".into(),
                    })
                })
                .transpose()?,
            api_url: None,
            api_key: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.to_string());
        }
        m
    }

    fn minimal_env() -> std::collections::HashMap<String, String> {
        env_with(&[
            ("MEMAYU_LLM_BASE_URL", "https://api.deepseek.com/v1"),
            ("MEMAYU_LLM_API_KEY", "llm-key"),
            ("MEMAYU_LLM_MODEL", "deepseek-chat"),
            ("MEMAYU_EMBEDDER_BASE_URL", "https://api.openai.com/v1"),
            ("MEMAYU_EMBEDDER_API_KEY", "emb-key"),
            ("MEMAYU_EMBEDDER_MODEL", "text-embedding-3-small"),
        ])
    }

    #[test]
    fn defaults_applied() {
        let cfg = Config::from_env(&minimal_env()).unwrap();
        assert_eq!(cfg.storage.backend, StorageBackend::Libsql);
        assert_eq!(cfg.storage.libsql_path, "memayu.db");
        assert_eq!(cfg.server.port, 8080);
        assert!((cfg.similarity_threshold - 0.55).abs() < 1e-6);
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
        assert!(matches!(err, ConfigError::Missing("MEMAYU_LLM_MODEL")));
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
        let err = Config::from_env(&env).unwrap_err();
        assert!(matches!(err, ConfigError::Missing("MEMAYU_DATABASE_URL")));
    }

    #[test]
    fn postgres_loads_url() {
        let mut env = minimal_env();
        env.insert("MEMAYU_STORAGE_BACKEND".into(), "postgres".into());
        env.insert("MEMAYU_DATABASE_URL".into(), "postgres://u:p@h/db".into());
        env.insert("MEMAYU_PORT".into(), "9000".into());
        env.insert("MEMAYU_EMBEDDING_DIM".into(), "768".into());
        let cfg = Config::from_env(&env).unwrap();
        assert_eq!(cfg.storage.backend, StorageBackend::Postgres);
        assert_eq!(
            cfg.storage.postgres_url.as_deref(),
            Some("postgres://u:p@h/db")
        );
        assert_eq!(cfg.server.port, 9000);
        assert_eq!(cfg.dimension, Some(768));
    }

    #[test]
    fn cloud_mode_accepts_any_url_string() {
        // Cloud mode path never validates URL — it just uses the string.
        let cfg = Config::from_env(&env_with(&[("MEMAYU_API_URL", "memayu")])).unwrap();
        assert_eq!(cfg.api_url.as_deref(), Some("memayu"));
    }

    #[test]
    fn cloud_mode_skips_llm_validation() {
        // Only MEMAYU_API_URL set; normally missing LLM vars would fail.
        let cfg =
            Config::from_env(&env_with(&[("MEMAYU_API_URL", "https://api.example.com")])).unwrap();
        assert_eq!(cfg.api_url.as_deref(), Some("https://api.example.com"));
        assert_eq!(cfg.llm.model, "");
    }

    #[test]
    fn non_cloud_mode_still_requires_llm_vars() {
        // Without MEMAYU_API_URL, we go through normal path and need LLM vars.
        let err = Config::from_env(&std::collections::HashMap::new()).unwrap_err();
        assert!(matches!(err, ConfigError::Missing(_)));
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
}
