use axum::extract::{FromRef, FromRequestParts};
use axum::http::header::COOKIE;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Redirect, Response};
use memayu_api::WebServices;

/// Extracted for authenticated dashboard routes.
/// Redirects to /setup or /login if no valid session is found.
#[derive(Clone)]
pub struct CurrentUser {
    pub id: String,
    pub email: String,
}

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
    WebServices: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let services = WebServices::from_ref(state);
        let token = extract_session_token(parts);
        if let Some(token) = token {
            if let Ok((user_id, email)) = services.auth_resolve_session_with_email(&token).await {
                return Ok(CurrentUser { id: user_id, email });
            }
        }
        let target = if services.auth_users_empty().await.unwrap_or(false) {
            "/setup"
        } else {
            "/login"
        };
        Err(Redirect::to(target).into_response())
    }
}

fn extract_session_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(|c| {
            c.split(';').find_map(|p| {
                p.trim()
                    .strip_prefix("memayu_session=")
                    .map(|s| s.to_string())
            })
        })
}
