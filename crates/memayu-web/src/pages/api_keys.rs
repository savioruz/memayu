use crate::auth::CurrentUser;
use crate::components;
use axum::extract::Path;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use maud::{html, PreEscaped};
use memayu_api::api_key_dto::GenerateKeyRequest;
use memayu_api::{ApiKey, WebServices};
use serde::Deserialize;

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub async fn get_api_keys(
    State(services): State<WebServices>,
    user: CurrentUser,
) -> Result<Html<String>, (StatusCode, String)> {
    let keys = services
        .api_keys_list()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let keys_table = render_keys_table(&keys);

    let markup = html! {
        div {
            div class="mb-4 flex items-center gap-3" {
                button
                    class="btn btn-primary"
                    onclick="document.getElementById('key-gen-modal').showModal()" {
                    svg xmlns="http://www.w3.org/2000/svg" class="h-4 w-4 mr-2" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                        path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" {}
                    }
                    "Generate new key"
                }
            }

            // Delete confirmation modal
            dialog class="modal" id="delete-key-modal" {
                div class="modal-box max-w-sm" {
                    h3 class="text-lg font-bold" { "Delete API key?" }
                    p class="py-4 text-sm text-base-content/70" {
                        "This will permanently remove the key "
                        strong id="delete-key-name" {}
                        ". All requests using it will be rejected."
                    }
                    form
                        x-data=""
                        x-on:submit="
                            $event.preventDefault();
                            var btn = document.getElementById('delete-confirm-btn');
                            if (!btn.disabled) btn.click();
                        " {
                        p class="text-sm font-medium mt-4 mb-2" { "Type the label to confirm:" }
                        input
                            type="text"
                            id="delete-confirm-input"
                            class="input input-bordered w-full"
                            placeholder="Label name"
                            onkeyup="
                                var btn = document.getElementById('delete-confirm-btn');
                                btn.disabled = this.value !== document.getElementById('delete-key-modal').dataset.keyLabel;
                            ";
                        div class="modal-action mt-4" {
                            button
                                type="button"
                                class="btn btn-ghost btn-sm"
                                onclick="document.getElementById('delete-key-modal').close()" { "Cancel" }
                            button
                                type="button"
                                class="btn btn-error btn-sm"
                                id="delete-confirm-btn"
                                disabled
                                onclick="
                                    var d = document.getElementById('delete-key-modal');
                                    htmx.ajax('POST', '/api-keys/' + d.dataset.keyId + '/delete', {target: '#keys-table-container', swap: 'outerHTML'});
                                    d.close();
                                " { "Delete" }
                        }
                    }
                }
                form method="dialog" class="modal-backdrop" {
                    button { "close" }
                }
            }

            // Generate key modal
            dialog class="modal" id="key-gen-modal" {
                div class="modal-box max-w-md" {
                    h3 class="text-lg font-bold" { "Generate API Key" }
                    p class="py-2 text-sm text-base-content/70" { "Give it a label so you can remember what it's for." }
                    form
                        x-data="{ label: '' }"
                        x-on:submit="
                            $event.preventDefault();
                            if (label.trim()) {
                                htmx.ajax('POST', '/api-keys/generate?label=' + encodeURIComponent(label.trim()), {target: $el.closest('.modal-box'), swap: 'innerHTML'});
                            }
                        " {
                        div class="form-control mt-4" {
                            label class="label mb-2" { span { "Label Name" } }
                            input
                                type="text"
                                name="label"
                                class="input w-full"
                                placeholder="Production"
                                x-model="label"
                                required;
                        }
                        div class="modal-action mt-6" {
                            button
                                type="button"
                                class="btn btn-ghost btn-sm"
                                onclick="document.getElementById('key-gen-modal').close()" { "Cancel" }
                            button type="submit" class="btn btn-primary btn-sm" { "Generate" }
                        }
                    }
                }
                form method="dialog" class="modal-backdrop" {
                    button { "close" }
                }
            }

            (keys_table)
        }
    };
    let shell = components::render_page(
        "api-keys",
        Some(&user.email),
        "API Keys",
        "API Keys",
        markup,
    );
    Ok(Html(shell))
}

#[derive(Deserialize)]
pub struct GenerateQuery {
    label: String,
}

pub async fn post_generate_key(
    State(services): State<WebServices>,
    Query(query): Query<GenerateQuery>,
    user: CurrentUser,
) -> Result<Html<String>, (StatusCode, String)> {
    let label = query.label.trim();
    if label.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "label is required".into()));
    }

    let req = GenerateKeyRequest {
        label: label.to_string(),
    };
    let resp = services
        .api_keys_generate(&user.id, &req)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.message))?;

    let _ = services
        .request_log_insert("POST", "/api-keys/generate", 200, 0.0, "Session")
        .await;

    Ok(Html(
        html! {
            div {
                h3 class="text-lg font-bold" { "Key created: " }
                div class="mt-4" {
                    input
                        type="text"
                        readonly
                        value=(resp.key)
                        class="input input-bordered w-full font-mono text-xs select-all bg-base-200"
                        x-data=""
                        x-on:click="
                            $el.select();
                            navigator.clipboard.writeText($el.value);
                        ";
                }
                p class="text-xs text-base-content/50 mt-2" {
                    "Click the key to copy. It won't be shown again."
                }
                div class="modal-action mt-6" {
                    button
                        type="button"
                        class="btn btn-primary btn-sm"
                        onclick="
                            document.getElementById('key-gen-modal').close();
                            location.reload();
                        " { "Done" }
                }
            }
        }
        .into_string(),
    ))
}

pub async fn delete_api_key(
    State(services): State<WebServices>,
    Path(id): Path<String>,
    _user: CurrentUser,
) -> Result<Html<String>, (StatusCode, String)> {
    let _ = services
        .api_keys_delete(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let _ = services
        .request_log_insert("POST", "/api-keys/{id}/delete", 200, 0.0, "Session")
        .await;

    let keys = services
        .api_keys_list()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let markup = render_keys_table(&keys);
    Ok(Html(markup.into_string()))
}

fn render_keys_table(keys: &[ApiKey]) -> maud::Markup {
    let inner = if keys.is_empty() {
        html! {
            div class="card bg-base-100 shadow-sm" {
                div class="card-body text-center py-12" {
                    div class="mx-auto mb-4" {
                        svg xmlns="http://www.w3.org/2000/svg" class="h-12 w-12 text-base-content/30 mx-auto" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                            path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" {}
                        }
                    }
                    h3 class="text-lg font-semibold text-base-content/70" { "No API keys yet" }
                    p class="text-sm text-base-content/50 mt-1" { "Generate one above to get started." }
                }
            }
        }
    } else {
        let mut rows = String::new();
        for k in keys {
            let last_used = k.last_used_at.clone().unwrap_or_else(|| "never".into());
            let created = &k.created_at;
            let created_fmt = chrono::DateTime::parse_from_rfc3339(created)
                .map(|dt| dt.format("%B %d, %Y").to_string())
                .unwrap_or_else(|_| {
                    if created.len() >= 10 {
                        created[..10].to_string()
                    } else {
                        created.clone()
                    }
                });
            rows.push_str(&format!(
                r#"<tr class="border-b border-base-300 hover:bg-base-200" data-key-id="{}" data-key-label="{}">
                    <td class="text-sm font-medium">{}</td>
                    <td><code class="text-xs bg-base-300 px-2 py-1">{}....</code></td>
                    <td class="text-sm text-base-content/70">{}</td>
                    <td class="text-sm text-base-content/70">{}</td>
                    <td>
                        <button class="btn btn-ghost btn-sm text-error" onclick="
                            var tr=this.closest('tr');
                            var d=document.getElementById('delete-key-modal');
                            d.dataset.keyId=tr.dataset.keyId;
                            d.dataset.keyLabel=tr.dataset.keyLabel;
                            document.getElementById('delete-key-name').textContent=tr.dataset.keyLabel;
                            document.getElementById('delete-confirm-input').value='';
                            document.getElementById('delete-confirm-btn').disabled=true;
                            d.showModal();
                        ">
                            <svg xmlns="http://www.w3.org/2000/svg" class="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/>
                            </svg>
                        </button>
                    </td>
                </tr>"#,
                k.id,
                html_escape(&k.label),
                html_escape(&k.label),
                k.key_prefix,
                last_used,
                created_fmt,
            ));
        }
        html! {
            div class="card bg-base-100 shadow-sm overflow-hidden" {
                div class="overflow-x-auto" {
                    table class="table table-zebra" {
                        thead {
                            tr class="border-b border-base-300" {
                                th class="text-xs uppercase tracking-wider text-base-content/60 font-medium" { "Label" }
                                th class="text-xs uppercase tracking-wider text-base-content/60 font-medium" { "Key" }
                                th class="text-xs uppercase tracking-wider text-base-content/60 font-medium" { "Last used" }
                                th class="text-xs uppercase tracking-wider text-base-content/60 font-medium" { "Created" }
                                th class="text-xs uppercase tracking-wider text-base-content/60 font-medium w-0" {}
                            }
                        }
                        tbody {
                            (PreEscaped(rows))
                        }
                    }
                }
            }
        }
    };

    html! {
        div id="keys-table-container" {
            (inner)
        }
    }
}
