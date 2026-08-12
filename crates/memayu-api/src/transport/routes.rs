/// Builds the full API router — all routes are defined here and assembled
/// into a single Axum Router.
use crate::transport::handlers;
use crate::transport::middleware::{self, ApiState};
use axum::routing::{delete, get, post};
use axum::Router;
use std::sync::Arc;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_scalar::Servable;

/// The Memayu memory API
#[derive(OpenApi)]
#[openapi(
    info(title = "Memayu API", version = "0.1.0"),
    paths(
        handlers::memory::add_memory,
        handlers::memory::search_memory,
        handlers::memory::list_memories,
        handlers::memory::delete_memory,
        handlers::memory::update_memory,
    ),
    components(schemas(
        crate::modules::memory::dto::AddMemoryRequest,
        crate::modules::memory::dto::AddMemoryResponse,
        crate::modules::memory::dto::SearchMemoryRequest,
        crate::modules::memory::dto::SearchMemoryResponse,
        crate::modules::memory::dto::SearchResult,
        crate::modules::memory::dto::ListQuery,
        crate::modules::memory::dto::ListMemoryResponse,
        crate::modules::memory::dto::ListedMemory,
        crate::modules::memory::dto::UpdateMemoryRequest,
        crate::modules::memory::dto::UpdateMemoryResponse,
        crate::error::ApiErrorBody,
    )),
    tags(
        (name = "memory", description = "Memory operations")
    )
)]
pub struct ApiDoc;

/// Build the full API router including auth, API keys, and memory endpoints.
/// All routes are behind the auth middleware except /docs which uses
/// a redirect-to-login middleware for unauthenticated users.
pub fn build(
    db: crate::infrastructure::db::DbClient,
    service: Arc<crate::MemoryService>,
    registry: crate::modules::providers::service::ConfigRegistry,
) -> Router {
    let state = ApiState::new(db.clone(), service, registry);

    // OpenAPI memory routes (protected by auth)
    let (memory_routes, openapi_spec) = {
        let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
            .routes(routes!(handlers::memory::add_memory))
            .routes(routes!(handlers::memory::search_memory))
            .routes(routes!(handlers::memory::list_memories))
            .routes(routes!(handlers::memory::delete_memory))
            .routes(routes!(handlers::memory::update_memory))
            .split_for_parts();
        (router, api)
    };

    // /docs is served outside the auth layer; unauthenticated users see a redirect to /login
    let docs_routes: Router<ApiState> =
        utoipa_scalar::Scalar::with_url("/docs", openapi_spec).into();
    let docs_routes = docs_routes.route_layer(axum::middleware::from_fn_with_state(
        db.clone(),
        middleware::docs_auth_redirect,
    ));

    // Auth routes (public — /api prefix) — with IP-based rate limiting
    let auth_routes = Router::new()
        .route("/api/auth/setup", post(handlers::auth::post_setup))
        .route("/api/auth/login", post(handlers::auth::post_login))
        .route("/api/auth/logout", post(handlers::auth::post_logout))
        .route(
            "/api/auth/check-setup",
            get(handlers::api_keys::check_setup),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::auth_rate_limiter,
        ));

    // Provider config routes (protected by auth)
    let provider_routes = Router::new().route(
        "/api/providers",
        get(handlers::providers::get_providers).post(handlers::providers::post_providers),
    );

    // API key routes (protected by auth)
    let api_key_routes = Router::new()
        .route("/api/api-keys", get(handlers::api_keys::list_keys))
        .route(
            "/api/api-keys/generate",
            post(handlers::api_keys::generate_key_with_user),
        )
        .route("/api/api-keys/{id}", delete(handlers::api_keys::delete_key));

    // Request log routes (protected by auth)
    let request_log_routes = Router::new().route(
        "/api/request-logs",
        get(handlers::request_logs::get_request_logs),
    );

    // All protected routes — with API-key/session rate limiting
    let protected = Router::new()
        .merge(memory_routes)
        .merge(provider_routes)
        .merge(api_key_routes)
        .merge(request_log_routes)
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::api_rate_limiter,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
            state.db.clone(),
            middleware::auth_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.db.clone(),
            middleware::api_request_logger,
        ));

    // Merge public + protected — security headers + CORS + request ID on the outermost layer
    Router::new()
        .merge(auth_routes)
        .merge(docs_routes)
        .merge(protected)
        .layer(axum::middleware::from_fn(middleware::security_headers))
        .layer(axum::middleware::from_fn(middleware::cors_middleware))
        .layer(axum::middleware::from_fn(middleware::request_id))
        .with_state(state)
}
