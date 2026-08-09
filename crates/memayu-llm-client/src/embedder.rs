use async_trait::async_trait;
use memayu_config::ProviderConfig;
use memayu_core::{EmbedError, EmbedderProvider};
use reqwest::StatusCode;
use serde::Deserialize;

pub struct HttpEmbedderProvider {
    client: reqwest::Client,
    config: ProviderConfig,
}

impl HttpEmbedderProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }
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
        let resp = self
            .client
            .post(&url)
            .bearer_auth(self.config.api_key.as_deref().unwrap_or_default())
            .json(&body)
            .send()
            .await
            .map_err(|e| EmbedError::Other(format!("embedding request to {url} failed: {e}")))?;

        if resp.status() == StatusCode::UNAUTHORIZED {
            return Err(EmbedError::Other(
                "embedding provider rejected the API key (401)".into(),
            ));
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(EmbedError::Other(format!(
                "embedding provider returned {status}: {text}"
            )));
        }

        let parsed: EmbedResponse = resp
            .json()
            .await
            .map_err(|e| EmbedError::Other(format!("embedding response parse failed: {e}")))?;
        parsed
            .data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| EmbedError::Other("embedding returned no data".into()))
    }
}
