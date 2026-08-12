use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    pub max_requests: usize,
    pub window_secs: u64,
}

impl RateLimitConfig {
    pub fn per_ip_auth() -> Self {
        Self {
            max_requests: 10,
            window_secs: 60,
        }
    }

    pub fn per_api_key() -> Self {
        Self {
            max_requests: 100,
            window_secs: 60,
        }
    }
}

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<RateLimiterInner>>,
    config: RateLimitConfig,
}

struct RateLimiterInner {
    windows: HashMap<String, Vec<Instant>>,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                windows: HashMap::new(),
            })),
            config,
        }
    }

    /// Check if a request for `key` is allowed.
    /// Returns Ok(()) if allowed, Err(retry_after_secs) if rate-limited.
    pub async fn check(&self, key: &str) -> Result<(), u64> {
        let now = Instant::now();
        let window_start = now - Duration::from_secs(self.config.window_secs);
        let mut inner = self.inner.lock().await;

        let timestamps = inner.windows.entry(key.to_string()).or_default();

        // Prune expired entries
        timestamps.retain(|t| *t > window_start);

        if timestamps.len() >= self.config.max_requests {
            let oldest = timestamps
                .first()
                .map(|t| {
                    let elapsed = now.duration_since(*t);
                    let retry = self.config.window_secs.saturating_sub(elapsed.as_secs());
                    retry.max(1)
                })
                .unwrap_or(1);
            return Err(oldest);
        }

        timestamps.push(now);
        Ok(())
    }
}
