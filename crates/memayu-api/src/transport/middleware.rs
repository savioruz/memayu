use crate::infrastructure::db::DbClient;
use crate::modules::auth::service as auth_service;
use axum::extract::{FromRef, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{extract::Request, middleware::Next};
use std::sync::Arc;
use std::time::Instant;

/// The authenticated account ID, injected by auth middleware.
#[derive(Clone, Debug)]
pub struct AccountId(pub String);

/// AppState for the API router.
#[derive(Clone)]
pub struct ApiState {
    pub db: DbClient,
    pub service: Arc<memayu_core::MemoryService>,
    pub provider_configs: crate::modules::providers::service::ConfigRegistry,
}

impl FromRef<ApiState> for DbClient {
    fn from_ref(s: &ApiState) -> Self {
        s.db.clone()
    }
}
impl FromRef<ApiState> for Arc<memayu_core::MemoryService> {
    fn from_ref(s: &ApiState) -> Self {
        s.service.clone()
    }
}
impl FromRef<ApiState> for crate::modules::providers::service::ConfigRegistry {
    fn from_ref(s: &ApiState) -> Self {
        s.provider_configs.clone()
    }
}

/// Axum extractor for AccountId from request extensions.
impl<S> FromRequestParts<S> for AccountId
where
    DbClient: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, axum::Json<crate::error::ApiErrorBody>);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<AccountId>().cloned().ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                axum::Json(crate::error::ApiErrorBody {
                    error: "unauthorized".into(),
                    message: "authentication required".into(),
                }),
            )
        })
    }
}

/// Unified auth middleware: resolves session cookie OR X-API-Key header
/// and injects AccountId into request extensions.
pub async fn auth_middleware(
    State(db): State<DbClient>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let account_id =
        if let Some(api_key) = req.headers().get("x-api-key").and_then(|v| v.to_str().ok()) {
            crate::modules::api_keys::service::resolve(api_key, &db)
                .await
                .map_err(|_| StatusCode::UNAUTHORIZED)?
        } else {
            // Try session cookie
            let token = req
                .headers()
                .get(axum::http::header::COOKIE)
                .and_then(|c| c.to_str().ok())
                .and_then(auth_service::extract_session_token_from_cookie);
            match token {
                Some(t) => auth_service::resolve_session(&db, &t)
                    .await
                    .map_err(|_| StatusCode::UNAUTHORIZED)?,
                None => return Err(StatusCode::UNAUTHORIZED),
            }
        };

    req.extensions_mut().insert(AccountId(account_id));
    Ok(next.run(req).await)
}

/// Middleware for /docs: redirects unauthenticated users to /login
/// instead of returning a raw 401. API key or session cookie both work.
pub async fn docs_auth_redirect(State(db): State<DbClient>, req: Request, next: Next) -> Response {
    // Try API key first
    if let Some(api_key) = req.headers().get("x-api-key").and_then(|v| v.to_str().ok()) {
        if crate::modules::api_keys::service::resolve(api_key, &db)
            .await
            .is_ok()
        {
            return next.run(req).await;
        }
    }

    // Try session cookie
    let session_found = req
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(auth_service::extract_session_token_from_cookie);

    if let Some(token) = session_found {
        if auth_service::resolve_session(&db, &token).await.is_ok() {
            return next.run(req).await;
        }
    }

    // No valid auth — redirect to login
    Redirect::to("/login").into_response()
}

pub async fn api_request_logger(State(db): State<DbClient>, req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let auth = if req.headers().contains_key("x-api-key") {
        "API Key"
    } else {
        "Session"
    };

    let start = Instant::now();
    let resp = next.run(req).await;
    let status = resp.status().as_u16();
    let latency = start.elapsed().as_secs_f64() * 1000.0;

    let _ = db
        .insert_request_log(&method, &path, status, latency, auth)
        .await;

    resp
}
