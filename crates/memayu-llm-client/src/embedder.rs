use crate::models::ModelsCheck;
use async_trait::async_trait;
use memayu_config::ProviderConfig;
use memayu_core::{EmbedError, EmbedderProvider};
use reqwest::StatusCode;
use serde::Deserialize;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RETRIES: u32 = 3;
const BASE_BACKOFF_MS: u64 = 500;

pub struct HttpEmbedderProvider {
    client: reqwest::Client,
    config: ProviderConfig,
}

impl HttpEmbedderProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("reqwest::Client should build with timeouts");
        Self { client, config }
    }

    /// Probe `GET {base_url}/models` for connectivity, key validity, and model
    /// availability without issuing an embedding request.
    pub async fn check_models(&self) -> ModelsCheck {
        crate::models::probe_models(
            &self.client,
            &self.config.base_url,
            self.config.api_key.as_deref(),
            &self.config.model,
        )
        .await
    }
}

fn retry_after_ms(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok())
        .map(|secs| (secs * 1000.0) as u64)
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbedderProvider for HttpEmbedderProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let body = serde_json::json!({
            "model": self.config.model,
            "input": text,
        });

        let url = format!("{}/embeddings", self.config.base_url.trim_end_matches('/'));

        let mut last_error = None;
        for attempt in 0..=MAX_RETRIES {
            let resp = self
                .client
                .post(&url)
                .bearer_auth(self.config.api_key.as_deref().unwrap_or_default())
                .json(&body)
                .send()
                .await;

            match resp {
                Ok(r) => {
                    let status = r.status();
                    let headers = r.headers().clone();

                    if status == StatusCode::UNAUTHORIZED {
                        return Err(EmbedError::Other(
                            "embedding provider rejected the API key (401)".into(),
                        ));
                    }

                    if status.is_success() {
                        let parsed: EmbedResponse = r.json().await.map_err(|e| {
                            EmbedError::Other(format!("embedding response parse failed: {e}"))
                        })?;
                        return parsed
                            .data
                            .into_iter()
                            .next()
                            .map(|d| d.embedding)
                            .ok_or_else(|| EmbedError::Other("embedding returned no data".into()));
                    }

                    let text = r.text().await.unwrap_or_default();
                    let err_msg = format!("embedding provider returned {status}: {text}");
                    if status.as_u16() == 429 || status.is_server_error() {
                        last_error = Some(EmbedError::Other(err_msg.clone()));
                        if attempt < MAX_RETRIES {
                            let delay_ms = retry_after_ms(&headers)
                                .unwrap_or(BASE_BACKOFF_MS * 2u64.pow(attempt));
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            continue;
                        }
                    }
                    return Err(EmbedError::Other(err_msg));
                }
                Err(e) => {
                    last_error = Some(EmbedError::Other(format!(
                        "embedding request to {url} failed: {e}"
                    )));
                    if attempt < MAX_RETRIES {
                        tokio::time::sleep(Duration::from_millis(
                            BASE_BACKOFF_MS * 2u64.pow(attempt),
                        ))
                        .await;
                        continue;
                    }
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| EmbedError::Other("embedding request failed after retries".into())))
    }
}
