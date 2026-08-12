pub mod handlers;
pub mod middleware;
pub mod rate_limiter;
pub mod routes;

use crate::error::ApiError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

/// Implements IntoResponse for ApiError so Axum handlers can return
/// `Result<T, ApiError>`. Kept in the transport layer so error.rs stays
/// free of Axum types.
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = self.body();
        (status, Json(body)).into_response()
    }
}
