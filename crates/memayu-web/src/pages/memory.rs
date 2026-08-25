//! Memory detail view for the dashboard. Returns an HTMX modal fragment so the
//! full, untruncated content (plus metadata) can be read even for multi-KB
//! memories — the list rows only ever render a `truncate` snippet (issue #56).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Html;
use memayu_core::Memory;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Render a single metadata entry as a small key/value badge.
fn meta_badge(key: &str, value: &str) -> maud::Markup {
    maud::html! {
        div class="badge badge-outline badge-sm gap-1 py-3 font-mono" {
            span class="text-base-content/50" { (key) ": " }
            span { (value) }
        }
    }
}

/// `GET /home/memory/{id}` — modal fragment showing the full memory.
///
/// The list row only shows a truncated snippet; this endpoint returns a dialog
/// fragment with the complete content in a scrollable region plus created_at,
/// updated_at, and metadata badges.
pub async fn get_memory_detail(
    user: crate::auth::CurrentUser,
    State(service): State<Arc<memayu_core::MemoryService>>,
    Path(id): Path<String>,
) -> Result<Html<String>, (StatusCode, String)> {
    match service.get_memory(&id).await {
        Ok(mem) if mem.user_id == user.id => Ok(Html(detail_modal(&mem).into_string())),
        Ok(_) | Err(_) => Err((StatusCode::NOT_FOUND, format!("memory {id} not found"))),
    }
}

/// Assemble the modal markup for a memory detail.
fn detail_modal(mem: &Memory) -> maud::Markup {
    // Sort metadata keys deterministically so tags render in a stable order.
    let sorted_meta: BTreeMap<&String, &String> = mem.metadata.iter().collect();
    let created = mem.created_at.format("%b %d, %Y  %H:%M").to_string();
    let updated = mem.updated_at.format("%b %d, %Y  %H:%M").to_string();
    let modal_id = format!("memory-detail-{}", mem.id);

    maud::html! {
        dialog id=(modal_id) class="modal" {
            div class="modal-box max-w-3xl w-full max-h-[85vh] flex flex-col" {
                // Header
                div class="flex items-start justify-between gap-4" {
                    h3 class="text-lg font-bold break-words" { "Memory detail" }
                    button type="button" class="btn btn-ghost btn-sm btn-circle"
                            onclick=(format!("document.getElementById('{modal_id}').close();")) {
                        "✕"
                    }
                }

                // Full, scrollable content — never clipped.
                div class="mt-4 overflow-y-auto grow pr-2 rounded-lg bg-base-200/50 p-4"
                    style="max-height:50vh; white-space:pre-wrap; word-break:break-word;" {
                    p class="text-sm leading-relaxed whitespace-pre-wrap" { (mem.content) }
                }

                // Metadata
                div class="mt-4 pt-3 border-t border-base-300" {
                    div class="flex flex-wrap gap-2 items-center" {
                        span class="text-xs text-base-content/60" { "Created:" }
                        span class="text-xs font-mono" { (created) }
                        span class="text-xs text-base-content/40 mx-1" { "·" }
                        span class="text-xs text-base-content/60" { "Updated:" }
                        span class="text-xs font-mono" { (updated) }
                    }
                    @if sorted_meta.is_empty() {
                        div class="mt-2" {
                            span class="text-xs text-base-content/40" { "No metadata" }
                        }
                    } @else {
                        div class="flex flex-wrap gap-2 items-center mt-2" {
                            span class="text-xs text-base-content/60" { "Metadata:" }
                            @for (k, v) in &sorted_meta {
                                (meta_badge(k, v))
                            }
                        }
                    }
                }
            }
            form method="dialog" class="modal-backdrop" {
                button type="button" onclick=(format!("document.getElementById('{modal_id}').close();")) {
                    "close"
                }
            }
        }
        // Runs after HTMX swaps the fragment in; the dialog is present by then.
        script {
            (maud::PreEscaped(format!(
                "var m = document.getElementById('{modal_id}'); if (m && typeof m.showModal === 'function') m.showModal();"
            )))
        }
    }
}
