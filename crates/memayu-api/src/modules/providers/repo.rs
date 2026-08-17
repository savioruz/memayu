use crate::infrastructure::db::DbClient;
use memayu_config::EmbedderBackend;
use std::collections::HashMap;

impl DbClient {
    // ── Provider config ──

    /// All provider rows keyed by `provider` (`"llm"` / `"embedder"`).
    ///
    /// Each value is the `(backend, base_url, api_key, model)` tuple. The
    /// `backend` field is DB-authoritative and is only meaningful for the
    /// embedder row (the LLM row is always `remote`).
    pub async fn provider_configs(
        &self,
    ) -> Result<HashMap<String, (String, String, String, String)>, String> {
        let mut out = HashMap::new();
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query(
                        "SELECT provider, backend, base_url, api_key, model FROM provider_config",
                        (),
                    )
                    .await
                    .map_err(|e| format!("list provider configs: {e}"))?;
                while let Some(row) = rows.next().await.map_err(|e| format!("read config: {e}"))? {
                    out.insert(
                        row.get(0).map_err(|e| format!("provider: {e}"))?,
                        (
                            row.get(1).map_err(|e| format!("backend: {e}"))?,
                            row.get(2).map_err(|e| format!("base_url: {e}"))?,
                            row.get(3).map_err(|e| format!("api_key: {e}"))?,
                            row.get(4).map_err(|e| format!("model: {e}"))?,
                        ),
                    );
                }
            }
            DbClient::Postgres(pool) => {
                let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
                    "SELECT provider, backend, base_url, api_key, model FROM provider_config",
                )
                .fetch_all(pool)
                .await
                .map_err(|e| format!("list provider configs: {e}"))?;
                for (provider, backend, base_url, api_key, model) in rows {
                    out.insert(provider, (backend, base_url, api_key, model));
                }
            }
        }
        Ok(out)
    }

    pub async fn upsert_provider_config(
        &self,
        provider: &str,
        backend: &str,
        base_url: &str,
        api_key: &str,
        model: &str,
    ) -> Result<(), String> {
        // Single normalization choke point: a local embedder needs neither a
        // base_url nor an api_key, so clear them (empty string) whenever the
        // backend is local — even if the caller passed a stale value from a
        // prior remote configuration. Every write path (setup wizard, boot
        // seeding, the web `/providers` form, `POST /api/providers`) funnels
        // through here, so this one guard prevents the "some paths clear, some
        // don't" failure mode.
        let (base_url, api_key) = if provider == "embedder"
            && matches!(
                backend.parse::<EmbedderBackend>(),
                Ok(EmbedderBackend::Local)
            ) {
            ("", "")
        } else {
            (base_url, api_key)
        };
        let updated = chrono::Utc::now().to_rfc3339();
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "INSERT INTO provider_config (provider, backend, base_url, api_key, model, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (provider) DO UPDATE SET
                       backend = excluded.backend,
                       base_url = excluded.base_url,
                       api_key = excluded.api_key,
                       model = excluded.model,
                       updated_at = excluded.updated_at",
                    (provider, backend, base_url, api_key, model, updated.as_str()),
                )
                .await
                .map_err(|e| format!("upsert provider config: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO provider_config (provider, backend, base_url, api_key, model, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6)
                     ON CONFLICT (provider) DO UPDATE SET
                       backend = EXCLUDED.backend,
                       base_url = EXCLUDED.base_url,
                       api_key = EXCLUDED.api_key,
                       model = EXCLUDED.model,
                       updated_at = EXCLUDED.updated_at",
                )
                .bind(provider)
                .bind(backend)
                .bind(base_url)
                .bind(api_key)
                .bind(model)
                .bind(updated)
                .execute(pool)
                .await
                .map_err(|e| format!("upsert provider config: {e}"))?;
            }
        }
        Ok(())
    }

    // ── Runtime settings ──

    /// Read the stored `extraction_mode` from `runtime_settings`, if present.
    pub async fn get_extraction_mode(&self) -> Result<Option<String>, String> {
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query(
                        "SELECT value FROM runtime_settings WHERE key = 'extraction_mode'",
                        (),
                    )
                    .await
                    .map_err(|e| format!("get extraction_mode: {e}"))?;
                match rows.next().await.map_err(|e| format!("read mode: {e}"))? {
                    Some(row) => row.get(0).map(Some).map_err(|e| format!("mode value: {e}")),
                    None => Ok(None),
                }
            }
            DbClient::Postgres(pool) => {
                let value: Option<String> = sqlx::query_scalar(
                    "SELECT value FROM runtime_settings WHERE key = 'extraction_mode'",
                )
                .fetch_optional(pool)
                .await
                .map_err(|e| format!("get extraction_mode: {e}"))?;
                Ok(value)
            }
        }
    }

    /// Upsert the stored `extraction_mode` into `runtime_settings`.
    pub async fn set_extraction_mode(&self, mode: &str) -> Result<(), String> {
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "INSERT INTO runtime_settings (key, value) VALUES ('extraction_mode', ?1)
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                    [mode],
                )
                .await
                .map_err(|e| format!("set extraction_mode: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO runtime_settings (key, value) VALUES ('extraction_mode', $1)
                     ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
                )
                .bind(mode)
                .execute(pool)
                .await
                .map_err(|e| format!("set extraction_mode: {e}"))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memayu_config::{StorageBackend, StorageConfig};

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
    async fn local_embedder_clears_base_url_and_api_key() {
        let db = mem_db().await;
        db.upsert_provider_config(
            "embedder",
            "local",
            "https://api.openai.com/v1",
            "sk-stale",
            "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
        )
        .await
        .unwrap();
        let rows = db.provider_configs().await.unwrap();
        let (backend, base_url, api_key, model) = &rows["embedder"];
        assert_eq!(backend, "local");
        assert_eq!(base_url, "", "local embedder must not persist a base_url");
        assert_eq!(api_key, "", "local embedder must not persist an api_key");
        assert_eq!(
            model,
            "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"
        );
    }

    #[tokio::test]
    async fn remote_embedder_keeps_base_url_and_api_key() {
        let db = mem_db().await;
        db.upsert_provider_config("embedder", "remote", "http://localhost:11434", "sk", "m")
            .await
            .unwrap();
        let rows = db.provider_configs().await.unwrap();
        let (backend, base_url, api_key, _) = &rows["embedder"];
        assert_eq!(backend, "remote");
        assert_eq!(base_url, "http://localhost:11434");
        assert_eq!(api_key, "sk");
    }

    #[tokio::test]
    async fn switching_remote_to_local_clears_stale_base_url() {
        let db = mem_db().await;
        // Prior remote config with a real base_url.
        db.upsert_provider_config(
            "embedder",
            "remote",
            "https://api.openai.com/v1",
            "sk-old",
            "m",
        )
        .await
        .unwrap();
        // Switch to local — the stale base_url/api_key must be cleared, not carried over.
        db.upsert_provider_config(
            "embedder",
            "local",
            "https://api.openai.com/v1",
            "sk-old",
            "m",
        )
        .await
        .unwrap();
        let rows = db.provider_configs().await.unwrap();
        let (backend, base_url, api_key, _) = &rows["embedder"];
        assert_eq!(backend, "local");
        assert_eq!(
            base_url, "",
            "stale base_url from a prior remote config must be cleared"
        );
        assert_eq!(
            api_key, "",
            "stale api_key from a prior remote config must be cleared"
        );
    }
}
