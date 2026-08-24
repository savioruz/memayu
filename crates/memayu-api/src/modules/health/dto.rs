use serde::Serialize;

/// Health status of the server. `setup_required` means the server is listening
/// but not yet usable (first-run setup incomplete); `ready` means the admin
/// account and provider config are both in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    SetupRequired,
    Ready,
}

/// Response body for `GET /api/health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthResponse {
    pub status: HealthStatus,
}
