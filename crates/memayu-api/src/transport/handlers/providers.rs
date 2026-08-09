/// Axum handlers for provider routes — thin wrappers that delegate to
/// modules::providers::service.
use crate::error::ApiError;
use crate::modules::providers::dto::{ProviderConfigRequest, ProviderConfigResponse};
use crate::transport::middleware::{AccountId, ApiState};
use axum::extract::State;
use axum::Json;

/// GET /api/providers — get both LLM and embedder provider configs
pub async fn get_providers(
    State(state): State<ApiState>,
    _account: AccountId,
) -> Json<ProviderConfigResponse> {
    let llm = state.provider_configs.llm();
    let embedder = state.provider_configs.embedder();
    Json(ProviderConfigResponse { llm, embedder })
}

/// POST /api/providers — upsert provider configs and refresh registry
pub async fn post_providers(
    State(state): State<ApiState>,
    _account: AccountId,
    Json(req): Json<ProviderConfigRequest>,
) -> Result<Json<ProviderConfigResponse>, ApiError> {
    // Persist to DB
    if let Some(cfg) = &req.llm {
        state
            .db
            .upsert_provider_config(
                "llm",
                &cfg.base_url,
                cfg.api_key.as_deref().unwrap_or(""),
                &cfg.model,
            )
            .await
            .map_err(|e| ApiError {
                status: 400,
                error: "bad_request".into(),
                message: e,
            })?;
        state.provider_configs.set_llm(cfg.clone());
    }
    if let Some(cfg) = &req.embedder {
        state
            .db
            .upsert_provider_config(
                "embedder",
                &cfg.base_url,
                cfg.api_key.as_deref().unwrap_or(""),
                &cfg.model,
            )
            .await
            .map_err(|e| ApiError {
                status: 400,
                error: "bad_request".into(),
                message: e,
            })?;
        state.provider_configs.set_embedder(cfg.clone());
    }

    let llm = state.provider_configs.llm();
    let embedder = state.provider_configs.embedder();
    Ok(Json(ProviderConfigResponse { llm, embedder }))
}
