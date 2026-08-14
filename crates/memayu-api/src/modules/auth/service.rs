use crate::error::ApiError;
use crate::infrastructure::db::DbClient;
use crate::modules::auth::dto::{AuthResponse, LoginRequest, SetupRequest};
use chrono::Duration;
use rand::Rng;

// Password rules, hashing, and salt generation live in `memayu-identity` so the
// terminal first-run setup and the web `POST /api/auth/setup` flow share the
// exact same account-creation logic (#32).
pub use memayu_identity::{hash_password, new_salt, validate_password};

pub const SESSION_COOKIE: &str = "memayu_session";
/// Session lifetime in seconds (1 day).
pub const SESSION_DURATION_SECS: i64 = 24 * 60 * 60;

/// Returns an RFC 3339 timestamp `duration_secs` from now.
pub fn expires_at_rfc3339(duration_secs: i64) -> String {
    (chrono::Utc::now() + Duration::seconds(duration_secs)).to_rfc3339()
}

// ── token helpers ──

pub fn new_token() -> String {
    let mut rng = rand::rngs::OsRng;
    hex::encode(rng.gen::<[u8; 32]>())
}

/// Transport-agnostic: extract session token from a raw Cookie header value.
/// Use the axum wrapper in transport/ for HeaderMap extraction.
pub fn extract_session_token_from_cookie(cookie_header: &str) -> Option<String> {
    cookie_header
        .split(';')
        .find_map(|p| p.trim().strip_prefix("memayu_session="))
        .map(|s| s.to_string())
}

// ── public query helpers ──

/// Check whether the users table is empty (fresh install).
pub async fn users_empty(db: &DbClient) -> Result<bool, String> {
    db.users_empty().await
}

/// Resolve a session token to a user_id.
pub async fn resolve_session(db: &DbClient, token: &str) -> Result<String, String> {
    db.find_session_user(token)
        .await?
        .ok_or_else(|| String::from("invalid session"))
}

/// Resolve a session token to a user_id and email.
pub async fn resolve_session_with_email(
    db: &DbClient,
    token: &str,
) -> Result<(String, String), String> {
    let user_id = db
        .find_session_user(token)
        .await?
        .ok_or_else(|| String::from("invalid session"))?;
    let email = db
        .find_email(&user_id)
        .await?
        .ok_or_else(|| String::from("invalid session"))?;
    Ok((user_id, email))
}

// ── password validation ──
//
// `validate_password` is re-exported from `memayu-identity` at the top of this
// module (see above).

// ── Business-logic functions (transport agnostic) ──

/// Create the admin account on first run. Returns auth response + session token
/// so the transport layer can set the cookie.
pub async fn setup(db: &DbClient, req: &SetupRequest) -> Result<(AuthResponse, String), ApiError> {
    if !db.users_empty().await.map_err(|e| ApiError {
        status: 500,
        error: "internal_error".into(),
        message: e,
    })? {
        return Err(ApiError {
            status: 409,
            error: "conflict".into(),
            message: "Setup already completed".into(),
        });
    }
    if req.email.trim().is_empty() {
        return Err(ApiError::bad_request("email is required"));
    }
    if let Some(err) = validate_password(&req.password) {
        return Err(ApiError::bad_request(err));
    }
    if req.password != req.confirm {
        return Err(ApiError::bad_request("passwords do not match"));
    }
    let salt = new_salt();
    let hash = hash_password(&salt, &req.password);
    db.create_user(req.email.trim(), &hash, &salt)
        .await
        .map_err(|e| ApiError {
            status: 500,
            error: "internal_error".into(),
            message: e,
        })?;
    let user = db
        .find_user(req.email.trim())
        .await
        .map_err(|e| ApiError {
            status: 500,
            error: "internal_error".into(),
            message: e,
        })?
        .ok_or_else(|| ApiError {
            status: 500,
            error: "internal_error".into(),
            message: "user not found after creation".into(),
        })?;
    let token = new_token();
    let expires_at = expires_at_rfc3339(SESSION_DURATION_SECS);
    db.create_session(&token, &user.id, &expires_at)
        .await
        .map_err(|e| ApiError {
            status: 500,
            error: "internal_error".into(),
            message: e,
        })?;
    Ok((
        AuthResponse {
            status: "ok".into(),
            message: "Admin account created".into(),
        },
        token,
    ))
}

/// Authenticate a user by email + password. Returns auth response + session token.
pub async fn login(db: &DbClient, req: &LoginRequest) -> Result<(AuthResponse, String), ApiError> {
    let user = db
        .find_user(req.email.trim())
        .await
        .map_err(|e| ApiError {
            status: 500,
            error: "internal_error".into(),
            message: e,
        })?
        .ok_or_else(|| ApiError {
            status: 401,
            error: "unauthorized".into(),
            message: "invalid credentials".into(),
        })?;
    if hash_password(&user.salt, &req.password) != user.password {
        return Err(ApiError {
            status: 401,
            error: "unauthorized".into(),
            message: "invalid credentials".into(),
        });
    }
    let token = new_token();
    let expires_at = expires_at_rfc3339(SESSION_DURATION_SECS);
    db.create_session(&token, &user.id, &expires_at)
        .await
        .map_err(|e| ApiError {
            status: 500,
            error: "internal_error".into(),
            message: e,
        })?;
    Ok((
        AuthResponse {
            status: "ok".into(),
            message: "Logged in".into(),
        },
        token,
    ))
}

/// Invalidate the session identified by `token` (if any).
pub async fn logout(db: &DbClient, token: Option<&str>) -> Result<AuthResponse, ApiError> {
    if let Some(t) = token {
        let _ = db.delete_session(t).await;
    }
    Ok(AuthResponse {
        status: "ok".into(),
        message: "Logged out".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_password ──

    #[test]
    fn validate_password_accepts_valid() {
        assert!(validate_password("Str0ng!Pass").is_none());
    }

    #[test]
    fn validate_password_rejects_too_short() {
        assert_eq!(
            validate_password("Ab1"),
            Some("Password must be at least 8 characters.")
        );
    }

    #[test]
    fn validate_password_rejects_no_uppercase() {
        assert_eq!(
            validate_password("abcdefg1"),
            Some("Password must contain at least one uppercase letter.")
        );
    }

    #[test]
    fn validate_password_rejects_no_lowercase() {
        assert_eq!(
            validate_password("ABCDEFG1"),
            Some("Password must contain at least one lowercase letter.")
        );
    }

    #[test]
    fn validate_password_rejects_no_digit() {
        assert_eq!(
            validate_password("Abcdefgh"),
            Some("Password must contain at least one digit.")
        );
    }

    // ── hash_password ──

    #[test]
    fn hash_password_deterministic() {
        let a = hash_password("salt", "pass");
        let b = hash_password("salt", "pass");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_password_different_salt_produces_different_hash() {
        let a = hash_password("saltA", "pass");
        let b = hash_password("saltB", "pass");
        assert_ne!(a, b);
    }

    #[test]
    fn hash_password_different_password_produces_different_hash() {
        let a = hash_password("salt", "passA");
        let b = hash_password("salt", "passB");
        assert_ne!(a, b);
    }

    // ── extract_session_token_from_cookie ──

    #[test]
    fn extract_session_token_single_cookie() {
        let t = extract_session_token_from_cookie("memayu_session=abc123");
        assert_eq!(t.as_deref(), Some("abc123"));
    }

    #[test]
    fn extract_session_token_among_others() {
        let t = extract_session_token_from_cookie("foo=bar; memayu_session=abc123; baz=qux");
        assert_eq!(t.as_deref(), Some("abc123"));
    }

    #[test]
    fn extract_session_token_missing() {
        let t = extract_session_token_from_cookie("foo=bar");
        assert!(t.is_none());
    }

    #[test]
    fn extract_session_token_empty() {
        let t = extract_session_token_from_cookie("");
        assert!(t.is_none());
    }

    // ── new_salt / new_token are non-empty ──

    #[test]
    fn new_salt_is_non_empty() {
        assert!(!new_salt().is_empty());
    }

    #[test]
    fn new_token_is_non_empty() {
        assert!(!new_token().is_empty());
    }

    // ── expires_at_rfc3339 produces valid timestamps ──

    #[test]
    fn expires_at_rfc3339_is_valid_iso() {
        let ts = expires_at_rfc3339(3600);
        // Should parse back without error (chrono format test)
        assert!(chrono::DateTime::parse_from_rfc3339(&ts).is_ok());
    }
}
