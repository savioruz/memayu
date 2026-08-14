use crate::infrastructure::db::DbClient;
use crate::modules::request_logs::model::RequestLog;

impl DbClient {
    // ── Request logs ──

    pub async fn insert_request_log(
        &self,
        method: &str,
        path: &str,
        status: u16,
        latency_ms: f64,
        auth: &str,
    ) -> Result<(), String> {
        let id = uuid::Uuid::new_v4().to_string();
        let created = chrono::Utc::now().to_rfc3339();
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "INSERT INTO request_logs (id, created_at, method, path, status, latency_ms, auth) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    (id.as_str(), created.as_str(), method, path, status as i64, latency_ms, auth),
                )
                .await
                .map_err(|e| format!("insert request_log: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO request_logs (id, created_at, method, path, status, latency_ms, auth) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                )
                .bind(id)
                .bind(created)
                .bind(method)
                .bind(path)
                .bind(status as i64)
                .bind(latency_ms)
                .bind(auth)
                .execute(pool)
                .await
                .map_err(|e| format!("insert request_log: {e}"))?;
            }
        }
        Ok(())
    }

    pub async fn list_request_logs_offset(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RequestLog>, String> {
        let mut out = Vec::new();
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query(
                        "SELECT id, created_at, method, path, status, latency_ms, auth FROM request_logs ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
                        vec![limit as i64, offset as i64],
                    )
                    .await
                    .map_err(|e| format!("list request_logs offset: {e}"))?;
                while let Some(row) = rows.next().await.map_err(|e| format!("read log: {e}"))? {
                    out.push(RequestLog {
                        id: row.get(0).map_err(|e| format!("id: {e}"))?,
                        created_at: row.get(1).map_err(|e| format!("created_at: {e}"))?,
                        method: row.get(2).map_err(|e| format!("method: {e}"))?,
                        path: row.get(3).map_err(|e| format!("path: {e}"))?,
                        status: row.get(4).map_err(|e| format!("status: {e}"))?,
                        latency_ms: row.get(5).map_err(|e| format!("latency: {e}"))?,
                        auth: row.get(6).map_err(|e| format!("auth: {e}"))?,
                    });
                }
            }
            DbClient::Postgres(pool) => {
                let rows: Vec<(String, String, String, String, i64, f64, String)> = sqlx::query_as(
                    "SELECT id, created_at, method, path, status, latency_ms, auth FROM request_logs ORDER BY created_at DESC LIMIT $1 OFFSET $2",
                )
                .bind(limit as i64)
                .bind(offset as i64)
                .fetch_all(pool)
                .await
                .map_err(|e| format!("list request_logs offset: {e}"))?;
                for (id, created_at, method, path, status, latency_ms, auth) in rows {
                    out.push(RequestLog {
                        id,
                        created_at,
                        method,
                        path,
                        status,
                        latency_ms,
                        auth,
                    });
                }
            }
        }
        Ok(out)
    }

    pub async fn count_request_logs(&self) -> Result<i64, String> {
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query("SELECT COUNT(*) FROM request_logs", ())
                    .await
                    .map_err(|e| format!("count request_logs: {e}"))?;
                let row = rows
                    .next()
                    .await
                    .map_err(|e| format!("read count: {e}"))?
                    .ok_or_else(|| "no count row".to_string())?;
                Ok(row.get(0).map_err(|e| format!("read count value: {e}"))?)
            }
            DbClient::Postgres(pool) => {
                let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM request_logs")
                    .fetch_one(pool)
                    .await
                    .map_err(|e| format!("count request_logs: {e}"))?;
                Ok(count)
            }
        }
    }

    pub async fn request_log_stats(&self) -> Result<(i64, f64, f64), String> {
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query(
                        "SELECT COUNT(*), \
                         COALESCE(CAST(AVG(latency_ms) AS REAL), 0.0), \
                         COALESCE(CAST(SUM(CASE WHEN status>=200 AND status<300 THEN 1 ELSE 0 END) AS REAL)/MAX(1,COUNT(*))*100, 0.0) \
                         FROM request_logs",
                        (),
                    )
                    .await
                    .map_err(|e| format!("stats: {e}"))?;
                let row = rows
                    .next()
                    .await
                    .map_err(|e| format!("row: {e}"))?
                    .ok_or("no stats")?;
                Ok((
                    row.get(0).map_err(|e| format!("c: {e}"))?,
                    row.get(1).map_err(|e| format!("a: {e}"))?,
                    row.get(2).map_err(|e| format!("r: {e}"))?,
                ))
            }
            DbClient::Postgres(pool) => {
                let (total, avg_latency, success_rate): (i64, f64, f64) = sqlx::query_as(
                    "SELECT COUNT(*), COALESCE(AVG(latency_ms),0), COALESCE(SUM(CASE WHEN status>=200 AND status<300 THEN 1 ELSE 0 END)::float / GREATEST(1,COUNT(*)) * 100, 0) FROM request_logs"
                )
                .fetch_one(pool)
                .await
                .map_err(|e| format!("stats: {e}"))?;
                Ok((total, avg_latency, success_rate))
            }
        }
    }
}
