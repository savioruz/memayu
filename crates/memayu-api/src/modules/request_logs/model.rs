pub struct RequestLog {
    pub id: String,
    pub created_at: String,
    pub method: String,
    pub path: String,
    pub status: i64,
    pub latency_ms: f64,
    pub auth: String,
}
