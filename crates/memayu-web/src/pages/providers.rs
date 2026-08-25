use crate::auth::CurrentUser;
use crate::components;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::Form;
use memayu_api::{ConfigRegistry, WebServices};
use memayu_config::{EmbedderBackend, ProviderConfig};
use serde::Deserialize;

fn llm_card(cfg: &ProviderConfig, mode: &str, has_key: bool) -> maud::Markup {
    let key_hint = if has_key {
        maud::html! { p class="text-xs text-base-content/50 mt-1" { "API key is saved. Leave blank to keep it." } }
    } else {
        maud::html! {}
    };

    let alpine_expr = format!("{{ mode: '{}' }}", mode);

    maud::html! {
        div class="card bg-base-100 shadow-sm" {
            div class="card-body" {
                h3 class="card-title text-lg mb-1" { "LLM & Extraction" }
                p class="text-xs text-base-content/60 mb-4" {
                    "Configure memory extraction mode and upstream LLM inference endpoint."
                }
                form method="post" action="/providers" class="space-y-4" x-data=(alpine_expr) {
                    input type="hidden" name="provider" value="llm";

                    fieldset class="fieldset" {
                        label class="label" { span { "Extraction Mode" } }
                        select name="extraction_mode" class="select w-full" x-model="mode" {
                            option value="llm" {
                                "LLM Extraction (structured facts & insights)"
                            }
                            option value="raw" {
                                "Raw Text Mode (store chunks verbatim)"
                            }
                        }
                        p class="text-xs text-base-content/50 mt-1" {
                            "Mode changes take effect on the next server restart."
                        }
                    }

                    div x-show="mode === 'raw'" {
                        span class="inline-block bg-base-200 px-3 py-2 text-xs text-base-content/70" {
                            "Raw Text Mode stores memories verbatim without LLM extraction."
                        }
                    }

                    div x-show="mode === 'llm'" class="space-y-4" {
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
                                value=(cfg.model.as_str());
                        }
                    }

                    div class="pt-2" {
                        button type="submit" class="btn btn-primary" { "Save" }
                    }
                }
            }
        }
    }
}

fn embedder_card(cfg: &ProviderConfig, has_key: bool) -> maud::Markup {
    let key_hint = if has_key {
        maud::html! { p class="text-xs text-base-content/50 mt-1" { "API key is saved. Leave blank to keep it." } }
    } else {
        maud::html! {}
    };

    let backend_str = if cfg.backend == EmbedderBackend::Local {
        "local"
    } else {
        "remote"
    };

    let matched_local_spec = memayu_setup::LOCAL_MODELS
        .iter()
        .find(|m| m.id == cfg.model || m.name == cfg.model)
        .unwrap_or(&memayu_setup::LOCAL_MODELS[memayu_setup::DEFAULT_MODEL_INDEX]);

    let initial_local_model_id = matched_local_spec.id;
    let initial_local_model_name = matched_local_spec.name;
    let initial_local_dim = matched_local_spec.dim;
    let initial_local_fp32 = matched_local_spec.fp32_size_mb;
    let initial_local_int8 = matched_local_spec.int8_size_mb;
    let initial_local_ram = matched_local_spec.min_ram_mb;
    let initial_local_cpu = matched_local_spec.cpu_notes;
    let initial_local_langs = matched_local_spec.langs;
    let initial_remote_model = if cfg.backend == EmbedderBackend::Remote {
        cfg.model.as_str()
    } else {
        "text-embedding-3-small"
    };

    let alpine_expr = format!(
        "{{ backend: '{}', dropdown_open: false, local_model: '{}', local_model_name: '{}', local_dim: {}, local_fp32: {}, local_int8: {}, local_ram: {}, local_cpu: '{}', local_langs: '{}', remote_model: '{}' }}",
        backend_str,
        initial_local_model_id,
        initial_local_model_name,
        initial_local_dim,
        initial_local_fp32,
        initial_local_int8,
        initial_local_ram,
        initial_local_cpu,
        initial_local_langs,
        initial_remote_model,
    );

    maud::html! {
        div class="card bg-base-100 shadow-sm" {
            div class="card-body" {
                h3 class="card-title text-lg mb-1" { "Embedder" }
                p class="text-xs text-base-content/60 mb-4" {
                    "Configure vector embedding model for semantic similarity search."
                }
                form method="post" action="/providers" class="space-y-4" x-data=(alpine_expr) {
                    input type="hidden" name="provider" value="embedder";
                    input type="hidden" name="model" x-bind:value="backend === 'local' ? local_model : remote_model" value=(if cfg.backend == EmbedderBackend::Local { initial_local_model_id } else { cfg.model.as_str() });

                    fieldset class="fieldset" {
                        label class="label" { span { "Backend" } }
                        select name="backend" class="select w-full" x-model="backend" {
                            option value="remote" {
                                "HTTP (remote OpenAI-compatible API)"
                            }
                            option value="local" {
                                "Local (in-process Candle model, no API key)"
                            }
                        }
                    }

                    // ── Local Model Dropdown ──
                    div x-show="backend === 'local'" class="space-y-4" {
                        fieldset class="fieldset" {
                            label class="label" { span { "Model" } }
                            div class="dropdown w-full" x-on:focusout="if (!$el.contains($event.relatedTarget)) dropdown_open = false" {
                                button
                                    type="button"
                                    class="model-dropdown-trigger font-normal"
                                    tabindex="0"
                                    x-on:click="dropdown_open = !dropdown_open" {
                                    div class="flex items-center gap-2 flex-1 min-w-0" {
                                        span class="font-bold text-sm truncate" x-text="local_model_name" { (initial_local_model_name) }
                                        span class="text-xs opacity-60 hidden sm:inline" x-text="'~' + local_int8 + ' MB (q8)'" { "~" (initial_local_int8) " MB (q8)" }
                                    }
                                    svg xmlns="http://www.w3.org/2000/svg" class="h-4 w-4 opacity-60 ml-2 shrink-0 transition-transform" x-bind:class="{ 'rotate-180': dropdown_open }" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                                        path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" {}
                                    }
                                }
                                div class="dropdown-content bg-base-200 border border-base-300 w-full z-20 mt-1 shadow-xl overflow-hidden"
                                    x-show="dropdown_open"
                                    x-cloak {
                                    @for m in memayu_setup::LOCAL_MODELS {
                                        div class="model-option-item"
                                            x-bind:class=(format!("{{ 'active': local_model === '{}' }}", m.id))
                                            x-on:click=(format!("local_model = '{}'; local_model_name = '{}'; local_dim = {}; local_fp32 = {}; local_int8 = {}; local_ram = {}; local_cpu = '{}'; local_langs = '{}'; dropdown_open = false;", m.id, m.name, m.dim, m.fp32_size_mb, m.int8_size_mb, m.min_ram_mb, m.cpu_notes, m.langs)) {
                                            div class="flex items-center justify-between" {
                                                span class="font-bold text-sm text-base-content" { (m.name) }
                                                div class="flex items-center gap-1.5" {
                                                    span class="badge badge-sm font-mono text-xs font-semibold" { (m.dim) "D" }
                                                    span class="badge badge-sm badge-outline text-xs" { (m.langs) }
                                                }
                                            }
                                            div class="text-xs text-base-content/60 mt-1" {
                                                "~" (m.int8_size_mb) "MB · min " (m.min_ram_mb) "MB RAM"
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        div class="flex items-center gap-3 text-xs text-base-content/60 my-2" {
                            span { "Embedding vector dimension:" }
                            span class="badge badge-outline text-xs font-mono font-semibold" x-text="local_dim" { (initial_local_dim) }
                        }
                    }

                    // ── Remote Embedder Fields ──
                    div x-show="backend === 'remote'" class="space-y-4" {
                        fieldset class="fieldset" {
                            label class="label" { span { "Base URL" } }
                            input type="url" name="base_url"
                                class="input w-full"
                                value=(cfg.base_url.as_str())
                                placeholder="https://api.openai.com/v1";
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
                            label class="label" { span { "Model Name" } }
                            input type="text"
                                class="input w-full"
                                x-model="remote_model"
                                placeholder="text-embedding-3-small";
                            p class="text-xs text-base-content/50 mt-1" {
                                "Model name for the remote OpenAI-compatible embedding endpoint."
                            }
                        }
                    }

                    div class="pt-2" {
                        button type="submit" class="btn btn-primary" { "Save" }
                    }
                }
            }
        }
    }
}

/// Render the full providers/settings page. `msg` is an optional success alert.
async fn render_config(
    user: CurrentUser,
    registry: ConfigRegistry,
    services: WebServices,
    msg: &str,
) -> Result<Html<String>, (StatusCode, String)> {
    let llm = registry.llm();
    let embedder = registry.embedder();
    let mode = services
        .get_extraction_mode()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .unwrap_or_else(|| "llm".into());
    let llm_has_key = llm.api_key.as_deref().is_some_and(|k| !k.is_empty());
    let emb_has_key = embedder.api_key.as_deref().is_some_and(|k| !k.is_empty());
    let body = maud::html! {
        div class="mb-6" {
            h2 class="text-xl font-bold" { "Providers" }
            p class="text-xs text-base-content/60 mt-1" {
                "Configure embedding models, LLM extraction endpoints, and memory extraction mode."
            }
        }
        @if !msg.is_empty() {
            div class="alert alert-success mb-6" role="alert" x-data="{ open: true }" x-show="open" {
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
        }
        div class="grid grid-cols-1 lg:grid-cols-2 gap-6" {
            (llm_card(&llm, &mode, llm_has_key))
            (embedder_card(&embedder, emb_has_key))
        }
    };
    Ok(Html(components::render_page(
        "providers",
        Some(&user.email),
        "Providers",
        "Providers",
        body,
    )))
}

pub async fn get_providers(
    user: CurrentUser,
    State(registry): State<ConfigRegistry>,
    State(services): State<WebServices>,
) -> Result<Html<String>, (StatusCode, String)> {
    render_config(user, registry, services, "").await
}

#[derive(Debug, Deserialize)]
pub struct ProviderForm {
    pub provider: String,
    pub backend: Option<String>,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub extraction_mode: Option<String>,
}

pub async fn post_providers(
    user: CurrentUser,
    State(registry): State<ConfigRegistry>,
    State(services): State<WebServices>,
    Form(form): Form<ProviderForm>,
) -> Result<Html<String>, (StatusCode, String)> {
    // Extraction-mode-only form post (backward compatibility)
    if form.provider == "extraction_mode" {
        let mode = form.extraction_mode.clone().unwrap_or_else(|| "llm".into());
        if mode != "llm" && mode != "raw" {
            return Err((StatusCode::BAD_REQUEST, "invalid extraction_mode".into()));
        }
        services
            .set_extraction_mode(&mode)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let _ = services
            .request_log_insert("POST", "/providers", 200, 0.0, "Session")
            .await;
        return render_config(user, registry, services, "Saved.").await;
    }

    // When saving the LLM card, also persist the extraction mode if supplied.
    if form.provider == "llm" {
        if let Some(mode) = &form.extraction_mode {
            if mode == "llm" || mode == "raw" {
                services
                    .set_extraction_mode(mode)
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
            }
        }
    }

    let api_key = if form.api_key == "••••••••" {
        let existing = services.provider_configs().await.unwrap_or_default();
        existing
            .get(&form.provider)
            .map(|(_, _, k, _)| k.clone())
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
        _ => EmbedderBackend::Remote,
    };
    let model = if form.model.is_empty() && form.provider == "llm" {
        registry.llm().model
    } else {
        form.model.clone()
    };
    let base_url = if form.base_url.is_empty() && form.provider == "llm" {
        registry.llm().base_url
    } else {
        form.base_url.clone()
    };

    let new_config = ProviderConfig {
        backend,
        base_url: base_url.clone(),
        api_key: api_key_for_config,
        model: model.clone(),
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
        .provider_upsert(
            &form.provider,
            &backend.to_string(),
            &base_url,
            &api_key,
            &model,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let _ = services
        .request_log_insert("POST", "/providers", 200, 0.0, "Session")
        .await;

    match form.provider.as_str() {
        "llm" => registry.set_llm(new_config),
        // Normalize so a local embedder clears any stale base_url/api_key in
        // the live registry too (the row itself is normalized by the DB write).
        "embedder" => registry.set_embedder(new_config.normalize()),
        _ => {}
    }

    let msg = match probe {
        None => "Saved.".into(),
        Some(Err(e)) => format!("Saved, but dimension probe failed: {e}"),
        Some(Ok(dim)) => format!("Saved. Embedding dimension: {dim}."),
    };

    render_config(user, registry, services, &msg).await
}
