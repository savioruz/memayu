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
                "remote",
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
        // Normalize so a local embedder clears any stale base_url/api_key — in
        // the persisted row (the DB upsert also normalizes as a backstop) and
        // in the in-memory registry, so GET reflects the cleared values.
        let cfg = cfg.clone().normalize();
        state
            .db
            .upsert_provider_config(
                "embedder",
                &cfg.backend.to_string(),
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
        state.provider_configs.set_embedder(cfg);
    }
    if let Some(mode) = &req.extraction_mode {
        state
            .db
            .set_extraction_mode(mode)
            .await
            .map_err(|e| ApiError {
                status: 400,
                error: "bad_request".into(),
                message: e,
            })?;
    }

    let llm = state.provider_configs.llm();
    let embedder = state.provider_configs.embedder();
    Ok(Json(ProviderConfigResponse { llm, embedder }))
}
