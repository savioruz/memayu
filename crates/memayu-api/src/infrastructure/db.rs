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
                    .database_url
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
        self.init_runtime_settings().await?;
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
                        backend TEXT NOT NULL DEFAULT 'remote',
                        base_url TEXT NOT NULL,
                        api_key TEXT NOT NULL,
                        model TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                    (),
                )
                .await
                .map_err(|e| format!("create provider_config table: {e}"))?;
                self.ensure_provider_config_backend_libsql(conn).await?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS provider_config (
                        provider TEXT PRIMARY KEY,
                        backend TEXT NOT NULL DEFAULT 'remote',
                        base_url TEXT NOT NULL,
                        api_key TEXT NOT NULL,
                        model TEXT NOT NULL,
                        updated_at TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await
                .map_err(|e| format!("create provider_config table: {e}"))?;
                sqlx::query(
                    "ALTER TABLE provider_config
                     ADD COLUMN IF NOT EXISTS backend TEXT NOT NULL DEFAULT 'remote'",
                )
                .execute(pool)
                .await
                .map_err(|e| format!("add provider_config.backend: {e}"))?;
            }
        }
        Ok(())
    }

    /// Migrate a pre-existing libsql `provider_config` table to add the
    /// `backend` column (SQLite has no `ADD COLUMN IF NOT EXISTS`, so check
    /// `PRAGMA table_info` first). The LLM row always stays `remote`; the
    /// column is only meaningful for the embedder row.
    async fn ensure_provider_config_backend_libsql(
        &self,
        conn: &libsql::Connection,
    ) -> Result<(), String> {
        let mut cols = conn
            .query("PRAGMA table_info(provider_config)", ())
            .await
            .map_err(|e| format!("inspect provider_config columns: {e}"))?;
        while let Some(row) = cols
            .next()
            .await
            .map_err(|e| format!("read columns: {e}"))?
        {
            let name: String = row.get(1).map_err(|e| format!("column name: {e}"))?;
            if name == "backend" {
                return Ok(());
            }
        }
        conn.execute(
            "ALTER TABLE provider_config ADD COLUMN backend TEXT NOT NULL DEFAULT 'remote'",
            (),
        )
        .await
        .map_err(|e| format!("add provider_config.backend: {e}"))?;
        Ok(())
    }

    /// Single-row key/value table for runtime-tunable behavior settings.
    ///
    /// Stores fields that are DB-authoritative after first boot but are seeded
    /// from Category B config on a fresh install — currently just
    /// `extraction_mode`.
    async fn init_runtime_settings(&self) -> Result<(), String> {
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS runtime_settings (
                        key TEXT PRIMARY KEY,
                        value TEXT NOT NULL
                    )",
                    (),
                )
                .await
                .map_err(|e| format!("create runtime_settings table: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS runtime_settings (
                        key TEXT PRIMARY KEY,
                        value TEXT NOT NULL
                    )",
                )
                .execute(pool)
                .await
                .map_err(|e| format!("create runtime_settings table: {e}"))?;
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
