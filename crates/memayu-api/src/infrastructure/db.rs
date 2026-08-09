use memayu_config::StorageConfig;

/// Unified database connection — wraps either a local libsql connection or a
/// remote Postgres pool so the rest of the crate stays backend-agnostic.
#[derive(Clone)]
pub enum DbClient {
    Libsql(libsql::Connection),
    Postgres(sqlx::PgPool),
}

impl DbClient {
    /// Open a database connection based on the `memayu_config::StorageConfig`.
    pub async fn open(config: &StorageConfig) -> Result<Self, String> {
        match config.backend {
            memayu_config::StorageBackend::Libsql => {
                let db = libsql::Builder::new_local(&config.libsql_path)
                    .build()
                    .await
                    .map_err(|e| format!("open libsql for api: {e}"))?;
                let conn = db
                    .connect()
                    .map_err(|e| format!("connect libsql for api: {e}"))?;
                Ok(DbClient::Libsql(conn))
            }
            memayu_config::StorageBackend::Postgres => {
                let url = config
                    .postgres_url
                    .as_deref()
                    .ok_or_else(|| "missing postgres url for api".to_string())?;
                let pool = sqlx::PgPool::connect(url)
                    .await
                    .map_err(|e| format!("connect postgres for api: {e}"))?;
                Ok(DbClient::Postgres(pool))
            }
        }
    }

    /// Run DDL migrations — creates all required tables if they don't exist.
    pub async fn init(&self) -> Result<(), String> {
        self.init_users().await?;
        self.init_sessions().await?;
        self.init_provider_config().await?;
        self.init_api_keys().await?;
        self.init_request_logs().await?;
        Ok(())
    }

    // ── DDL helpers ──

    async fn init_users(&self) -> Result<(), String> {
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS users (
                        id TEXT PRIMARY KEY,
                        email TEXT NOT NULL UNIQUE,
                        password TEXT NOT NULL,
                        salt TEXT NOT NULL,
                        is_admin INTEGER NOT NULL DEFAULT 1,
                        created_at TEXT NOT NULL
                    )",
                    (),
                )
                .await
                .map_err(|e| format!("create users table: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS users (
                        id TEXT PRIMARY KEY,
                        email TEXT NOT NULL UNIQUE,
                        password TEXT NOT NULL,
                        salt TEXT NOT NULL,
                        is_admin INTEGER NOT NULL DEFAULT 1,
                        created_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await
                .map_err(|e| format!("create users table: {e}"))?;
            }
        }
        Ok(())
    }

    async fn init_sessions(&self) -> Result<(), String> {
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS sessions (
                        id TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        created_at TEXT NOT NULL,
                        expires_at TEXT NOT NULL
                    )",
                    (),
                )
                .await
                .map_err(|e| format!("create sessions table: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS sessions (
                        id TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        created_at TEXT NOT NULL,
                        expires_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await
                .map_err(|e| format!("create sessions table: {e}"))?;
            }
        }
        Ok(())
    }

    async fn init_provider_config(&self) -> Result<(), String> {
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS provider_config (
                        provider TEXT PRIMARY KEY,
                        base_url TEXT NOT NULL,
                        api_key TEXT NOT NULL,
                        model TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                    (),
                )
                .await
                .map_err(|e| format!("create provider_config table: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS provider_config (
                        provider TEXT PRIMARY KEY,
                        base_url TEXT NOT NULL,
                        api_key TEXT NOT NULL,
                        model TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await
                .map_err(|e| format!("create provider_config table: {e}"))?;
            }
        }
        Ok(())
    }

    async fn init_api_keys(&self) -> Result<(), String> {
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS api_keys (
                        id TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        label TEXT NOT NULL DEFAULT '',
                        key_hash TEXT NOT NULL,
                        key_prefix TEXT NOT NULL DEFAULT '',
                        created_at TEXT NOT NULL,
                        last_used_at TEXT
                    )",
                    (),
                )
                .await
                .map_err(|e| format!("create api_keys table: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS api_keys (
                        id TEXT PRIMARY KEY,
                        user_id TEXT NOT NULL,
                        label TEXT NOT NULL DEFAULT '',
                        key_hash TEXT NOT NULL,
                        key_prefix TEXT NOT NULL DEFAULT '',
                        created_at TEXT NOT NULL,
                        last_used_at TEXT
                    )",
                )
                .execute(pool)
                .await
                .map_err(|e| format!("create api_keys table: {e}"))?;
            }
        }
        Ok(())
    }

    async fn init_request_logs(&self) -> Result<(), String> {
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS request_logs (
                        id TEXT PRIMARY KEY,
                        created_at TEXT NOT NULL,
                        method TEXT NOT NULL,
                        path TEXT NOT NULL,
                        status INTEGER NOT NULL,
                        latency_ms REAL NOT NULL,
                        auth TEXT NOT NULL DEFAULT 'none'
                    )",
                    (),
                )
                .await
                .map_err(|e| format!("create request_logs: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS request_logs (
                        id TEXT PRIMARY KEY,
                        created_at TEXT NOT NULL,
                        method TEXT NOT NULL,
                        path TEXT NOT NULL,
                        status INTEGER NOT NULL,
                        latency_ms REAL NOT NULL,
                        auth TEXT NOT NULL DEFAULT 'none'
                    )",
                )
                .execute(pool)
                .await
                .map_err(|e| format!("create request_logs: {e}"))?;
            }
        }
        Ok(())
    }
}
