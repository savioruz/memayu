mod assets;
mod auth;
pub mod components;
mod pages;

use axum::extract::FromRef;
use axum::response::Redirect;
use axum::routing::{get, post};
use axum::Router;
use memayu_api::{ConfigRegistry, WebServices};
use memayu_core::MemoryService;
use std::sync::Arc;

/// Web router state — the web crate never accesses DbClient directly.
/// All data access goes through `WebServices` (the memayu-api service layer).
#[derive(Clone)]
pub struct WebState {
    pub services: WebServices,
    pub service: Arc<MemoryService>,
    pub registry: ConfigRegistry,
}

impl FromRef<WebState> for WebServices {
    fn from_ref(s: &WebState) -> Self {
        s.services.clone()
    }
}
impl FromRef<WebState> for Arc<MemoryService> {
    fn from_ref(s: &WebState) -> Self {
        s.service.clone()
    }
}
impl FromRef<WebState> for ConfigRegistry {
    fn from_ref(s: &WebState) -> Self {
        s.registry.clone()
    }
}

/// Build the web dashboard router.
pub fn build_web_router(
    db: memayu_api::DbClient,
    service: Arc<MemoryService>,
    registry: ConfigRegistry,
) -> Router {
    let state = WebState {
        services: WebServices::new(db),
        service,
        registry,
    };

    Router::new()
        // Auth pages
        .route(
            "/setup",
            get(pages::setup::get_setup).post(pages::setup::post_setup),
        )
        .route(
            "/login",
            get(pages::login::get_login).post(pages::login::post_login),
        )
        .route(
            "/logout",
            get(pages::logout::get_logout).post(pages::logout::post_logout),
        )
        // Dashboard pages
        .route("/", get(|| async { Redirect::permanent("/home") }))
        .route("/home", get(pages::home::get_home))
        .route("/home/list", get(pages::home::get_home_list))
        .route("/home/search", post(pages::home::post_search))
        .route("/requests", get(pages::requests::get_requests))
        .route(
            "/providers",
            get(pages::providers::get_providers).post(pages::providers::post_providers),
        )
        .route("/api-keys", get(pages::api_keys::get_api_keys))
        .route(
            "/api-keys/generate",
            post(pages::api_keys::post_generate_key),
        )
        .route(
            "/api-keys/{id}/delete",
            post(pages::api_keys::delete_api_key),
        )
        .route("/static/{*path}", get(assets::serve_static))
        .with_state(state)
}

/// Minimal router for the setup-only boot path (#55). Served when the process
/// starts with a valid infrastructure slice (storage + bind address + port)
/// but no fully-valid config yet — i.e. a fresh, unconfigured install. Only the
/// first-run setup page and static assets are reachable; the dashboard,
/// providers, and API routes are intentionally absent until setup completes
/// and the server is restarted.
///
/// The setup handlers only touch [`WebServices`] (they never need
/// `MemoryService` or a `ConfigRegistry`), so no placeholder service is
/// required here.
pub fn build_setup_router(db: memayu_api::DbClient) -> Router {
    Router::new()
        .route(
            "/setup",
            get(pages::setup::get_setup).post(pages::setup::post_setup),
        )
        .route("/static/{*path}", get(assets::serve_static))
        .with_state(WebServices::new(db))
}
