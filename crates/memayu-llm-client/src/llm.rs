use crate::models::ModelsCheck;
use async_trait::async_trait;
use memayu_config::ProviderConfig;
use memayu_core::{extraction, ExtractionResult, LlmError, LlmProvider, Message};
use serde::Deserialize;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RETRIES: u32 = 3;
const BASE_BACKOFF_MS: u64 = 500;

pub struct HttpLlmProvider {
    client: reqwest::Client,
    config: ProviderConfig,
}

impl HttpLlmProvider {
    pub fn new(config: ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("reqwest::Client should build with timeouts");
        Self { client, config }
    }

    /// Probe `GET {base_url}/models` for connectivity, key validity, and model
    /// availability without issuing a completion request.
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
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: Option<String>,
}

#[async_trait]
impl LlmProvider for HttpLlmProvider {
    async fn extract(&self, messages: &[Message]) -> Result<ExtractionResult, LlmError> {
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": messages.iter().map(|m| {
                serde_json::json!({ "role": m.role, "content": m.content })
            }).collect::<Vec<_>>(),
            "response_format": { "type": "json_object" },
        });

        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );

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

                    if status.is_success() {
                        let raw_body = r.text().await.map_err(|e| {
                            LlmError::Other(format!("LLM response body read failed: {e}"))
                        })?;
                        let parsed: ChatResponse = serde_json::Deserializer::from_str(&raw_body)
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                LlmError::Other(format!(
                                    "LLM response was empty. Body: {}",
                                    &raw_body[..raw_body.len().min(500)]
                                ))
                            })?
                            .map_err(|e| {
                                LlmError::Other(format!(
                                    "LLM response parse failed: {e}. Body: {}",
                                    &raw_body[..raw_body.len().min(500)]
                                ))
                            })?;
                        let content = parsed
                            .choices
                            .into_iter()
                            .next()
                            .and_then(|c| c.message.content)
                            .ok_or_else(|| {
                                LlmError::Other("LLM returned no choices or null content".into())
                            })?;

                        return extraction::parse_extraction_shape_only(&content)
                            .map_err(|e| LlmError::Other(format!("{e}")));
                    }

                    // Non-success status — retry on 429 or 5xx
                    let text = r.text().await.unwrap_or_default();
                    let err_msg = format!("LLM provider returned {status}: {text}");
                    if status.as_u16() == 429 || status.is_server_error() {
                        last_error = Some(LlmError::Other(err_msg.clone()));
                        if attempt < MAX_RETRIES {
                            let delay_ms = retry_after_ms(&headers)
                                .unwrap_or(BASE_BACKOFF_MS * 2u64.pow(attempt));
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            continue;
                        }
                    }
                    return Err(LlmError::Other(err_msg));
                }
                Err(e) => {
                    last_error = Some(LlmError::Other(format!("LLM request to {url} failed: {e}")));
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
            .unwrap_or_else(|| LlmError::Other("LLM request failed after retries".into())))
    }
}
