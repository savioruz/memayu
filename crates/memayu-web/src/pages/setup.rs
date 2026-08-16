use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use memayu_api::{WebServices, SESSION_COOKIE, SESSION_DURATION_SECS};
use memayu_setup::{embedding_dimension, preseed, read_config_file_if_any, SetupAnswers};

// ── helpers ──

fn base_page(title: &str, content: maud::Markup) -> String {
    maud::html! {
        (maud::DOCTYPE)
        html lang="en" data-theme="dark" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " - Memayu" }
                link href="/static/mu.slate.css" rel="stylesheet";
                link href="/static/memayu.css" rel="stylesheet";
                script src="/static/htmx.min.js" {}
                script defer src="/static/alpine.min.js" {}
                script {
                    (maud::PreEscaped("
                        (function() {
                            var saved = localStorage.getItem('memayu-theme');
                            if (saved) {
                                document.documentElement.setAttribute('data-theme', saved);
                            } else if (window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches) {
                                document.documentElement.setAttribute('data-theme', 'light');
                            } else {
                                document.documentElement.setAttribute('data-theme', 'dark');
                            }
                        })();
                    "))
                }
            }
            body class="min-h-screen bg-base-300 flex items-center justify-center p-4 sm:p-6" {
                main class="w-full max-w-2xl mx-auto" {
                    (content)
                }
            }
        }
    }
    .into_string()
}

fn setup_form(a: &SetupAnswers, error: Option<&str>) -> maud::Markup {
    let emb_is_remote = a.embedder_backend == "remote";
    let extraction_is_llm = a.extraction_mode == "llm";
    let storage_is_pg = a.storage_backend == memayu_config::StorageBackend::Postgres;
    let initial_step = if error.is_some() { 5 } else { 1 };

    let default_spec = memayu_setup::LOCAL_MODELS
        .iter()
        .find(|m| m.id == a.embedder_model)
        .unwrap_or(&memayu_setup::LOCAL_MODELS[0]);
    let initial_model_id = default_spec.id;
    let initial_model_name = default_spec.name;
    let initial_dim = default_spec.dim;
    let initial_fp32 = default_spec.fp32_size_mb;
    let initial_int8 = default_spec.int8_size_mb;
    let initial_ram = default_spec.min_ram_mb;
    let initial_cpu = default_spec.cpu_notes;
    let initial_langs = default_spec.langs;

    let alpine_init = format!(
        "{{ current_step: {}, storage_backend: '{}', embedder_backend: '{}', extraction_mode: '{}', local_model: '{}', local_model_name: '{}', local_dim: {}, local_fp32: {}, local_int8: {}, local_ram: {}, local_cpu: '{}', local_langs: '{}', dropdown_open: false }}",
        initial_step,
        if storage_is_pg { "postgres" } else { "libsql" },
        if emb_is_remote { "remote" } else { "local" },
        if extraction_is_llm { "llm" } else { "raw" },
        initial_model_id,
        initial_model_name,
        embedding_dimension(a).unwrap_or(initial_dim as usize),
        initial_fp32,
        initial_int8,
        initial_ram,
        initial_cpu,
        initial_langs,
    );

    maud::html! {
        div class="card wizard-card w-full max-w-2xl mx-auto bg-base-100 shadow-sm border border-base-200/80 rounded-2xl" {
            div class="card-body p-6 sm:p-10" {
                div class="text-center mb-6" {
                    h2 class="text-2xl sm:text-3xl font-extrabold tracking-tight text-base-content mb-2" { "Set Up Memayu" }
                    p class="text-xs sm:text-sm text-base-content/60" {
                        "Configure storage, embeddings, extraction, and your administrator account."
                    }
                }

                @if let Some(e) = error {
                    div class="alert alert-error mb-6" role="alert" x-data="{ open: true }" x-show="open" {
                        svg xmlns="http://www.w3.org/2000/svg" class="h-5 w-5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" {}
                        }
                        span class="flex-1" { (e) }
                        button type="button" class="alert-close" x-on:click="open = false" aria-label="Close alert" {
                            svg class="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                                path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" {}
                            }
                        }
                    }
                }

                form method="post" action="/setup" class="wizard-form" x-data=(alpine_init) {
                    // ── Stepper Indicator ──
                    div class="stepper my-6" {
                        div class="step-item" x-bind:class="{ 'active': current_step === 1, 'completed': current_step > 1 }" {
                            div class="step-circle" {
                                span x-show="current_step >= 1" { "✓" }
                            }
                            span class="step-label" { "Storage" }
                        }
                        div class="step-item" x-bind:class="{ 'active': current_step === 2, 'completed': current_step > 2 }" {
                            div class="step-circle" {
                                span x-show="current_step >= 2" { "✓" }
                                span x-show="current_step < 2" { "2" }
                            }
                            span class="step-label" { "Embedder" }
                        }
                        div class="step-item" x-bind:class="{ 'active': current_step === 3, 'completed': current_step > 3 }" {
                            div class="step-circle" {
                                span x-show="current_step >= 3" { "✓" }
                                span x-show="current_step < 3" { "3" }
                            }
                            span class="step-label" { "Extraction" }
                        }
                        div class="step-item" x-bind:class="{ 'active': current_step === 4, 'completed': current_step > 4 }" {
                            div class="step-circle" {
                                span x-show="current_step >= 4" { "✓" }
                                span x-show="current_step < 4" { "4" }
                            }
                            span class="step-label" { "Server" }
                        }
                        div class="step-item" x-bind:class="{ 'active': current_step === 5, 'completed': current_step > 5 }" {
                            div class="step-circle" {
                                span x-show="current_step >= 5" { "✓" }
                                span x-show="current_step < 5" { "5" }
                            }
                            span class="step-label" { "Admin" }
                        }
                    }

                    // ── Step Content Body (Fixed & Non-collapsing) ──
                    div class="wizard-content wizard-step-body" {
                        // ── Step 1: Storage ──
                        div x-show="current_step === 1" class="my-4" {
                            div class="mb-4 text-left" {
                                h3 class="text-base font-bold text-base-content" { "Storage Configuration" }
                                p class="text-xs text-base-content/60 mt-0.5" { "Choose how Memayu stores memories, vector embeddings, and application data." }
                            }
                            div class="grid grid-cols-1 sm:grid-cols-2 gap-4 my-4" {
                                label class="choice-card" x-bind:class="{ 'selected': storage_backend === 'libsql' }" {
                                div class="flex items-start gap-3" {
                                    input type="radio" name="storage_backend" value="libsql" x-model="storage_backend" class="radio radio-sm mt-0.5" checked=(if !storage_is_pg { "checked" } else { "" });
                                    div {
                                        span class="choice-title" { "libsql (SQLite)" }
                                        span class="choice-desc" { "Embedded local database file. Zero setup, fast, and lightweight." }
                                    }
                                }
                            }
                            label class="choice-card" x-bind:class="{ 'selected': storage_backend === 'postgres' }" {
                                div class="flex items-start gap-3" {
                                    input type="radio" name="storage_backend" value="postgres" x-model="storage_backend" class="radio radio-sm mt-0.5" checked=(if storage_is_pg { "checked" } else { "" });
                                    div {
                                        span class="choice-title" { "PostgreSQL" }
                                        span class="choice-desc" { "External Postgres database with pgvector for high concurrency." }
                                    }
                                }
                            }
                        }
                        fieldset class="fieldset my-4" x-show="storage_backend === 'libsql'" {
                            label class="label mb-1" { span class="font-bold text-xs text-base-content" { "Database File Path" } }
                            input type="text" name="libsql_path" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                placeholder="./memayu.db" value=(a.libsql_path);
                        }
                        fieldset class="fieldset my-4" x-show="storage_backend === 'postgres'" {
                            label class="label mb-1" { span class="font-bold text-xs text-base-content" { "PostgreSQL Connection URL" } }
                            input type="text" name="database_url" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                placeholder="postgres://user:pass@host:5432/memayu" value=(a.database_url);
                        }
                    }

                    // ── Step 2: Embedder ──
                    div x-show="current_step === 2" class="my-4" {
                        div class="mb-4 text-left" {
                            h3 class="text-base font-bold text-base-content" { "Embedding Model" }
                            p class="text-xs text-base-content/60 mt-0.5" { "Choose how text content is converted into vector representations for similarity search." }
                        }
                        div class="grid grid-cols-1 sm:grid-cols-2 gap-4 my-4" {
                            label class="choice-card" x-bind:class="{ 'selected': embedder_backend === 'local' }" {
                                div class="flex items-start gap-3" {
                                    input type="radio" name="embedder_backend" value="local" x-model="embedder_backend" class="radio radio-sm mt-0.5" checked=(if !emb_is_remote { "checked" } else { "" });
                                    div {
                                        span class="choice-title" { "Local Embedder" }
                                        span class="choice-desc" { "Runs on-device, 100% private, zero API keys needed." }
                                    }
                                }
                            }
                            label class="choice-card" x-bind:class="{ 'selected': embedder_backend === 'remote' }" {
                                div class="flex items-start gap-3" {
                                    input type="radio" name="embedder_backend" value="remote" x-model="embedder_backend" class="radio radio-sm mt-0.5" checked=(if emb_is_remote { "checked" } else { "" });
                                    div {
                                        span class="choice-title" { "Remote API Endpoint" }
                                        span class="choice-desc" { "Use OpenAI or custom OpenAI-compatible embedding endpoint." }
                                    }
                                }
                            }
                        }
                        div x-show="embedder_backend === 'local'" class="my-4 space-y-3" {
                            fieldset class="fieldset" {
                                label class="label mb-1" { span class="font-bold text-xs text-base-content" { "Local Model Architecture" } }
                                div class="dropdown w-full" x-on:focusout="if (!$el.contains($event.relatedTarget)) dropdown_open = false" {
                                    button type="button"
                                        class="model-dropdown-trigger font-normal"
                                        tabindex="0"
                                        x-on:click="dropdown_open = !dropdown_open" {
                                        div class="flex items-center gap-2 flex-1 min-w-0" {
                                            span class="font-bold text-sm truncate" x-text="local_model_name" { (initial_model_name) }
                                            span class="badge badge-sm badge-outline font-mono text-xs" x-text="local_dim + 'd'" { (initial_dim) "d" }
                                            span class="text-xs opacity-60 hidden sm:inline" x-text="'~' + local_int8 + ' MB (q8)'" { "~" (initial_int8) " MB (q8)" }
                                        }
                                        svg xmlns="http://www.w3.org/2000/svg" class="h-4 w-4 opacity-60 ml-2 shrink-0 transition-transform" x-bind:class="{ 'rotate-180': dropdown_open }" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                                            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" {}
                                        }
                                    }
                                    input type="hidden" name="local_model" x-bind:value="local_model" value=(initial_model_id);
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
                                                        span class="badge badge-sm font-mono text-xs font-semibold" { (m.dim) "d" }
                                                        span class="badge badge-sm badge-outline text-xs" { (m.langs) }
                                                    }
                                                }
                                                div class="grid grid-cols-3 gap-2 text-xs text-base-content/70 mt-1.5 pt-1.5 border-t border-base-300/40" {
                                                    span { "💾 ~" (m.int8_size_mb) " MB (q8) / ~" (m.fp32_size_mb) " MB" }
                                                    span { "🧠 Min ~" (m.min_ram_mb) " MB RAM" }
                                                    span class="text-right" { "⚡ " (m.cpu_notes) }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            div class="flex items-center gap-2 text-xs text-base-content/60 my-3" {
                                span { "Embedding vector dimension:" }
                                span class="badge badge-outline text-xs font-mono font-semibold" x-text="local_dim" { (embedding_dimension(a).unwrap_or(initial_dim as usize)) }
                            }
                        }
                        div x-show="embedder_backend === 'remote'" class="my-4 space-y-3" {
                            fieldset class="fieldset" {
                                label class="label mb-1" { span class="font-bold text-xs text-base-content" { "API Base URL" } }
                                input type="text" name="embedder_base_url" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                    placeholder="https://api.openai.com/v1" value=(a.embedder_base_url);
                            }
                            fieldset class="fieldset" {
                                label class="label mb-1" { span class="font-bold text-xs text-base-content" { "API Key (Optional)" } }
                                input type="password" name="embedder_api_key" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                    placeholder="sk-..." value=(a.embedder_api_key);
                            }
                            fieldset class="fieldset" {
                                label class="label mb-1" { span class="font-bold text-xs text-base-content" { "Model Name" } }
                                input type="text" name="embedder_model" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                    placeholder="text-embedding-3-small" value=(a.embedder_model);
                            }
                        }
                    }

                    // ── Step 3: Extraction ──
                    div x-show="current_step === 3" class="my-4" {
                        div class="mb-4 text-left" {
                            h3 class="text-base font-bold text-base-content" { "Memory Extraction Mode" }
                            p class="text-xs text-base-content/60 mt-0.5" { "Choose how memories are extracted from raw documents and conversations." }
                        }
                        div class="grid grid-cols-1 sm:grid-cols-2 gap-4 my-4" {
                            label class="choice-card" x-bind:class="{ 'selected': extraction_mode === 'raw' }" {
                                div class="flex items-start gap-3" {
                                    input type="radio" name="extraction_mode" value="raw" x-model="extraction_mode" class="radio radio-sm mt-0.5" checked=(if !extraction_is_llm { "checked" } else { "" });
                                    div {
                                        span class="choice-title" { "Raw Text Mode" }
                                        span class="choice-desc" { "Directly stores input text chunks without running an LLM pipeline." }
                                    }
                                }
                            }
                            label class="choice-card" x-bind:class="{ 'selected': extraction_mode === 'llm' }" {
                                div class="flex items-start gap-3" {
                                    input type="radio" name="extraction_mode" value="llm" x-model="extraction_mode" class="radio radio-sm mt-0.5" checked=(if extraction_is_llm { "checked" } else { "" });
                                    div {
                                        span class="choice-title" { "LLM Extraction" }
                                        span class="choice-desc" { "Extracts structured facts, entities, and deduplicated insights." }
                                    }
                                }
                            }
                        }
                        div x-show="extraction_mode === 'llm'" class="my-4 space-y-3" {
                            fieldset class="fieldset" {
                                label class="label mb-1" { span class="font-bold text-xs text-base-content" { "LLM Base URL" } }
                                input type="text" name="llm_base_url" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                    placeholder="https://api.openai.com/v1" value=(a.llm_base_url);
                            }
                            fieldset class="fieldset" {
                                label class="label mb-1" { span class="font-bold text-xs text-base-content" { "LLM API Key (Optional)" } }
                                input type="password" name="llm_api_key" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                    placeholder="sk-..." value=(a.llm_api_key);
                            }
                            fieldset class="fieldset" {
                                label class="label mb-1" { span class="font-bold text-xs text-base-content" { "LLM Model" } }
                                input type="text" name="llm_model" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                    placeholder="gpt-4o-mini" value=(a.llm_model);
                            }
                        }
                    }

                    // ── Step 4: Server ──
                    div x-show="current_step === 4" class="my-4" {
                        div class="mb-4 text-left" {
                            h3 class="text-base font-bold text-base-content" { "Server & Network" }
                            p class="text-xs text-base-content/60 mt-0.5" { "Configure local network binding and default API credentials." }
                        }
                        div class="grid grid-cols-1 sm:grid-cols-2 gap-4 my-4" {
                            fieldset class="fieldset" {
                                label class="label mb-1" { span class="font-bold text-xs text-base-content" { "Bind Address" } }
                                input type="text" name="bind_addr" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                    placeholder="127.0.0.1" value=(a.bind_addr);
                            }
                            fieldset class="fieldset" {
                                label class="label mb-1" { span class="font-bold text-xs text-base-content" { "HTTP Port" } }
                                input type="number" name="port" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                    placeholder="18080" value=(a.port.to_string());
                            }
                        }
                        fieldset class="fieldset my-4" {
                            label class="label mb-1" { span class="font-bold text-xs text-base-content" { "Initial API Key Label" } }
                            input type="text" name="api_key_label" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                placeholder="default" value=(a.api_key_label);
                            span class="text-xs text-base-content/50 mt-1" { "A primary API key with this label will be generated upon completion." }
                        }
                    }

                    // ── Step 5: Admin Account ──
                    div x-show="current_step === 5" class="my-4" {
                        div class="mb-4 text-left" {
                            h3 class="text-base font-bold text-base-content" { "Admin Account" }
                            p class="text-xs text-base-content/60 mt-0.5" { "Create master administrator credentials for accessing the Memayu dashboard." }
                        }
                        fieldset class="fieldset my-4" {
                            label class="label mb-1" { span class="font-bold text-xs text-base-content" { "Admin Email" } }
                            input type="email" name="email" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                placeholder="admin@example.com" value=(a.admin_email) required;
                        }
                        fieldset class="fieldset my-4" {
                            label class="label mb-1" { span class="font-bold text-xs text-base-content" { "Admin Password" } }
                            input type="password" name="password" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                placeholder="Min 8 chars, uppercase, lowercase, number" required;
                        }
                        fieldset class="fieldset my-4" {
                            label class="label mb-1" { span class="font-bold text-xs text-base-content" { "Confirm Password" } }
                            input type="password" name="confirm" class="input w-full border border-base-300 rounded-md py-2 px-3 text-sm bg-base-100"
                                placeholder="Re-enter password" required;
                        }
                    }
                    }

                    // ── Navigation Buttons ──
                    div class="wizard-footer flex items-center justify-between mt-auto" {
                        div {
                            button type="button" class="btn btn-ghost btn-sm"
                                   x-show="current_step > 1"
                                   x-on:click="current_step--" {
                                "‹ Back"
                            }
                        }
                        div class="text-xs font-semibold text-base-content/60" x-text="'Step ' + current_step + ' of 5'" {}
                        div {
                            button type="button" class="btn btn-primary btn-sm"
                                   x-show="current_step < 5"
                                   x-on:click="current_step++" {
                                "Next ›"
                            }
                            button type="submit" class="btn btn-primary btn-sm"
                                   x-show="current_step === 5" {
                                "Finish Setup ›"
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── DTOs ──

#[derive(serde::Deserialize)]
pub struct SetupForm {
    pub storage_backend: Option<String>,
    pub libsql_path: Option<String>,
    pub database_url: Option<String>,
    pub embedder_backend: Option<String>,
    pub local_model: Option<String>,
    pub embedder_base_url: Option<String>,
    pub embedder_api_key: Option<String>,
    pub embedder_model: Option<String>,
    pub extraction_mode: Option<String>,
    pub llm_base_url: Option<String>,
    pub llm_api_key: Option<String>,
    pub llm_model: Option<String>,
    pub email: String,
    pub password: String,
    pub confirm: String,
    pub bind_addr: Option<String>,
    pub port: Option<String>,
    pub api_key_label: Option<String>,
}

// Maps the submitted form into setup answers. The many conditional
// overrides read better as assignments than as a giant struct literal.
#[allow(clippy::field_reassign_with_default)]
fn build_answers(form: &SetupForm) -> SetupAnswers {
    let mut a = SetupAnswers::default();
    a.storage_backend = if form.storage_backend.as_deref() == Some("postgres") {
        memayu_config::StorageBackend::Postgres
    } else {
        memayu_config::StorageBackend::Libsql
    };
    if !form.libsql_path.as_deref().unwrap_or("").is_empty() {
        a.libsql_path = form.libsql_path.clone().unwrap_or_default();
    }
    if !form.database_url.as_deref().unwrap_or("").is_empty() {
        a.database_url = form.database_url.clone().unwrap_or_default();
    }

    let emb_backend = form.embedder_backend.as_deref().unwrap_or("local");
    a.embedder_backend = emb_backend.to_string();
    if emb_backend == "local" {
        // Local model id chosen from the catalog.
        if let Some(m) = form.local_model.as_deref() {
            if !m.is_empty() {
                a.embedder_model = m.to_string();
            }
        }
    } else {
        if let Some(u) = form.embedder_base_url.as_deref() {
            if !u.is_empty() {
                a.embedder_base_url = u.to_string();
            }
        }
        if let Some(k) = form.embedder_api_key.as_deref() {
            if !k.is_empty() {
                a.embedder_api_key = k.to_string();
            }
        }
        if let Some(m) = form.embedder_model.as_deref() {
            if !m.is_empty() {
                a.embedder_model = m.to_string();
            }
        }
    }

    a.extraction_mode = form.extraction_mode.as_deref().unwrap_or("llm").to_string();
    if a.extraction_mode == "llm" {
        if let Some(u) = form.llm_base_url.as_deref() {
            if !u.is_empty() {
                a.llm_base_url = u.to_string();
            }
        }
        if let Some(k) = form.llm_api_key.as_deref() {
            if !k.is_empty() {
                a.llm_api_key = k.to_string();
            }
        }
        if let Some(m) = form.llm_model.as_deref() {
            if !m.is_empty() {
                a.llm_model = m.to_string();
            }
        }
    }

    a.admin_email = form.email.clone();
    a.admin_password = form.password.clone();

    if let Some(b) = form.bind_addr.as_deref() {
        if !b.is_empty() {
            a.bind_addr = b.to_string();
        }
    }
    if let Some(p) = form.port.as_deref() {
        if let Ok(num) = p.parse::<u16>() {
            a.port = num;
        }
    }
    if let Some(l) = form.api_key_label.as_deref() {
        if !l.is_empty() {
            a.api_key_label = l.to_string();
        }
    }

    a
}

// ── Handlers ──

pub async fn get_setup(
    State(services): State<WebServices>,
) -> Result<Html<String>, (StatusCode, String)> {
    let empty = services
        .auth_users_empty()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if !empty {
        return Ok(Html(base_page(
            "Setup",
            maud::html! {
                div class="card bg-base-100 shadow-xl" {
                    div class="card-body text-center" {
                        h2 class="card-title justify-center" { "Already Set Up" }
                        p { "Setup has already been completed." }
                        a href="/login" class="btn btn-primary mt-4" { "Go to Login" }
                    }
                }
            },
        )));
    }

    // Prefill defaults from an existing config file (if any), mirroring the
    // CLI/TUI reconfiguration flow.
    let existing = read_config_file_if_any();
    let answers = preseed(existing.as_ref());
    Ok(Html(base_page("Setup", setup_form(&answers, None))))
}

pub async fn post_setup(
    State(services): State<WebServices>,
    Form(form): Form<SetupForm>,
) -> Result<Response, (StatusCode, String)> {
    if form.password != form.confirm {
        return Ok(Html(base_page(
            "Setup",
            setup_form(&preseed(None), Some("Passwords do not match.")),
        ))
        .into_response());
    }

    let answers = build_answers(&form);

    // Create the admin account in the server's own database and capture a web
    // session token. The running server's storage/registry are fixed at launch,
    // so (like the CLI) the embedder/extraction choices take effect on restart;
    // here we only surface them for the wizard and mirror the admin/API-key flow.
    let req = memayu_api::auth_dto::SetupRequest {
        email: answers.admin_email.clone(),
        password: answers.admin_password.clone(),
        confirm: answers.admin_password.clone(),
    };
    let (_, token) = match services.auth_setup(&req).await {
        Ok(v) => v,
        Err(e) => {
            let message = if e.status == 409 {
                "Setup already completed.".to_string()
            } else {
                e.message
            };
            return Ok(Html(base_page(
                "Setup",
                setup_form(&preseed(None), Some(&message)),
            ))
            .into_response());
        }
    };

    // Generate an API key for the freshly-created admin to show exactly once.
    let api_key = match services.auth_resolve_session_with_email(&token).await {
        Ok((user_id, _)) => {
            let req = memayu_api::api_key_dto::GenerateKeyRequest {
                label: answers.api_key_label.clone(),
            };
            services
                .api_keys_generate(&user_id, &req)
                .await
                .map(|r| r.key)
                .unwrap_or_default()
        }
        Err(_) => String::new(),
    };

    let redirect_url = if api_key.is_empty() {
        "/home".to_string()
    } else {
        format!("/home?new_key={api_key}")
    };

    let mut resp = axum::response::Redirect::to(&redirect_url).into_response();
    resp.headers_mut().insert(
        SET_COOKIE,
        format!(
            "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_DURATION_SECS}",
        )
        .parse()
        .unwrap(),
    );
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_form() -> SetupForm {
        SetupForm {
            storage_backend: Some("libsql".into()),
            libsql_path: Some("./memayu.db".into()),
            database_url: None,
            embedder_backend: Some("local".into()),
            local_model: Some("nomic-embed-text-v1.5".into()),
            embedder_base_url: None,
            embedder_api_key: None,
            embedder_model: None,
            extraction_mode: Some("raw".into()),
            llm_base_url: None,
            llm_api_key: None,
            llm_model: None,
            email: "admin@example.com".into(),
            password: "Password1!".into(),
            confirm: "Password1!".into(),
            bind_addr: Some("0.0.0.0".into()),
            port: Some("9090".into()),
            api_key_label: Some("web".into()),
        }
    }

    #[test]
    fn local_embedder_uses_catalog_model_id() {
        let a = build_answers(&base_form());
        assert_eq!(a.embedder_backend, "local");
        assert_eq!(a.embedder_model, "nomic-embed-text-v1.5");
        // local backend keeps the default base URL but no API key is set
        assert_eq!(a.embedder_base_url, "https://api.openai.com/v1");
        assert!(a.embedder_api_key.is_empty());
    }

    #[test]
    fn remote_embedder_uses_remote_fields() {
        let mut f = base_form();
        f.embedder_backend = Some("remote".into());
        f.embedder_base_url = Some("https://api.example.com/v1".into());
        f.embedder_api_key = Some("sk-abc".into());
        f.embedder_model = Some("text-embedding-3-small".into());
        let a = build_answers(&f);
        assert_eq!(a.embedder_backend, "remote");
        assert_eq!(a.embedder_model, "text-embedding-3-small");
        assert_eq!(a.embedder_base_url, "https://api.example.com/v1");
        assert_eq!(a.embedder_api_key, "sk-abc");
    }

    #[test]
    fn llm_extraction_populates_llm_fields() {
        let mut f = base_form();
        f.extraction_mode = Some("llm".into());
        f.llm_base_url = Some("https://api.example.com/v1".into());
        f.llm_api_key = Some("sk-llm".into());
        f.llm_model = Some("gpt-4".into());
        let a = build_answers(&f);
        assert_eq!(a.extraction_mode, "llm");
        assert_eq!(a.llm_base_url, "https://api.example.com/v1");
        assert_eq!(a.llm_api_key, "sk-llm");
        assert_eq!(a.llm_model, "gpt-4");
    }

    #[test]
    fn raw_extraction_ignores_llm_fields() {
        let mut f = base_form();
        f.extraction_mode = Some("raw".into());
        f.llm_base_url = Some("https://api.example.com/v1".into());
        let a = build_answers(&f);
        assert_eq!(a.extraction_mode, "raw");
        // raw mode must ignore the submitted LLM base URL and keep the default
        assert_eq!(a.llm_base_url, "https://api.openai.com/v1");
    }

    #[test]
    fn postgres_storage_and_server_settings() {
        let mut f = base_form();
        f.storage_backend = Some("postgres".into());
        f.database_url = Some("postgres://u:p@host/memayu".into());
        let a = build_answers(&f);
        assert_eq!(a.storage_backend, memayu_config::StorageBackend::Postgres);
        assert_eq!(a.database_url, "postgres://u:p@host/memayu");
        assert_eq!(a.port, 9090);
        assert_eq!(a.bind_addr, "0.0.0.0");
        assert_eq!(a.api_key_label, "web");
    }

    #[test]
    fn setup_form_renders_steps_and_options() {
        let answers = SetupAnswers::default();
        let markup = setup_form(&answers, None).into_string();

        assert!(markup.contains("name=\"local_model\""));
        assert!(markup.contains("all-MiniLM-L6-v2"));
        assert!(markup.contains("bge-small-en-v1.5"));
        assert!(markup.contains("paraphrase-multilingual-MiniLM-L12-v2"));
        assert!(markup.contains("nomic-embed-text-v1.5"));

        // Rich model specs in dropdown
        assert!(markup.contains("model-dropdown-trigger"));
        assert!(markup.contains("model-option-item"));
        assert!(markup.contains("Embedding vector dimension:"));

        // All 5 steps present
        assert!(markup.contains("Storage Configuration"));
        assert!(markup.contains("Embedding Model"));
        assert!(markup.contains("Memory Extraction Mode"));
        assert!(markup.contains("Server &amp; Network") || markup.contains("Server"));
        assert!(markup.contains("Admin Account"));

        // Stepper elements
        assert!(markup.contains("stepper"));
        assert!(markup.contains("step-item"));
        assert!(markup.contains("step-circle"));

        // Wizard stability classes
        assert!(markup.contains("wizard-card"));
        assert!(markup.contains("wizard-content"));
        assert!(markup.contains("wizard-footer"));

        // Button bar
        assert!(markup.contains("Next"));
        assert!(markup.contains("Finish Setup"));
    }
}
