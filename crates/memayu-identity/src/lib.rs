//! Self-hosted single-admin identity resolution.
//!
//! Self-hosted memayu is a single-admin model: exactly one `users` row backs
//! the whole instance. The Web frontend resolves that row through its auth
//! session middleware, but in-process frontends (TUI, local MCP) talk to
//! [`MemoryService`](memayu_core::MemoryService) directly and previously
//! hardcoded `"default"` as the `user_id` — silently splitting one instance
//! into two disjoint memory stores (#32).
//!
//! This crate provides the shared bootstrap helpers both paths use so every
//! frontend resolves to the identical admin account_id:
//!
//! - [`resolve_self_hosted_account_id`] returns the real admin UUID from
//!   `users`, erroring with [`IdentityError::NoAdminAccount`] when no account
//!   exists yet (i.e. first-run before `/api/auth/setup` has completed).
//! - [`create_admin_account`] creates that first admin account in-process, so
//!   terminal frontends can complete first-run setup without going through the
//!   HTTP server. It shares the same password rules and hashing as the web
//!   `POST /api/auth/setup` flow.
//! - [`backfill_placeholder_memories`] idempotently re-assigns legacy
//!   placeholder rows (e.g. `"default"`) to the admin account.
//! - [`bootstrap`] combines the two: resolve the admin and backfill in one
//!   call, which is what the TUI and local MCP frontends run at startup.
//! - [`generate_api_key`] creates a `mmyu_…` API key (storing only its hash),
//!   the shared generator used by both the terminal setup wizard and the web
//!   `/api/keys` flow.

use memayu_config::{StorageBackend, StorageConfig};
use rand::Rng;
use sha2::Digest;
use thiserror::Error;

/// Errors produced by identity resolution and admin account creation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error(
        "no admin account found for this self-hosted instance; complete first-run setup \
         first (start `memayu serve` and open /api/auth/setup)"
    )]
    NoAdminAccount,
    #[error("setup already completed; an admin account already exists")]
    SetupAlreadyCompleted,
    #[error("{0}")]
    Validation(&'static str),
    #[error("database error: {0}")]
    Db(String),
}

/// Legacy `user_id` values written by in-process frontends before #32 was
/// fixed. Every one of these is reassigned to the admin account by
/// [`backfill_placeholder_memories`].
const PLACEHOLDER_USER_IDS: &[&str] = &["default", ""];

/// A literal SQL list (`'a', 'b'`) built from [`PLACEHOLDER_USER_IDS`].
///
/// The values are an internal constant (never user input), so this is safe to
/// splice into a query string.
fn placeholder_list() -> String {
    PLACEHOLDER_USER_IDS
        .iter()
        .map(|p| format!("'{p}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve the admin account_id for a self-hosted instance.
///
/// Returns [`IdentityError::NoAdminAccount`] when the `users` table does not
/// exist or contains no rows, which on a fresh instance means first-run setup
/// has not been completed yet.
pub async fn resolve_self_hosted_account_id(
    config: &StorageConfig,
) -> Result<String, IdentityError> {
    match config.backend {
        StorageBackend::Libsql => resolve_libsql(config).await,
        StorageBackend::Postgres => resolve_postgres(config).await,
    }
}

async fn resolve_libsql(config: &StorageConfig) -> Result<String, IdentityError> {
    let conn = open_libsql(config).await?;
    if !libsql_table_exists(&conn, "users").await? {
        return Err(IdentityError::NoAdminAccount);
    }
    let mut rows = conn
        .query("SELECT id FROM users ORDER BY created_at ASC LIMIT 1", ())
        .await
        .map_err(|e| IdentityError::Db(format!("resolve admin: {e}")))?;
    match rows
        .next()
        .await
        .map_err(|e| IdentityError::Db(format!("resolve admin: {e}")))?
    {
        Some(row) => row
            .get::<String>(0)
            .map_err(|e| IdentityError::Db(format!("read admin id: {e}"))),
        None => Err(IdentityError::NoAdminAccount),
    }
}

async fn resolve_postgres(config: &StorageConfig) -> Result<String, IdentityError> {
    let pool = open_postgres(config).await?;
    if !postgres_table_exists(&pool, "users").await? {
        return Err(IdentityError::NoAdminAccount);
    }
    let row: Option<(String,)> =
        sqlx::query_as("SELECT id FROM users ORDER BY created_at ASC LIMIT 1")
            .fetch_optional(&pool)
            .await
            .map_err(|e| IdentityError::Db(format!("resolve admin: {e}")))?;
    row.map(|(id,)| id).ok_or(IdentityError::NoAdminAccount)
}

/// Re-assign every memory row still under a legacy placeholder `user_id` to
/// the given admin account. Returns the number of rows migrated.
///
/// This is idempotent (safe to run on every startup). It is a no-op returning
/// `0` when the `memories` table does not exist yet.
pub async fn backfill_placeholder_memories(
    config: &StorageConfig,
    admin_id: &str,
) -> Result<u64, IdentityError> {
    match config.backend {
        StorageBackend::Libsql => backfill_libsql(config, admin_id).await,
        StorageBackend::Postgres => backfill_postgres(config, admin_id).await,
    }
}

async fn backfill_libsql(config: &StorageConfig, admin_id: &str) -> Result<u64, IdentityError> {
    let conn = open_libsql(config).await?;
    let Some(count) = libsql_count_placeholders(&conn).await? else {
        return Ok(0);
    };
    if count == 0 {
        return Ok(0);
    }
    let list = placeholder_list();
    conn.execute(
        &format!("UPDATE memories SET user_id = ?1 WHERE user_id IN ({list})"),
        vec![admin_id],
    )
    .await
    .map_err(|e| IdentityError::Db(format!("backfill memories: {e}")))?;
    // Keep the FTS mirror's stored user_id consistent. It is UNINDEXED and
    // never used for filtering, so a missing table (pre-FTS DB) is fine.
    if libsql_table_exists(&conn, "memories_fts").await? {
        let _ = conn
            .execute(
                &format!("UPDATE memories_fts SET user_id = ?1 WHERE user_id IN ({list})"),
                vec![admin_id],
            )
            .await;
    }
    Ok(count as u64)
}

async fn backfill_postgres(config: &StorageConfig, admin_id: &str) -> Result<u64, IdentityError> {
    let pool = open_postgres(config).await?;
    if !postgres_table_exists(&pool, "memories").await? {
        return Ok(0);
    }
    let list = placeholder_list();
    let result = sqlx::query(&format!(
        "UPDATE memories SET user_id = $1 WHERE user_id IN ({list})"
    ))
    .bind(admin_id)
    .execute(&pool)
    .await
    .map_err(|e| IdentityError::Db(format!("backfill memories: {e}")))?;
    Ok(result.rows_affected())
}

/// Resolve the admin account_id and backfill legacy placeholders in one call.
///
/// Used by in-process frontends (TUI, local MCP) that must have an account.
pub async fn bootstrap(config: &StorageConfig) -> Result<String, IdentityError> {
    let admin_id = resolve_self_hosted_account_id(config).await?;
    backfill_placeholder_memories(config, &admin_id).await?;
    Ok(admin_id)
}

/// Reset the admin password for a self-hosted instance, bypassing the login
/// gate entirely (the recovery path for a forgotten password). Validates the
/// new password against the shared policy and writes a fresh salt + hash.
pub async fn reset_password(config: &StorageConfig, password: &str) -> Result<(), IdentityError> {
    if let Some(msg) = validate_password(password) {
        return Err(IdentityError::Validation(msg));
    }
    let admin_id = resolve_self_hosted_account_id(config).await?;
    let salt = new_salt();
    let hash = hash_password(&salt, password);
    match config.backend {
        StorageBackend::Libsql => {
            let conn = open_libsql(config).await?;
            conn.execute(
                "UPDATE users SET password = ?1, salt = ?2 WHERE id = ?3",
                (hash.as_str(), salt.as_str(), admin_id.as_str()),
            )
            .await
            .map_err(|e| IdentityError::Db(format!("reset password: {e}")))?;
        }
        StorageBackend::Postgres => {
            let pool = open_postgres(config).await?;
            sqlx::query("UPDATE users SET password = $1, salt = $2 WHERE id = $3")
                .bind(hash.as_str())
                .bind(salt.as_str())
                .bind(admin_id.as_str())
                .execute(&pool)
                .await
                .map_err(|e| IdentityError::Db(format!("reset password: {e}")))?;
        }
    }
    Ok(())
}

// ── password rules / hashing ──
//
// These mirror the rules enforced by the web `POST /api/auth/setup` flow so
// an admin created from the terminal is byte-for-byte equivalent to one
// created over HTTP. `memayu-api` re-exports these instead of keeping its own
// copies.

/// Validate a password against the shared policy. Returns `None` when valid.
pub fn validate_password(password: &str) -> Option<&'static str> {
    if password.len() < 8 {
        return Some("Password must be at least 8 characters.");
    }
    if !password.chars().any(|c| c.is_uppercase()) {
        return Some("Password must contain at least one uppercase letter.");
    }
    if !password.chars().any(|c| c.is_lowercase()) {
        return Some("Password must contain at least one lowercase letter.");
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        return Some("Password must contain at least one digit.");
    }
    None
}

/// Hash a password with a per-account salt (SHA-256 of `salt || password`).
pub fn hash_password(salt: &str, password: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize())
}

/// Generate a fresh random salt (32 hex chars).
pub fn new_salt() -> String {
    use rand::Rng;
    hex::encode(rand::rngs::OsRng.gen::<[u8; 16]>())
}

/// Create the instance's first admin account in-process.
///
/// Validates the email/password (same rules as the web setup flow), ensures
/// the `users` table exists, refuses to run if an admin already exists, and
/// inserts a new admin row. Returns the new admin account_id.
///
/// This is what terminal frontends use to complete first-run setup without
/// shelling out to `memayu serve` + the browser. The Web frontend reaches the
/// same account via `POST /api/auth/setup`.
pub async fn create_admin_account(
    config: &StorageConfig,
    email: &str,
    password: &str,
    confirm: &str,
) -> Result<String, IdentityError> {
    let email = email.trim();
    if email.is_empty() {
        return Err(IdentityError::Validation("email is required"));
    }
    if let Some(msg) = validate_password(password) {
        return Err(IdentityError::Validation(msg));
    }
    if password != confirm {
        return Err(IdentityError::Validation("passwords do not match"));
    }
    match config.backend {
        StorageBackend::Libsql => create_admin_libsql(config, email, password).await,
        StorageBackend::Postgres => create_admin_postgres(config, email, password).await,
    }
}

async fn create_admin_libsql(
    config: &StorageConfig,
    email: &str,
    password: &str,
) -> Result<String, IdentityError> {
    let conn = open_libsql(config).await?;
    create_users_table_libsql(&conn).await?;
    if !libsql_users_empty(&conn).await? {
        return Err(IdentityError::SetupAlreadyCompleted);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let salt = new_salt();
    let hash = hash_password(&salt, password);
    let created = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO users (id, email, password, salt, is_admin, created_at)
         VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        (
            id.as_str(),
            email,
            hash.as_str(),
            salt.as_str(),
            created.as_str(),
        ),
    )
    .await
    .map_err(|e| IdentityError::Db(format!("create admin: {e}")))?;
    Ok(id)
}

async fn create_admin_postgres(
    config: &StorageConfig,
    email: &str,
    password: &str,
) -> Result<String, IdentityError> {
    let pool = open_postgres(config).await?;
    create_users_table_postgres(&pool).await?;
    if !postgres_users_empty(&pool).await? {
        return Err(IdentityError::SetupAlreadyCompleted);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let salt = new_salt();
    let hash = hash_password(&salt, password);
    let created = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, email, password, salt, is_admin, created_at)
         VALUES ($1, $2, $3, $4, 1, $5)",
    )
    .bind(id.as_str())
    .bind(email)
    .bind(hash.as_str())
    .bind(salt.as_str())
    .bind(created.as_str())
    .execute(&pool)
    .await
    .map_err(|e| IdentityError::Db(format!("create admin: {e}")))?;
    Ok(id)
}

// ── API keys ──

/// A freshly generated API key. Only the raw `key` is ever shown to the user;
/// the database stores only its hash plus a `key_prefix` for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedApiKey {
    pub key: String,
    pub id: String,
    pub key_prefix: String,
}

/// The material for a freshly generated API key, before any DB insert.
/// [`ApiKeyMaterial::key_hash`] is what gets stored; [`ApiKeyMaterial::key`]
/// (the full raw key) is shown to the user exactly once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyMaterial {
    pub key: String,
    pub key_prefix: String,
    pub key_hash: String,
}

fn sha256_hex(s: &str) -> String {
    let mut h = sha2::Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

/// Generate a new `mmyu_…` API key and its stored hash. Pure; no I/O.
///
/// This is the single source of truth for the key format (#54), shared by the
/// terminal setup wizard (via [`generate_api_key`]) and the web `/api/keys`
/// flow so the two never drift apart.
pub fn new_api_key() -> ApiKeyMaterial {
    let mut rng = rand::rngs::OsRng;
    let raw: [u8; 24] = rng.gen();
    let key = format!("mmyu_{}", hex::encode(raw));
    let key_prefix = format!("mmyu_{}", hex::encode(&raw[..2]));
    let key_hash = sha256_hex(&key);
    ApiKeyMaterial {
        key,
        key_prefix,
        key_hash,
    }
}

/// Generate a new `mmyu_…` API key owned by `user_id`, storing only its hash.
///
/// This is the shared, transport-agnostic key generator that both the terminal
/// setup wizard and the web `/api/keys` flow use, so there is one source of
/// truth for the key format (#54). The `api_keys` table is created on first use
/// (same DDL as `memayu-api`). The raw key is returned exactly once.
pub async fn generate_api_key(
    config: &StorageConfig,
    user_id: &str,
    label: &str,
) -> Result<GeneratedApiKey, IdentityError> {
    let label = label.trim();
    if label.is_empty() {
        return Err(IdentityError::Validation("label is required"));
    }
    let material = new_api_key();
    let id = uuid::Uuid::new_v4().to_string();
    let created = chrono::Utc::now().to_rfc3339();

    match config.backend {
        StorageBackend::Libsql => {
            let conn = open_libsql(config).await?;
            create_api_keys_table_libsql(&conn).await?;
            conn.execute(
                "INSERT INTO api_keys (id, user_id, label, key_prefix, key_hash, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    id.as_str(),
                    user_id,
                    label,
                    material.key_prefix.as_str(),
                    material.key_hash.as_str(),
                    created.as_str(),
                ),
            )
            .await
            .map_err(|e| IdentityError::Db(format!("insert api_key: {e}")))?;
        }
        StorageBackend::Postgres => {
            let pool = open_postgres(config).await?;
            create_api_keys_table_postgres(&pool).await?;
            sqlx::query(
                "INSERT INTO api_keys (id, user_id, label, key_prefix, key_hash, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(id.as_str())
            .bind(user_id)
            .bind(label)
            .bind(material.key_prefix.as_str())
            .bind(material.key_hash.as_str())
            .bind(created.as_str())
            .execute(&pool)
            .await
            .map_err(|e| IdentityError::Db(format!("insert api_key: {e}")))?;
        }
    }

    Ok(GeneratedApiKey {
        key: material.key,
        id,
        key_prefix: material.key_prefix,
    })
}

async fn create_api_keys_table_libsql(conn: &libsql::Connection) -> Result<(), IdentityError> {
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
    .map_err(|e| IdentityError::Db(format!("create api_keys table: {e}")))?;
    Ok(())
}

async fn create_api_keys_table_postgres(
    pool: &sqlx::Pool<sqlx::Postgres>,
) -> Result<(), IdentityError> {
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
    .map_err(|e| IdentityError::Db(format!("create api_keys table: {e}")))?;
    Ok(())
}

// ── users-table helpers ──

async fn create_users_table_libsql(conn: &libsql::Connection) -> Result<(), IdentityError> {
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
    .map_err(|e| IdentityError::Db(format!("create users table: {e}")))?;
    Ok(())
}

async fn libsql_users_empty(conn: &libsql::Connection) -> Result<bool, IdentityError> {
    let mut rows = conn
        .query("SELECT COUNT(*) FROM users", ())
        .await
        .map_err(|e| IdentityError::Db(format!("count users: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| IdentityError::Db(format!("read count: {e}")))?
        .ok_or_else(|| IdentityError::Db("no count row".into()))?;
    let count: i64 = row
        .get(0)
        .map_err(|e| IdentityError::Db(format!("read count value: {e}")))?;
    Ok(count == 0)
}

async fn create_users_table_postgres(
    pool: &sqlx::Pool<sqlx::Postgres>,
) -> Result<(), IdentityError> {
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
    .map_err(|e| IdentityError::Db(format!("create users table: {e}")))?;
    Ok(())
}

async fn postgres_users_empty(pool: &sqlx::Pool<sqlx::Postgres>) -> Result<bool, IdentityError> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
        .map_err(|e| IdentityError::Db(format!("count users: {e}")))?;
    Ok(count == 0)
}

// ── connection helpers ──

async fn open_libsql(config: &StorageConfig) -> Result<libsql::Connection, IdentityError> {
    let db = libsql::Builder::new_local(&config.libsql_path)
        .build()
        .await
        .map_err(|e| IdentityError::Db(format!("open libsql db: {e}")))?;
    db.connect()
        .map_err(|e| IdentityError::Db(format!("connect to libsql db: {e}")))
}

async fn libsql_table_exists(conn: &libsql::Connection, name: &str) -> Result<bool, IdentityError> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            vec![name],
        )
        .await
        .map_err(|e| IdentityError::Db(format!("check table {name}: {e}")))?;
    Ok(rows
        .next()
        .await
        .map_err(|e| IdentityError::Db(format!("check table {name}: {e}")))?
        .is_some())
}

async fn libsql_count_placeholders(
    conn: &libsql::Connection,
) -> Result<Option<i64>, IdentityError> {
    if !libsql_table_exists(conn, "memories").await? {
        return Ok(None);
    }
    let list = placeholder_list();
    let mut rows = conn
        .query(
            &format!("SELECT COUNT(*) FROM memories WHERE user_id IN ({list})"),
            (),
        )
        .await
        .map_err(|e| IdentityError::Db(format!("count placeholders: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| IdentityError::Db(format!("count placeholders: {e}")))?
        .ok_or_else(|| IdentityError::Db("no count row".into()))?;
    row.get::<i64>(0)
        .map(Some)
        .map_err(|e| IdentityError::Db(format!("read count: {e}")))
}

async fn open_postgres(
    config: &StorageConfig,
) -> Result<sqlx::Pool<sqlx::Postgres>, IdentityError> {
    let url = config
        .database_url
        .as_deref()
        .ok_or_else(|| IdentityError::Db("missing postgres database_url".into()))?;
    sqlx::PgPool::connect(url)
        .await
        .map_err(|e| IdentityError::Db(format!("connect to postgres: {e}")))
}

async fn postgres_table_exists(
    pool: &sqlx::Pool<sqlx::Postgres>,
    name: &str,
) -> Result<bool, IdentityError> {
    let found: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1)",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .map_err(|e| IdentityError::Db(format!("check table {name}: {e}")))?;
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a temp file path for a libsql DB (each connection must see the
    /// same on-disk file, so `:memory:` is not usable across connections).
    fn temp_db() -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("memayu-identity-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        dir.join(format!("db-{n}.db"))
    }

    fn unique_id() -> String {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("id-{n}")
    }

    fn config_for(path: &std::path::Path) -> StorageConfig {
        StorageConfig {
            backend: StorageBackend::Libsql,
            libsql_path: path.display().to_string(),
            database_url: None,
        }
    }

    /// Create the auth `users` table (same DDL as memayu-api) and seed one
    /// admin row. Returns the admin id.
    async fn seed_admin(conn: &libsql::Connection) -> String {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                salt TEXT NOT NULL,
                is_admin INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            )",
            (),
        )
        .await
        .unwrap();
        let admin_id = "a0000000-0000-4000-8000-000000000000".to_string();
        let created = "2024-01-01T00:00:00Z".to_string();
        conn.execute(
            "INSERT INTO users (id, email, password, salt, is_admin, created_at)
             VALUES (?1, 'admin@example.com', 'hash', 'salt', 1, ?2)",
            (admin_id.as_str(), created.as_str()),
        )
        .await
        .unwrap();
        admin_id
    }

    /// Create the storage `memories` + `memories_fts` schema (mirrors the
    /// storage provider) and insert a legacy placeholder row.
    async fn seed_memory(conn: &libsql::Connection, user_id: &str, content: &str) {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                content TEXT NOT NULL,
                embedding TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO memories (id, user_id, content, embedding, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, '[]', '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
            (
                unique_id(),
                user_id,
                content,
            ),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn resolve_returns_admin_id() {
        let path = temp_db();
        let conn = libsql::Builder::new_local(path.display().to_string())
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap();
        let admin = seed_admin(&conn).await;

        let resolved = resolve_self_hosted_account_id(&config_for(&path))
            .await
            .unwrap();
        assert_eq!(resolved, admin);
    }

    #[tokio::test]
    async fn resolve_errors_when_no_admin_account() {
        let path = temp_db();
        let conn = libsql::Builder::new_local(path.display().to_string())
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap();
        // users table exists (real schema) but is empty.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                salt TEXT NOT NULL,
                is_admin INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            )",
            (),
        )
        .await
        .unwrap();

        let err = resolve_self_hosted_account_id(&config_for(&path))
            .await
            .unwrap_err();
        assert_eq!(err, IdentityError::NoAdminAccount);

        // No users table at all (fresh before setup).
        let fresh = temp_db();
        let err2 = resolve_self_hosted_account_id(&config_for(&fresh))
            .await
            .unwrap_err();
        assert_eq!(err2, IdentityError::NoAdminAccount);
    }

    #[tokio::test]
    async fn backfill_reassigns_placeholders_idempotently() {
        let path = temp_db();
        let conn = libsql::Builder::new_local(path.display().to_string())
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap();
        let admin = seed_admin(&conn).await;
        seed_memory(&conn, "default", "legacy row").await;
        seed_memory(&conn, "", "legacy empty row").await;
        seed_memory(&conn, &admin, "already owned").await;

        let config = config_for(&path);
        let n = backfill_placeholder_memories(&config, &admin)
            .await
            .unwrap();
        assert_eq!(n, 2);

        // Idempotent: second run migrates nothing.
        let n2 = backfill_placeholder_memories(&config, &admin)
            .await
            .unwrap();
        assert_eq!(n2, 0);

        // Every row now belongs to the admin.
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM memories WHERE user_id = ?1",
                vec![admin.as_str()],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let count: i64 = row.get(0).unwrap();
        assert_eq!(count, 3);
    }

    // ── password rules / hashing ──

    #[test]
    fn validate_password_rules() {
        assert!(validate_password("Str0ng!Pass").is_none());
        assert_eq!(
            validate_password("Ab1"),
            Some("Password must be at least 8 characters.")
        );
        assert_eq!(
            validate_password("abcdefg1"),
            Some("Password must contain at least one uppercase letter.")
        );
        assert_eq!(
            validate_password("ABCDEFG1"),
            Some("Password must contain at least one lowercase letter.")
        );
        assert_eq!(
            validate_password("Abcdefgh"),
            Some("Password must contain at least one digit.")
        );
    }

    #[test]
    fn hash_password_is_deterministic_and_salt_sensitive() {
        assert_eq!(hash_password("salt", "pass"), hash_password("salt", "pass"));
        assert_ne!(
            hash_password("saltA", "pass"),
            hash_password("saltB", "pass")
        );
        assert_ne!(
            hash_password("salt", "passA"),
            hash_password("salt", "passB")
        );
    }

    #[test]
    fn new_salt_is_non_empty() {
        assert!(!new_salt().is_empty());
    }

    // ── create_admin_account ──

    #[tokio::test]
    async fn create_admin_account_is_resolvable() {
        let path = temp_db();
        let config = config_for(&path);

        let admin = create_admin_account(
            &config,
            "admin@example.com",
            "Correct-Horse-Battery-Staple-42!",
            "Correct-Horse-Battery-Staple-42!",
        )
        .await
        .unwrap();

        // The created account is what resolve finds as the admin.
        assert_eq!(
            resolve_self_hosted_account_id(&config).await.unwrap(),
            admin
        );

        // The row was stored with the correct hashed password + is_admin flag.
        let conn = libsql::Builder::new_local(path.display().to_string())
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap();
        let mut rows = conn
            .query(
                "SELECT password, salt, is_admin FROM users WHERE id = ?1",
                vec![admin.as_str()],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let stored_hash: String = row.get(0).unwrap();
        let salt: String = row.get(1).unwrap();
        let is_admin: i64 = row.get(2).unwrap();
        assert_eq!(
            stored_hash,
            hash_password(&salt, "Correct-Horse-Battery-Staple-42!")
        );
        assert_eq!(is_admin, 1);
    }

    #[tokio::test]
    async fn reset_password_updates_hash_and_validates() {
        let path = temp_db();
        let config = config_for(&path);
        let admin = create_admin_account(
            &config,
            "admin@example.com",
            "Correct-Horse-Battery-Staple-42!",
            "Correct-Horse-Battery-Staple-42!",
        )
        .await
        .unwrap();

        // Reset to a new password.
        reset_password(&config, "NewHorse-Staple-99!")
            .await
            .unwrap();

        // The stored row now verifies against the new password.
        let conn = libsql::Builder::new_local(path.display().to_string())
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap();
        let mut rows = conn
            .query(
                "SELECT password, salt FROM users WHERE id = ?1",
                vec![admin.as_str()],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let stored_hash: String = row.get(0).unwrap();
        let salt: String = row.get(1).unwrap();
        assert_eq!(stored_hash, hash_password(&salt, "NewHorse-Staple-99!"));
        assert_ne!(
            stored_hash,
            hash_password(&salt, "Correct-Horse-Battery-Staple-42!"),
            "old password must no longer verify"
        );

        // Weak password is rejected.
        let err = reset_password(&config, "weak").await.unwrap_err();
        assert_eq!(
            err,
            IdentityError::Validation("Password must be at least 8 characters.")
        );
    }

    #[tokio::test]
    async fn reset_password_errors_when_no_admin() {
        let config = config_for(&temp_db());
        let err = reset_password(&config, "NewHorse-Staple-99!")
            .await
            .unwrap_err();
        assert_eq!(err, IdentityError::NoAdminAccount);
    }

    #[tokio::test]
    async fn create_admin_account_rejects_weak_password() {
        let config = config_for(&temp_db());
        let err = create_admin_account(&config, "admin@example.com", "weak", "weak")
            .await
            .unwrap_err();
        assert_eq!(
            err,
            IdentityError::Validation("Password must be at least 8 characters.")
        );
    }

    #[tokio::test]
    async fn create_admin_account_rejects_mismatched_confirm() {
        let config = config_for(&temp_db());
        let err = create_admin_account(
            &config,
            "admin@example.com",
            "Correct-Horse-Battery-Staple-42!",
            "Different-Password-99!",
        )
        .await
        .unwrap_err();
        assert_eq!(err, IdentityError::Validation("passwords do not match"));
    }

    #[tokio::test]
    async fn create_admin_account_refuses_second_account() {
        let path = temp_db();
        let config = config_for(&path);
        create_admin_account(
            &config,
            "admin@example.com",
            "Correct-Horse-Battery-Staple-42!",
            "Correct-Horse-Battery-Staple-42!",
        )
        .await
        .unwrap();

        let err = create_admin_account(
            &config,
            "other@example.com",
            "Correct-Horse-Battery-Staple-42!",
            "Correct-Horse-Battery-Staple-42!",
        )
        .await
        .unwrap_err();
        assert_eq!(err, IdentityError::SetupAlreadyCompleted);
    }

    // ── generate_api_key ──

    #[tokio::test]
    async fn generate_api_key_returns_raw_key_and_stores_hash() {
        let path = temp_db();
        let config = config_for(&path);
        let admin = create_admin_account(
            &config,
            "admin@example.com",
            "Correct-Horse-Battery-Staple-42!",
            "Correct-Horse-Battery-Staple-42!",
        )
        .await
        .unwrap();

        let generated = generate_api_key(&config, &admin, "default").await.unwrap();

        // Key format + prefix, exactly like the web flow.
        assert!(generated.key.starts_with("mmyu_"));
        assert_eq!(generated.key_prefix, generated.key[..9].to_string());
        assert_eq!(generated.key.len(), 5 + 48); // "mmyu_" + 24 bytes hex

        // Only the hash is stored, not the raw key.
        let conn = libsql::Builder::new_local(path.display().to_string())
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap();
        let mut rows = conn
            .query(
                "SELECT user_id, label, key_hash, key_prefix FROM api_keys WHERE id = ?1",
                vec![generated.id.as_str()],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), admin);
        assert_eq!(row.get::<String>(1).unwrap(), "default");
        assert_eq!(row.get::<String>(2).unwrap(), sha256_hex(&generated.key));
        assert_eq!(row.get::<String>(3).unwrap(), generated.key_prefix);
    }

    #[tokio::test]
    async fn generate_api_key_rejects_empty_label() {
        let path = temp_db();
        let config = config_for(&path);
        let admin = create_admin_account(
            &config,
            "admin@example.com",
            "Correct-Horse-Battery-Staple-42!",
            "Correct-Horse-Battery-Staple-42!",
        )
        .await
        .unwrap();

        let err = generate_api_key(&config, &admin, "   ").await.unwrap_err();
        assert_eq!(err, IdentityError::Validation("label is required"));
    }
}
