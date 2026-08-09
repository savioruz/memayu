use crate::infrastructure::db::DbClient;
use std::collections::HashMap;

impl DbClient {
    // ── Provider config ──

    pub async fn provider_configs(
        &self,
    ) -> Result<HashMap<String, (String, String, String)>, String> {
        let mut out = HashMap::new();
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query(
                        "SELECT provider, base_url, api_key, model FROM provider_config",
                        (),
                    )
                    .await
                    .map_err(|e| format!("list provider configs: {e}"))?;
                while let Some(row) = rows.next().await.map_err(|e| format!("read config: {e}"))? {
                    out.insert(
                        row.get(0).map_err(|e| format!("provider: {e}"))?,
                        (
                            row.get(1).map_err(|e| format!("base_url: {e}"))?,
                            row.get(2).map_err(|e| format!("api_key: {e}"))?,
                            row.get(3).map_err(|e| format!("model: {e}"))?,
                        ),
                    );
                }
            }
            DbClient::Postgres(pool) => {
                let rows: Vec<(String, String, String, String)> = sqlx::query_as(
                    "SELECT provider, base_url, api_key, model FROM provider_config",
                )
                .fetch_all(pool)
                .await
                .map_err(|e| format!("list provider configs: {e}"))?;
                for (provider, base_url, api_key, model) in rows {
                    out.insert(provider, (base_url, api_key, model));
                }
            }
        }
        Ok(out)
    }

    pub async fn upsert_provider_config(
        &self,
        provider: &str,
        base_url: &str,
        api_key: &str,
        model: &str,
    ) -> Result<(), String> {
        let updated = chrono::Utc::now().to_rfc3339();
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "INSERT INTO provider_config (provider, base_url, api_key, model, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (provider) DO UPDATE SET
                       base_url = excluded.base_url,
                       api_key = excluded.api_key,
                       model = excluded.model,
                       updated_at = excluded.updated_at",
                    (provider, base_url, api_key, model, updated.as_str()),
                )
                .await
                .map_err(|e| format!("upsert provider config: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO provider_config (provider, base_url, api_key, model, updated_at)
                     VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT (provider) DO UPDATE SET
                       base_url = EXCLUDED.base_url,
                       api_key = EXCLUDED.api_key,
                       model = EXCLUDED.model,
                       updated_at = EXCLUDED.updated_at",
                )
                .bind(provider)
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
}
