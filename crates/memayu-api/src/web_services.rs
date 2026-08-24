//! Application-layer facade for the web dashboard.
//!
//! Exposes a single `WebServices` handle that the `memayu-web` crate uses
//! instead of reaching into `DbClient` directly. Follows the layered pattern:
//! handler → service → repository → database.

use crate::error::ApiError;
use crate::infrastructure::db::DbClient;
use crate::modules::api_keys::dto::GenerateKeyRequest;
use crate::modules::api_keys::model::ApiKey;
use crate::modules::api_keys::service as api_key_service;
use crate::modules::auth::dto::{AuthResponse, LoginRequest, SetupRequest};
use crate::modules::auth::service as auth_service;
use crate::modules::request_logs::model::RequestLog;
use std::collections::HashMap;

/// All web-facing service operations. The web crate receives this handle via
/// axum state and never imports `DbClient` directly.
#[derive(Clone)]
pub struct WebServices {
    db: DbClient,
}

impl WebServices {
    pub fn new(db: DbClient) -> Self {
        Self { db }
    }
}

// ── Auth ──

impl WebServices {
    pub async fn auth_setup(&self, req: &SetupRequest) -> Result<(AuthResponse, String), ApiError> {
        auth_service::setup(&self.db, req).await
    }

    pub async fn auth_login(&self, req: &LoginRequest) -> Result<(AuthResponse, String), ApiError> {
        auth_service::login(&self.db, req).await
    }

    pub async fn auth_logout(&self, token: Option<&str>) -> Result<AuthResponse, ApiError> {
        auth_service::logout(&self.db, token).await
    }

    pub async fn auth_resolve_session_with_email(
        &self,
        token: &str,
    ) -> Result<(String, String), String> {
        auth_service::resolve_session_with_email(&self.db, token).await
    }

    pub async fn auth_users_empty(&self) -> Result<bool, String> {
        auth_service::users_empty(&self.db).await
    }

    pub async fn auth_resolve_session(&self, token: &str) -> Result<String, String> {
        auth_service::resolve_session(&self.db, token).await
    }
}

// ── API Keys ──

impl WebServices {
    /// Generate a new API key (business logic + persistence).
    pub async fn api_keys_generate(
        &self,
        user_id: &str,
        req: &GenerateKeyRequest,
    ) -> Result<crate::modules::api_keys::dto::GenerateKeyResponse, ApiError> {
        api_key_service::generate_key(&self.db, user_id, req).await
    }

    pub async fn api_keys_list(&self) -> Result<Vec<ApiKey>, String> {
        self.db.list_api_keys().await
    }

    pub async fn api_keys_delete(&self, id: &str) -> Result<bool, String> {
        self.db.delete_api_key(id).await
    }
}

// ── Health ──

impl WebServices {
    /// Compute the readiness status exposed by `GET /api/health`.
    pub async fn health_status(&self) -> crate::modules::health::dto::HealthResponse {
        crate::modules::health::service::status(&self.db).await
    }
}

// ── Provider Config ──

impl WebServices {
    pub async fn provider_configs(
        &self,
    ) -> Result<HashMap<String, (String, String, String, String)>, String> {
        self.db.provider_configs().await
    }

    pub async fn provider_upsert(
        &self,
        provider: &str,
        backend: &str,
        base_url: &str,
        api_key: &str,
        model: &str,
    ) -> Result<(), String> {
        self.db
            .upsert_provider_config(provider, backend, base_url, api_key, model)
            .await
    }

    pub async fn get_extraction_mode(&self) -> Result<Option<String>, String> {
        self.db.get_extraction_mode().await
    }

    pub async fn set_extraction_mode(&self, mode: &str) -> Result<(), String> {
        self.db.set_extraction_mode(mode).await
    }

    /// Shared web-side persistence for Category B settings, mirroring the CLI/TUI
    /// wizard's `finalize` write path. Called by the web `/setup` handler.
    pub async fn setup_persist(
        &self,
        llm: &memayu_config::ProviderConfig,
        embedder: &memayu_config::ProviderConfig,
        extraction_mode: memayu_core::ExtractionMode,
    ) -> Result<(), String> {
        crate::modules::providers::service::persist_provider_config(
            &self.db,
            llm,
            embedder,
            extraction_mode,
        )
        .await
    }
}

// ── Request Logs ──

impl WebServices {
    pub async fn request_log_insert(
        &self,
        method: &str,
        path: &str,
        status: u16,
        latency_ms: f64,
        auth: &str,
    ) -> Result<(), String> {
        self.db
            .insert_request_log(method, path, status, latency_ms, auth)
            .await
    }

    pub async fn request_log_list(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RequestLog>, String> {
        self.db.list_request_logs_offset(limit, offset).await
    }

    pub async fn request_log_stats(&self) -> Result<(i64, f64, f64), String> {
        self.db.request_log_stats().await
    }
}
