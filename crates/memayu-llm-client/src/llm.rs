use async_trait::async_trait;
use memayu_config::ProviderConfig;
use memayu_core::{extraction, ExtractionResult, LlmError, LlmProvider, Message};
use serde::Deserialize;

pub struct HttpLlmProvider {
    client: reqwest::Client,
    config: ProviderConfig,
}

impl HttpLlmProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }
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
        let resp = self
            .client
            .post(&url)
            .bearer_auth(self.config.api_key.as_deref().unwrap_or_default())
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Other(format!("LLM request to {url} failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::Other(format!(
                "LLM provider returned {status}: {text}"
            )));
        }

        let raw_body = resp
            .text()
            .await
            .map_err(|e| LlmError::Other(format!("LLM response body read failed: {e}")))?;
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
            .ok_or_else(|| LlmError::Other("LLM returned no choices or null content".into()))?;

        extraction::parse_extraction_shape_only(&content)
            .map_err(|e| LlmError::Other(format!("{e}")))
    }
}
