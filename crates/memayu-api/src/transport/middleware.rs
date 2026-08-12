use crate::infrastructure::db::DbClient;
use crate::modules::auth::service as auth_service;
use crate::transport::rate_limiter::RateLimiter;
use axum::extract::{FromRef, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{extract::Request, middleware::Next};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct AccountId(pub String);

#[derive(Clone)]
pub struct ApiState {
    pub db: DbClient,
    pub service: Arc<memayu_core::MemoryService>,
    pub provider_configs: crate::modules::providers::service::ConfigRegistry,
    pub ip_rate_limiter: RateLimiter,
    pub api_key_rate_limiter: RateLimiter,
}

impl ApiState {
    pub fn new(
        db: DbClient,
        service: Arc<memayu_core::MemoryService>,
        provider_configs: crate::modules::providers::service::ConfigRegistry,
    ) -> Self {
        use crate::transport::rate_limiter::RateLimitConfig;
        Self {
            db,
            service,
            provider_configs,
            ip_rate_limiter: RateLimiter::new(RateLimitConfig::per_ip_auth()),
            api_key_rate_limiter: RateLimiter::new(RateLimitConfig::per_api_key()),
        }
    }
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

pub async fn api_rate_limiter(State(state): State<ApiState>, req: Request, next: Next) -> Response {
    let key = req
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|k| format!("apikey:{k}"))
        .or_else(|| {
            req.headers()
                .get(axum::http::header::COOKIE)
                .and_then(|c| c.to_str().ok())
                .and_then(auth_service::extract_session_token_from_cookie)
                .map(|t| format!("session:{t}"))
        })
        .unwrap_or_else(|| {
            req.headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown")
                .to_string()
        });

    match state.api_key_rate_limiter.check(&key).await {
        Ok(()) => next.run(req).await,
        Err(retry_secs) => (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(crate::error::ApiErrorBody {
                error: "rate_limited".into(),
                message: format!("too many requests, retry after {retry_secs}s"),
            }),
        )
            .into_response(),
    }
}

pub async fn auth_rate_limiter(
    State(state): State<ApiState>,
    req: Request,
    next: Next,
) -> Response {
    let ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    match state.ip_rate_limiter.check(ip).await {
        Ok(()) => next.run(req).await,
        Err(retry_secs) => (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(crate::error::ApiErrorBody {
                error: "rate_limited".into(),
                message: format!("too many requests, retry after {retry_secs}s"),
            }),
        )
            .into_response(),
    }
}

fn behind_tls() -> bool {
    std::env::var("MEMAYU_BEHIND_TLS")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(false)
}

pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;

    let headers = resp.headers_mut();
    headers.insert(
        axum::http::header::HeaderName::from_static("x-content-type-options"),
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("x-frame-options"),
        axum::http::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        axum::http::header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("content-security-policy"),
        axum::http::HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
        ),
    );
    if behind_tls() {
        headers.insert(
            axum::http::header::HeaderName::from_static("strict-transport-security"),
            axum::http::HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }

    resp
}

pub async fn cors_middleware(req: Request, next: Next) -> Response {
    if req.method() == axum::http::Method::OPTIONS {
        let origin = req
            .headers()
            .get(axum::http::header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let allowed_origins = std::env::var("MEMAYU_CORS_ORIGINS").unwrap_or_default();
        let allowed: Vec<&str> = allowed_origins
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        let allow_origin = if allowed.is_empty() {
            None
        } else if allowed.contains(&origin) {
            Some(origin.to_string())
        } else {
            None
        };

        let mut resp = Response::new(axum::body::Body::default());
        *resp.status_mut() = StatusCode::NO_CONTENT;
        if let Some(o) = allow_origin {
            resp.headers_mut().insert(
                axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
                axum::http::HeaderValue::from_str(&o).unwrap(),
            );
            resp.headers_mut().insert(
                axum::http::header::ACCESS_CONTROL_ALLOW_METHODS,
                axum::http::HeaderValue::from_static("GET, POST, PATCH, DELETE, OPTIONS"),
            );
            resp.headers_mut().insert(
                axum::http::header::ACCESS_CONTROL_ALLOW_HEADERS,
                axum::http::HeaderValue::from_static("content-type, x-api-key"),
            );
            resp.headers_mut().insert(
                axum::http::header::ACCESS_CONTROL_MAX_AGE,
                axum::http::HeaderValue::from_static("86400"),
            );
        }
        return resp;
    }

    next.run(req).await
}

pub async fn request_id(req: Request, next: Next) -> Response {
    let id = uuid::Uuid::new_v4().to_string();
    let mut resp = next.run(req).await;
    resp.headers_mut().insert(
        axum::http::HeaderName::from_static("x-request-id"),
        axum::http::HeaderValue::from_str(&id).unwrap(),
    );
    resp
}
