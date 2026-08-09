#[derive(Debug, serde::Deserialize)]
pub struct RequestLogQuery {
    #[serde(default = "default_request_log_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_request_log_limit() -> usize {
    50
}

#[derive(Debug, serde::Serialize)]
pub struct RequestLogStats {
    pub total: i64,
    pub avg_latency_ms: f64,
    pub success_rate: f64,
}

#[derive(Debug, serde::Serialize)]
pub struct RequestLogEntry {
    pub id: String,
    pub created_at: String,
    pub method: String,
    pub path: String,
    pub status: i64,
    pub latency_ms: f64,
    pub auth: String,
}

#[derive(Debug, serde::Serialize)]
pub struct RequestLogsResponse {
    pub logs: Vec<RequestLogEntry>,
    pub stats: RequestLogStats,
    pub limit: usize,
    pub offset: usize,
}
