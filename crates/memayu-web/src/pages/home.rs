use crate::auth::CurrentUser;
use crate::components;
use axum::extract::{Query, State};
use axum::response::Html;
use axum::Form;
use memayu_core::{MemoryPage, MemoryService};
use std::sync::Arc;

/// Number of memories shown per page in the dashboard.
const LIST_PAGE_SIZE: usize = 10;

#[derive(serde::Deserialize)]
pub struct SearchForm {
    pub query: String,
}

#[derive(serde::Deserialize)]
pub struct ListCursorQuery {
    pub cursor: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub struct HomeQuery {
    pub new_key: Option<String>,
}

pub async fn get_home(
    user: CurrentUser,
    State(service): State<Arc<MemoryService>>,
    Query(query): Query<HomeQuery>,
) -> Result<Html<String>, (axum::http::StatusCode, String)> {
    let page = service
        .list_memories_paged(&user.id, LIST_PAGE_SIZE, None, None)
        .await
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("{}", e),
            )
        })?;

    Ok(Html(components::render_page(
        "home",
        Some(&user.email),
        "Home",
        "Home",
        maud::html! {
            @if let Some(key) = &query.new_key {
                dialog class="modal" id="setup-key-modal" {
                    div class="modal-box max-w-md" {
                        h3 class="text-lg font-bold" { "Key created: default" }
                        div class="mt-4" {
                            input
                                type="text"
                                readonly
                                value=(key)
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
                                    document.getElementById('setup-key-modal').close();
                                    window.history.replaceState({}, '', '/home');
                                " { "Done" }
                        }
                    }
                    form method="dialog" class="modal-backdrop" {
                        button type="button" onclick="document.getElementById('setup-key-modal').close(); window.history.replaceState({}, '', '/home');" { "close" }
                    }
                }
                (maud::PreEscaped(r#"<script>
                    document.addEventListener('DOMContentLoaded', function() {
                        var m = document.getElementById('setup-key-modal');
                        if (m && typeof m.showModal === 'function') {
                            m.showModal();
                        }
                    });
                    if (document.readyState === 'complete' || document.readyState === 'interactive') {
                        var m = document.getElementById('setup-key-modal');
                        if (m && typeof m.showModal === 'function' && !m.open) {
                            m.showModal();
                        }
                    }
                </script>"#))
            }
            div class="mb-4" {
                @let mode = service.extraction_mode();
                @if mode.to_string() == "raw" {
                    span class="badge badge-warning badge-sm gap-1" {
                        "Mode: Raw — automatic conflict detection disabled"
                    }
                } @else {
                    span class="badge badge-outline badge-sm" {
                        (format!("Mode: {}", mode))
                    }
                }
            }
            // Search bar
            form method="post" action="/home/search" class="mb-6" {
                div class="join w-full" {
                    input type="text" name="query" class="input input-bordered join-item flex-1"
                        placeholder="Search memories...";
                    button type="submit" class="btn btn-primary join-item" { "Search" }
                }
            }
            // Memory list (swappable HTMX fragment)
            (list_fragment(&page))
            // HTMX target for the memory-detail modal (loaded on row click).
            div id="memory-detail-target" {}
            // Pager controls (kept outside the swap target): total on the left,
            // Prev/Next on the right.
            div class="flex items-center justify-between gap-2 mt-4 w-full" {
                span id="mem-total" class="text-xs text-base-content/60" {
                    (format!("{} memor{}", page.total, if page.total == 1 { "y" } else { "ies" }))
                }
                div class="flex items-center gap-2" {
                    button id="mem-prev" type="button" class="btn btn-sm join-item" disabled { "‹ Prev" }
                    button id="mem-next" type="button" class="btn btn-sm join-item" { "Next ›" }
                }
            }
            (pager_script())
        },
    )))
}

/// Serve a single memory-list page fragment for the HTMX pager.
pub async fn get_home_list(
    user: CurrentUser,
    State(service): State<Arc<MemoryService>>,
    Query(q): Query<ListCursorQuery>,
) -> Result<Html<String>, (axum::http::StatusCode, String)> {
    let page = service
        .list_memories_paged(&user.id, LIST_PAGE_SIZE, q.cursor.as_deref(), None)
        .await
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("{}", e),
            )
        })?;

    Ok(Html(list_fragment(&page).into_string()))
}

/// The `#memory-list` fragment. Its `data-next-cursor` and `data-total`
/// attributes drive the pager's Next/Prev state.
fn list_fragment(page: &MemoryPage) -> maud::Markup {
    let next = page.next_cursor.clone().unwrap_or_default();
    maud::html! {
        div id="memory-list" data-next-cursor=(next) data-total=(page.total.to_string()) class="mb-4" {
            @if page.memories.is_empty() {
                div class="card bg-base-100 shadow-sm" {
                    div class="card-body text-center py-12" {
                        h2 class="text-lg font-semibold text-base-content/60" { "No memories yet" }
                        p class="text-sm text-base-content/40 mt-1" {
                            "Memories will appear here once you add them via the API."
                        }
                    }
                }
            } @else {
                div class="overflow-x-auto" {
                    table class="table table-fixed table-zebra table-sm" {
                        thead {
                            tr {
                                th { "Content" }
                                th class="w-48" { "Created" }
                            }
                        }
                        tbody {
                            @for mem in &page.memories {
                                tr class="cursor-pointer hover:bg-base-200"
                                    hx-get=(format!("/home/memory/{}", mem.id))
                                    hx-target="#memory-detail-target"
                                    hx-swap="innerHTML"
                                    title="View full memory" {
                                    td {
                                        p class="truncate" { (mem.content) }
                                    }
                                    td class="whitespace-nowrap text-xs text-base-content/60" {
                                        (mem.created_at.format("%b %d, %Y  %H:%M").to_string())
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Inline pager logic. It keeps a stack of the fetch-cursors used to reach each
/// visited page (starting with `null` for the first page) so that Prev can
/// replay them in reverse. `htmx.ajax` swaps the `#memory-list` fragment.
fn pager_script() -> maud::Markup {
    maud::PreEscaped(
        r#"
<script>
(function () {
    var list = document.getElementById('memory-list');
    if (!list) return;
    var prevBtn = document.getElementById('mem-prev');
    var nextBtn = document.getElementById('mem-next');
    var totalEl = document.getElementById('mem-total');
    var history = [null]; // fetch-cursor of each page visited, current page last

    function nextCursor() { return list.getAttribute('data-next-cursor') || ''; }

    function update() {
        var hasNext = nextCursor() !== '';
        nextBtn.disabled = !hasNext;
        prevBtn.disabled = history.length < 2;
        var total = list.getAttribute('data-total');
        if (totalEl && total !== null && total !== '') {
            totalEl.textContent = total + ' Memor' + (total === '1' ? 'y' : 'ies');
        }
    }

    function load(url) {
        htmx.ajax('GET', url, { target: '#memory-list', swap: 'outerHTML' });
    }

    nextBtn.addEventListener('click', function () {
        var nc = nextCursor();
        if (!nc) return;
        history.push(nc);
        load('/home/list?cursor=' + encodeURIComponent(nc));
    });

    prevBtn.addEventListener('click', function () {
        if (history.length < 2) return;
        history.pop();
        var prev = history[history.length - 1];
        load(prev ? '/home/list?cursor=' + encodeURIComponent(prev) : '/home/list');
    });

    document.addEventListener('htmx:afterSwap', function () {
        list = document.getElementById('memory-list');
        update();
    });

    update();
})();
</script>
"#
        .to_string(),
    )
}

pub async fn post_search(
    user: CurrentUser,
    State(service): State<Arc<MemoryService>>,
    Form(form): Form<SearchForm>,
) -> Result<Html<String>, (axum::http::StatusCode, String)> {
    let q = form.query.trim();
    let results: Vec<SearchResultRow> = if q.is_empty() {
        service
            .list_memories(&user.id, 100)
            .await
            .map_err(|e| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("{}", e),
                )
            })?
            .into_iter()
            .map(|m| SearchResultRow {
                id: m.id.clone(),
                content: m.content,
                created_at: m.created_at,
                score: None,
            })
            .collect()
    } else {
        service
            .search_memory(&user.id, q, 20)
            .await
            .map_err(|e| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("{}", e),
                )
            })?
            .into_iter()
            .map(|(m, score)| SearchResultRow {
                id: m.id.clone(),
                content: m.content,
                created_at: m.created_at,
                score: Some(score),
            })
            .collect()
    };

    let has_scores = results.first().map(|r| r.score.is_some()).unwrap_or(false);

    Ok(Html(components::render_page(
        "home",
        Some(&user.email),
        "Home",
        "Search",
        maud::html! {
            form method="post" action="/home/search" class="mb-6" {
                div class="join w-full" {
                    input type="text" name="query" class="input input-bordered join-item flex-1"
                        placeholder="Search memories..." value=(form.query);
                    button type="submit" class="btn btn-primary join-item" { "Search" }
                }
            }
            h2 class="text-sm font-medium text-base-content/60 mb-2" {
                (results.len()) " result(s)"
            }
            @if results.is_empty() {
                p class="text-sm text-base-content/40" { "No matches found." }
            } @else {
                div class="overflow-x-auto" {
                    table class="table table-fixed table-zebra table-sm" {
                        thead {
                            tr {
                                th { "Content" }
                                @if has_scores {
                                    th class="w-24" { "Score" }
                                }
                                th class="w-48" { "Created" }
                            }
                        }
                        tbody {
                            @for r in &results {
                                tr class="cursor-pointer hover:bg-base-200"
                                    hx-get=(format!("/home/memory/{}", r.id))
                                    hx-target="#memory-detail-target"
                                    hx-swap="innerHTML"
                                    title="View full memory" {
                                    td {
                                        p class="truncate" { (r.content) }
                                    }
                                    @if let Some(score) = r.score {
                                        td {
                                            span class="text-xs font-mono" {
                                                (format!("{:.2}", score))
                                            }
                                        }
                                    }
                                    td class="whitespace-nowrap text-xs text-base-content/60" {
                                        (r.created_at.format("%b %d, %Y  %H:%M").to_string())
                                    }
                                }
                            }
                        }
                    }
                }
                // HTMX target for the memory-detail modal (loaded on row click).
                div id="memory-detail-target" {}
            }
        },
    )))
}

struct SearchResultRow {
    id: String,
    content: String,
    created_at: chrono::DateTime<chrono::Utc>,
    score: Option<f32>,
}
