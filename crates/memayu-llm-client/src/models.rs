//! Lightweight connectivity probe for OpenAI-compatible providers.
//!
//! Used by `memayu doctor` to verify a provider is reachable, that the API key
//! is accepted, and that the configured model is advertised, without paying for
//! a full completion/embedding request.

use reqwest::StatusCode;
use serde::Deserialize;

/// Outcome of a `GET {base_url}/models` probe.
#[derive(Debug)]
pub enum ModelsCheck {
    /// The endpoint responded 2xx. `model_available` is true when the
    /// configured model appears in the advertised list.
    Ok { model_available: bool },
    /// The endpoint rejected the API key with 401.
    Unauthorized,
    /// A non-2xx, non-401 HTTP status, or a parse failure of the response.
    Http { status: u16, detail: String },
    /// A transport/connection-level failure (host unreachable, TLS error, ...).
    Unreachable(String),
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelsEntry>,
}

#[derive(Deserialize)]
struct ModelsEntry {
    id: String,
}

/// Probe `GET {base_url}/models` with a bearer token, reporting whether the
/// provider is reachable, the key is accepted, and the model is listed.
///
/// Builds its own [`reqwest::Client`] with sane timeouts so callers (e.g. the
/// `memayu doctor` CLI) do not need a `reqwest` dependency of their own.
pub async fn check_models(base_url: &str, api_key: Option<&str>, model: &str) -> ModelsCheck {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest::Client should build with timeouts");
    probe_models(&client, base_url, api_key, model).await
}

/// Probe using an existing client (used internally by the HTTP providers so
/// they share their configured client).
pub(crate) async fn probe_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
) -> ModelsCheck {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    match client
        .get(&url)
        .bearer_auth(api_key.unwrap_or_default())
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            if status == StatusCode::UNAUTHORIZED {
                return ModelsCheck::Unauthorized;
            }
            if !status.is_success() {
                let detail = resp.text().await.unwrap_or_default();
                return ModelsCheck::Http {
                    status: status.as_u16(),
                    detail,
                };
            }
            match resp.json::<ModelsResponse>().await {
                Ok(parsed) => ModelsCheck::Ok {
                    model_available: parsed.data.iter().any(|e| e.id == model),
                },
                Err(e) => ModelsCheck::Http {
                    status: status.as_u16(),
                    detail: format!("response parse failed: {e}"),
                },
            }
        }
        Err(e) => ModelsCheck::Unreachable(e.to_string()),
    }
}
