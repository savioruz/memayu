/// Axum handlers for API key routes — thin wrappers that delegate to
/// modules::api_keys::service.
use crate::error::ApiErrorBody;
use crate::modules::api_keys::dto::{GenerateKeyRequest, GenerateKeyResponse, ListKeysResponse};
use crate::modules::api_keys::service as api_key_service;
use crate::transport::middleware::{AccountId, ApiState};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

/// POST /api/api-keys/generate
pub async fn generate_key_with_user(
    State(state): State<ApiState>,
    account_id: AccountId,
    Json(req): Json<GenerateKeyRequest>,
) -> Result<Json<GenerateKeyResponse>, (StatusCode, Json<ApiErrorBody>)> {
    let resp = api_key_service::generate_key(&state.db, &account_id.0, &req)
        .await
        .map_err(|e| {
            (
                StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(e.body()),
            )
        })?;
    Ok(Json(resp))
}

/// GET /api/api-keys
pub async fn list_keys(
    State(state): State<ApiState>,
    _account_id: AccountId,
) -> Result<Json<ListKeysResponse>, (StatusCode, Json<ApiErrorBody>)> {
    let resp = api_key_service::list_keys(&state.db).await.map_err(|e| {
        (
            StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(e.body()),
        )
    })?;
    Ok(Json(resp))
}

/// DELETE /api/api-keys/:id
pub async fn delete_key(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    _account_id: AccountId,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorBody>)> {
    api_key_service::delete_key(&state.db, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(e.body()),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/auth/check-setup — used by dashboard to decide redirect
pub async fn check_setup(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let empty = state.db.users_empty().await.unwrap_or(true);
    Json(serde_json::json!({ "setup_completed": !empty }))
}
