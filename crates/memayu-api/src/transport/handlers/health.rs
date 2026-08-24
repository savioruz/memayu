/// Axum handler for the unauthenticated health endpoint.
use crate::modules::health::{dto::HealthResponse, service};
use crate::transport::middleware::ApiState;
use axum::extract::State;
use axum::Json;

/// GET /api/health — unauthenticated readiness probe.
///
/// Returns `{"status":"setup_required"}` before first-run setup is complete
/// (no admin account and/or no provider config), and `{"status":"ready"}` once
/// both are in place. Process supervisors (Docker HEALTHCHECK, systemd) should
/// target this instead of treating "port is listening" as "server is usable".
pub async fn get_health(State(state): State<ApiState>) -> Json<HealthResponse> {
    Json(service::status(&state.db).await)
}
