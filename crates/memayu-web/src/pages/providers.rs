use crate::auth::CurrentUser;
use crate::components;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::Form;
use memayu_api::{ConfigRegistry, WebServices};
use memayu_config::{EmbedderBackend, ProviderConfig};
use serde::Deserialize;

fn provider_card(
    kind: &str,
    title: &str,
    cfg: &ProviderConfig,
    has_key: bool,
    show_backend: bool,
) -> maud::Markup {
    let key_hint = if has_key {
        maud::html! { p class="text-xs text-base-content/50 mt-1" { "API key is saved. Leave blank to keep it." } }
    } else {
        maud::html! {}
    };
    let backend_field = if show_backend {
        maud::html! {
            fieldset class="fieldset" {
                label class="label" { span { "Backend" } }
                select name="backend" class="select w-full" {
                    option value="http" selected=(cfg.backend == EmbedderBackend::Http) {
                        "HTTP (remote OpenAI-compatible API)"
                    }
                    option value="local" selected=(cfg.backend == EmbedderBackend::Local) {
                        "Local (in-process Candle model, no API key)"
                    }
                }
                p class="text-xs text-base-content/50 mt-1" {
                    "Base URL and API key are only used by the HTTP backend. The local backend runs the model on-device; \'model\' is the Hugging Face model id."
                }
            }
        }
    } else {
        maud::html! {}
    };
    maud::html! {
        div class="card bg-base-100 shadow-sm" {
            div class="card-body" {
                h3 class="card-title text-lg" { (title) }
                form method="post" action="/providers" class="space-y-4 mt-2" {
                    input type="hidden" name="provider" value=(kind);
                    (backend_field)
                    fieldset class="fieldset" {
                        label class="label" { span { "Base URL" } }
                        input type="url" name="base_url"
                            class="input w-full"
                            value=(cfg.base_url.as_str());
                    }
                    fieldset class="fieldset" {
                        label class="label" { span { "API Key" } }
                        input type="password" name="api_key"
                            class="input w-full"
                            value=@if has_key { "••••••••" } @else { "" }
                            placeholder=@if has_key { "••••••••" } @else { "sk-..." };
                        (key_hint)
                    }
                    fieldset class="fieldset" {
                        label class="label" { span { "Model" } }
                        input type="text" name="model"
                            class="input w-full"
                            value=(cfg.model.as_str()) required;
                    }
                    button type="submit" class="btn btn-primary" { "Save" }
                }
            }
        }
    }
}

pub async fn get_providers(
    user: CurrentUser,
    State(registry): State<ConfigRegistry>,
) -> Result<Html<String>, (StatusCode, String)> {
    let llm = registry.llm();
    let embedder = registry.embedder();
    let llm_has_key = llm.api_key.as_deref().is_some_and(|k| !k.is_empty());
    let emb_has_key = embedder.api_key.as_deref().is_some_and(|k| !k.is_empty());
    let body = maud::html! {
        div class="grid grid-cols-1 md:grid-cols-2 gap-6" {
            (provider_card("llm", "LLM (extraction)", &llm, llm_has_key, false))
            (provider_card("embedder", "Embedder", &embedder, emb_has_key, true))
        }
    };
    Ok(Html(components::render_page(
        "providers",
        Some(&user.email),
        "Configuration",
        "Configuration",
        body,
    )))
}

#[derive(Debug, Deserialize)]
pub struct ProviderForm {
    pub provider: String,
    pub backend: Option<String>,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

pub async fn post_providers(
    user: CurrentUser,
    State(registry): State<ConfigRegistry>,
    State(services): State<WebServices>,
    Form(form): Form<ProviderForm>,
) -> Result<Html<String>, (StatusCode, String)> {
    let api_key = if form.api_key == "••••••••" {
        let existing = services.provider_configs().await.unwrap_or_default();
        existing
            .get(&form.provider)
            .map(|(_, k, _)| k.clone())
            .unwrap_or_default()
    } else {
        form.api_key.clone()
    };
    let api_key_for_config = if api_key.is_empty() {
        None
    } else {
        Some(api_key.clone())
    };

    // The embedder backend lives in the config file (authoritative). A DB save
    // must not clobber it: fall back to the current registry backend unless the
    // form explicitly picked one. The LLM is always HTTP.
    let backend = match form.provider.as_str() {
        "embedder" => form
            .backend
            .as_deref()
            .and_then(|b| b.parse().ok())
            .unwrap_or_else(|| registry.embedder().backend),
        _ => EmbedderBackend::Http,
    };
    let new_config = ProviderConfig {
        backend,
        base_url: form.base_url.clone(),
        api_key: api_key_for_config,
        model: form.model.clone(),
    };

    let probe = match form.provider.as_str() {
        "llm" => None,
        "embedder" => {
            let provider = memayu_llm_client::build_embedder(&new_config);
            match provider.embed("dimension probe").await {
                Ok(v) => Some(Ok(v.len())),
                Err(e) => Some(Err(format!("{}", e))),
            }
        }
        _ => return Err((StatusCode::BAD_REQUEST, "unknown provider".into())),
    };

    services
        .provider_upsert(&form.provider, &form.base_url, &api_key, &form.model)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let _ = services
        .request_log_insert("POST", "/providers", 200, 0.0, "Session")
        .await;

    match form.provider.as_str() {
        "llm" => registry.set_llm(new_config),
        "embedder" => registry.set_embedder(new_config),
        _ => {}
    }

    let msg = match probe {
        None => "Saved.".into(),
        Some(Err(e)) => format!("Saved, but dimension probe failed: {e}"),
        Some(Ok(dim)) => format!("Saved. Embedding dimension: {dim}."),
    };

    // Re-render the full page after save
    let llm = registry.llm();
    let embedder = registry.embedder();
    let llm_has_key = llm.api_key.as_deref().is_some_and(|k| !k.is_empty());
    let emb_has_key = embedder.api_key.as_deref().is_some_and(|k| !k.is_empty());
    let body = maud::html! {
        div class="alert alert-success mt-4 mb-4" role="alert" x-data="{ open: true }" x-show="open" {
            svg xmlns="http://www.w3.org/2000/svg" class="stroke-current shrink-0 h-5 w-5" fill="none" viewBox="0 0 24 24" {
                path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" {}
            }
            span class="flex-1" { (msg) }
            button type="button" class="alert-close" x-on:click="open = false" aria-label="Close alert" {
                svg class="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                    path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" {}
                }
            }
        }
        div class="grid grid-cols-1 md:grid-cols-2 gap-6" {
            (provider_card("llm", "LLM (extraction)", &llm, llm_has_key, false))
            (provider_card("embedder", "Embedder", &embedder, emb_has_key, true))
        }
    };
    Ok(Html(components::render_page(
        "providers",
        Some(&user.email),
        "Configuration",
        "Configuration",
        body,
    )))
}
