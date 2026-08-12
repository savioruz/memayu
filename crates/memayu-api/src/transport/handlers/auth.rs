/// Axum handlers for auth routes — thin wrappers that extract from HTTP
/// and delegate to modules::auth::service.
use crate::modules::auth::dto::{AuthResponse, LoginRequest, SetupRequest};
use crate::modules::auth::service as auth_service;
use crate::transport::middleware::ApiState;
use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

fn session_cookie_header(token: &str) -> String {
    let secure = if std::env::var("MEMAYU_BEHIND_TLS")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(false)
    {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
        auth_service::SESSION_COOKIE,
        token,
        auth_service::SESSION_DURATION_SECS,
        secure,
    )
}

/// POST /api/auth/setup — first-run admin account creation
pub async fn post_setup(
    State(state): State<ApiState>,
    Json(req): Json<SetupRequest>,
) -> Result<Response, (StatusCode, Json<AuthResponse>)> {
    let (body, token) = auth_service::setup(&state.db, &req).await.map_err(|e| {
        let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (
            status,
            Json(AuthResponse {
                status: "error".into(),
                message: e.message,
            }),
        )
    })?;

    let mut resp = Json(body).into_response();
    resp.headers_mut()
        .insert(SET_COOKIE, session_cookie_header(&token).parse().unwrap());
    Ok(resp)
}

/// POST /api/auth/login
pub async fn post_login(
    State(state): State<ApiState>,
    Json(req): Json<LoginRequest>,
) -> Result<Response, (StatusCode, Json<AuthResponse>)> {
    let (body, token) = auth_service::login(&state.db, &req).await.map_err(|e| {
        let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (
            status,
            Json(AuthResponse {
                status: "error".into(),
                message: e.message,
            }),
        )
    })?;

    let mut resp = Json(body).into_response();
    resp.headers_mut()
        .insert(SET_COOKIE, session_cookie_header(&token).parse().unwrap());
    Ok(resp)
}

/// POST /api/auth/logout
pub async fn post_logout(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<AuthResponse>, (StatusCode, Json<AuthResponse>)> {
    let token = headers
        .get(axum::http::header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(auth_service::extract_session_token_from_cookie);

    let body = auth_service::logout(&state.db, token.as_deref())
        .await
        .map_err(|e| {
            let status =
                StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (
                status,
                Json(AuthResponse {
                    status: "error".into(),
                    message: e.message,
                }),
            )
        })?;

    Ok(Json(body))
}
