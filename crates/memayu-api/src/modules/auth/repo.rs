use crate::infrastructure::db::DbClient;
use crate::modules::auth::model::User;

impl DbClient {
    // ── Users ──

    pub async fn users_empty(&self) -> Result<bool, String> {
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query("SELECT COUNT(*) FROM users", ())
                    .await
                    .map_err(|e| format!("count users: {e}"))?;
                let row = rows
                    .next()
                    .await
                    .map_err(|e| format!("read count: {e}"))?
                    .ok_or_else(|| "no users row".to_string())?;
                let count: i64 = row.get(0).map_err(|e| format!("read count value: {e}"))?;
                Ok(count == 0)
            }
            DbClient::Postgres(pool) => {
                let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
                    .fetch_one(pool)
                    .await
                    .map_err(|e| format!("count users: {e}"))?;
                Ok(count == 0)
            }
        }
    }

    pub async fn create_user(
        &self,
        email: &str,
        password_hash: &str,
        salt: &str,
    ) -> Result<(), String> {
        let id = uuid::Uuid::new_v4().to_string();
        let created = chrono::Utc::now().to_rfc3339();
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "INSERT INTO users (id, email, password, salt, is_admin, created_at)
                     VALUES (?1, ?2, ?3, ?4, 1, ?5)",
                    (id.as_str(), email, password_hash, salt, created.as_str()),
                )
                .await
                .map_err(|e| format!("create user: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO users (id, email, password, salt, is_admin, created_at)
                     VALUES ($1, $2, $3, $4, 1, $5)",
                )
                .bind(id)
                .bind(email)
                .bind(password_hash)
                .bind(salt)
                .bind(created)
                .execute(pool)
                .await
                .map_err(|e| format!("create user: {e}"))?;
            }
        }
        Ok(())
    }

    pub async fn find_user(&self, email: &str) -> Result<Option<User>, String> {
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query(
                        "SELECT id, email, password, salt FROM users WHERE email = ?1",
                        vec![email],
                    )
                    .await
                    .map_err(|e| format!("find user: {e}"))?;
                if let Some(row) = rows.next().await.map_err(|e| format!("read user: {e}"))? {
                    Ok(Some(User {
                        id: row.get(0).map_err(|e| format!("id: {e}"))?,
                        email: row.get(1).map_err(|e| format!("email: {e}"))?,
                        password: row.get(2).map_err(|e| format!("password: {e}"))?,
                        salt: row.get(3).map_err(|e| format!("salt: {e}"))?,
                    }))
                } else {
                    Ok(None)
                }
            }
            DbClient::Postgres(pool) => {
                let row = sqlx::query_as::<_, (String, String, String, String)>(
                    "SELECT id, email, password, salt FROM users WHERE email = $1",
                )
                .bind(email)
                .fetch_optional(pool)
                .await
                .map_err(|e| format!("find user: {e}"))?;
                Ok(row.map(|(id, email, password, salt)| User {
                    id,
                    email,
                    password,
                    salt,
                }))
            }
        }
    }

    pub async fn find_email(&self, user_id: &str) -> Result<Option<String>, String> {
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query("SELECT email FROM users WHERE id = ?1", vec![user_id])
                    .await
                    .map_err(|e| format!("find email: {e}"))?;
                if let Some(row) = rows.next().await.map_err(|e| format!("read user: {e}"))? {
                    Ok(Some(row.get(0).map_err(|e| format!("email: {e}"))?))
                } else {
                    Ok(None)
                }
            }
            DbClient::Postgres(pool) => {
                let row: Option<(String,)> =
                    sqlx::query_as("SELECT email FROM users WHERE id = $1")
                        .bind(user_id)
                        .fetch_optional(pool)
                        .await
                        .map_err(|e| format!("find email: {e}"))?;
                Ok(row.map(|(e,)| e))
            }
        }
    }

    pub async fn find_user_by_id(&self, user_id: &str) -> Result<Option<User>, String> {
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query(
                        "SELECT id, email, password, salt FROM users WHERE id = ?1",
                        vec![user_id],
                    )
                    .await
                    .map_err(|e| format!("find user by id: {e}"))?;
                if let Some(row) = rows.next().await.map_err(|e| format!("read user: {e}"))? {
                    Ok(Some(User {
                        id: row.get(0).map_err(|e| format!("id: {e}"))?,
                        email: row.get(1).map_err(|e| format!("email: {e}"))?,
                        password: row.get(2).map_err(|e| format!("password: {e}"))?,
                        salt: row.get(3).map_err(|e| format!("salt: {e}"))?,
                    }))
                } else {
                    Ok(None)
                }
            }
            DbClient::Postgres(pool) => {
                let row = sqlx::query_as::<_, (String, String, String, String)>(
                    "SELECT id, email, password, salt FROM users WHERE id = $1",
                )
                .bind(user_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| format!("find user by id: {e}"))?;
                Ok(row.map(|(id, email, password, salt)| User {
                    id,
                    email,
                    password,
                    salt,
                }))
            }
        }
    }

    /// Update a user's password hash + salt. Returns `false` when the row does
    /// not exist (so the caller can decide whether that is an error).
    pub async fn update_password(
        &self,
        user_id: &str,
        password_hash: &str,
        salt: &str,
    ) -> Result<bool, String> {
        match self {
            DbClient::Libsql(conn) => {
                let res = conn
                    .execute(
                        "UPDATE users SET password = ?1, salt = ?2 WHERE id = ?3",
                        (password_hash, salt, user_id),
                    )
                    .await
                    .map_err(|e| format!("update password: {e}"))?;
                Ok(res > 0)
            }
            DbClient::Postgres(pool) => {
                let res = sqlx::query("UPDATE users SET password = $1, salt = $2 WHERE id = $3")
                    .bind(password_hash)
                    .bind(salt)
                    .bind(user_id)
                    .execute(pool)
                    .await
                    .map_err(|e| format!("update password: {e}"))?;
                Ok(res.rows_affected() > 0)
            }
        }
    }

    /// Update a user's email. Returns `false` when the row does not exist.
    pub async fn update_email(&self, user_id: &str, email: &str) -> Result<bool, String> {
        match self {
            DbClient::Libsql(conn) => {
                let res = conn
                    .execute(
                        "UPDATE users SET email = ?1 WHERE id = ?2",
                        (email, user_id),
                    )
                    .await
                    .map_err(|e| format!("update email: {e}"))?;
                Ok(res > 0)
            }
            DbClient::Postgres(pool) => {
                let res = sqlx::query("UPDATE users SET email = $1 WHERE id = $2")
                    .bind(email)
                    .bind(user_id)
                    .execute(pool)
                    .await
                    .map_err(|e| format!("update email: {e}"))?;
                Ok(res.rows_affected() > 0)
            }
        }
    }

    // ── Sessions ──

    pub async fn create_session(
        &self,
        token: &str,
        user_id: &str,
        expires_at: &str,
    ) -> Result<(), String> {
        let created = chrono::Utc::now().to_rfc3339();
        match self {
            DbClient::Libsql(conn) => {
                conn.execute(
                    "INSERT INTO sessions (id, user_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
                    (token, user_id, created.as_str(), expires_at),
                )
                .await
                .map_err(|e| format!("create session: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO sessions (id, user_id, created_at, expires_at) VALUES ($1, $2, $3, $4)",
                )
                .bind(token)
                .bind(user_id)
                .bind(created)
                .bind(expires_at)
                .execute(pool)
                .await
                .map_err(|e| format!("create session: {e}"))?;
            }
        }
        Ok(())
    }

    pub async fn find_session_user(&self, token: &str) -> Result<Option<String>, String> {
        let now = chrono::Utc::now().to_rfc3339();
        match self {
            DbClient::Libsql(conn) => {
                let mut rows = conn
                    .query(
                        "SELECT user_id FROM sessions WHERE id = ?1 AND expires_at > ?2",
                        vec![token, now.as_str()],
                    )
                    .await
                    .map_err(|e| format!("find session: {e}"))?;
                if let Some(row) = rows
                    .next()
                    .await
                    .map_err(|e| format!("read session: {e}"))?
                {
                    Ok(Some(row.get(0).map_err(|e| format!("user_id: {e}"))?))
                } else {
                    Ok(None)
                }
            }
            DbClient::Postgres(pool) => {
                let row: Option<(String,)> = sqlx::query_as(
                    "SELECT user_id FROM sessions WHERE id = $1 AND expires_at > $2",
                )
                .bind(token)
                .bind(&now)
                .fetch_optional(pool)
                .await
                .map_err(|e| format!("find session: {e}"))?;
                Ok(row.map(|(id,)| id))
            }
        }
    }

    pub async fn delete_session(&self, token: &str) -> Result<(), String> {
        match self {
            DbClient::Libsql(conn) => {
                conn.execute("DELETE FROM sessions WHERE id = ?1", vec![token])
                    .await
                    .map_err(|e| format!("delete session: {e}"))?;
            }
            DbClient::Postgres(pool) => {
                sqlx::query("DELETE FROM sessions WHERE id = $1")
                    .bind(token)
                    .execute(pool)
                    .await
                    .map_err(|e| format!("delete session: {e}"))?;
            }
        }
        Ok(())
    }
}
