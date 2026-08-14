/// Axum handlers for request log routes — thin wrappers that query the DB
/// directly (request_logs has no business logic, it's a read-only view).
use crate::error::ApiError;
use crate::modules::request_logs::dto::{
    RequestLogEntry, RequestLogQuery, RequestLogStats, RequestLogsResponse,
};
use crate::transport::middleware::{AccountId, ApiState};
use axum::extract::{Query, State};
use axum::Json;

/// GET /api/request-logs
pub async fn get_request_logs(
    State(state): State<ApiState>,
    _account: AccountId,
    Query(q): Query<RequestLogQuery>,
) -> Result<Json<RequestLogsResponse>, ApiError> {
    let logs = state
        .db
        .list_request_logs_offset(q.limit, q.offset)
        .await
        .map_err(|e| ApiError {
            status: 500,
            error: "internal_error".into(),
            message: e,
        })?;
    let (total, avg, rate) = state.db.request_log_stats().await.map_err(|e| ApiError {
        status: 500,
        error: "internal_error".into(),
        message: e,
    })?;

    Ok(Json(RequestLogsResponse {
        logs: logs
            .into_iter()
            .map(|l| RequestLogEntry {
                id: l.id,
                created_at: l.created_at,
                method: l.method,
                path: l.path,
                status: l.status,
                latency_ms: l.latency_ms,
                auth: l.auth,
            })
            .collect(),
        stats: RequestLogStats {
            total,
            avg_latency_ms: Some(avg),
            success_rate: rate,
        },
        limit: q.limit,
        offset: q.offset,
    }))
}
