use crate::infrastructure::db::DbClient;
use crate::modules::api_keys::model::ApiKey;

impl DbClient {
    // ── API Keys ──

    pub async fn api_keys_empty(&self) -> Result<bool, String> {
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query("SELECT COUNT(*) FROM api_keys", ())
                    .await
                    .map_err(|e| format!("count api_keys: {e}"))?;
                let row = rows
                    .next()
                    .await
                    .map_err(|e| format!("read count: {e}"))?
                    .ok_or_else(|| "no api_keys row".to_string())?;
                let count: i64 = row.get(0).map_err(|e| format!("read count value: {e}"))?;
                Ok(count == 0)
            }
            DbClient::Postgres(pool) => {
                let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_keys")
                    .fetch_one(pool)
                    .await
                    .map_err(|e| format!("count api_keys: {e}"))?;
                Ok(count == 0)
            }
        }
    }

    pub async fn list_api_keys(&self) -> Result<Vec<ApiKey>, String> {
        let mut out = Vec::new();
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query(
                        "SELECT id, label, key_prefix, last_used_at, created_at FROM api_keys ORDER BY created_at",
                        (),
                    )
                    .await
                    .map_err(|e| format!("list api_keys: {e}"))?;
                while let Some(row) = rows.next().await.map_err(|e| format!("read key: {e}"))? {
                    out.push(ApiKey {
                        id: row.get(0).map_err(|e| format!("id: {e}"))?,
                        label: row.get(1).map_err(|e| format!("label: {e}"))?,
                        key_prefix: row.get(2).map_err(|e| format!("key_prefix: {e}"))?,
                        last_used_at: row.get(3).map_err(|e| format!("last_used: {e}"))?,
                        created_at: row
                            .get::<String>(4)
                            .map_err(|e| format!("created_at: {e}"))?,
                    });
                }
            }
            DbClient::Postgres(pool) => {
                let rows: Vec<(String, String, String, Option<String>, String)> =
                    sqlx::query_as("SELECT id, label, key_prefix, last_used_at, created_at FROM api_keys ORDER BY created_at")
                        .fetch_all(pool)
                        .await
                        .map_err(|e| format!("list api_keys: {e}"))?;
                for (id, label, key_prefix, last_used_at, created_at) in rows {
                    out.push(ApiKey {
                        id,
                        label,
                        key_prefix,
                        last_used_at,
                        created_at,
                    });
                }
            }
        }
        Ok(out)
    }

    pub async fn insert_api_key(
        &self,
        id: &str,
        user_id: &str,
        label: &str,
        key_prefix: &str,
        key_hash: &str,
    ) -> Result<(), String> {
        let created = chrono::Utc::now().to_rfc3339();
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "INSERT INTO api_keys (id, user_id, label, key_prefix, key_hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    (id, user_id, label, key_prefix, key_hash, created.as_str()),
                )
                .await
                .map_err(|e| format!("insert api_key: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO api_keys (id, user_id, label, key_prefix, key_hash, created_at) VALUES ($1, $2, $3, $4, $5, $6)",
                )
                .bind(id)
                .bind(user_id)
                .bind(label)
                .bind(key_prefix)
                .bind(key_hash)
                .bind(created)
                .execute(pool)
                .await
                .map_err(|e| format!("insert api_key: {e}"))?;
            }
        }
        Ok(())
    }

    pub async fn delete_api_key(&self, id: &str) -> Result<bool, String> {
        match self {
            DbClient::Libsql(conn) => {
                let changes = conn
                    .execute("DELETE FROM api_keys WHERE id = ?1", [id])
                    .await
                    .map_err(|e| format!("delete api_key: {e}"))?;
                Ok(changes > 0)
            }
            DbClient::Postgres(pool) => {
                let result = sqlx::query("DELETE FROM api_keys WHERE id = $1")
                    .bind(id)
                    .execute(pool)
                    .await
                    .map_err(|e| format!("delete api_key: {e}"))?;
                Ok(result.rows_affected() > 0)
            }
        }
    }

    pub async fn find_api_key_by_hash(&self, key_hash: &str) -> Result<Option<String>, String> {
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query(
                        "SELECT user_id FROM api_keys WHERE key_hash = ?1",
                        vec![key_hash],
                    )
                    .await
                    .map_err(|e| format!("find api_key: {e}"))?;
                if let Some(row) = rows
                    .next()
                    .await
                    .map_err(|e| format!("read api_key: {e}"))?
                {
                    Ok(Some(row.get(0).map_err(|e| format!("user_id: {e}"))?))
                } else {
                    Ok(None)
                }
            }
            DbClient::Postgres(pool) => {
                let row: Option<(String,)> =
                    sqlx::query_as("SELECT user_id FROM api_keys WHERE key_hash = $1")
                        .bind(key_hash)
                        .fetch_optional(pool)
                        .await
                        .map_err(|e| format!("find api_key: {e}"))?;
                Ok(row.map(|(id,)| id))
            }
        }
    }

    pub async fn touch_api_key(&self, key_hash: &str) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "UPDATE api_keys SET last_used_at = ?1 WHERE key_hash = ?2",
                    (now.as_str(), key_hash),
                )
                .await
                .map_err(|e| format!("touch api_key: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query("UPDATE api_keys SET last_used_at = $1 WHERE key_hash = $2")
                    .bind(&now)
                    .bind(key_hash)
                    .execute(pool)
                    .await
                    .map_err(|e| format!("touch api_key: {e}"))?;
            }
        }
        Ok(())
    }
}
