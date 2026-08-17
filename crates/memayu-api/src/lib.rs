mod error;
mod infrastructure;
mod modules;
mod transport;
mod web_services;

pub use error::{ApiError, ApiErrorBody};
pub use infrastructure::db::DbClient;
pub use memayu_core::{Memory, MemoryService};
pub use modules::api_keys::dto as api_key_dto;
pub use modules::api_keys::model::ApiKey;
pub use modules::auth::dto as auth_dto;
pub use modules::auth::model::User;
pub use modules::auth::service::{
    expires_at_rfc3339, validate_password, SESSION_COOKIE, SESSION_DURATION_SECS,
};
pub use modules::providers::service::{
    ensure_provider_config, load_extraction_mode, load_registry, ConfigRegistry,
    EmbedderConfigProvider, LlmConfigProvider,
};
pub use modules::request_logs::model::RequestLog;
pub use transport::middleware::{
    api_rate_limiter, api_request_logger, auth_middleware, auth_rate_limiter, docs_auth_redirect,
    security_headers, AccountId, ApiState,
};
pub use transport::rate_limiter::{RateLimitConfig, RateLimiter};
pub use web_services::WebServices;

/// Build the full API router. Delegates to transport::routes::build().
pub fn build_api_router(
    db: DbClient,
    service: std::sync::Arc<memayu_core::MemoryService>,
    registry: ConfigRegistry,
) -> axum::Router {
    transport::routes::build(db, service, registry)
}

pub async fn open_db(config: &memayu_config::StorageConfig) -> Result<DbClient, String> {
    let db = DbClient::open(config).await?;
    db.init().await?;
    Ok(db)
}
