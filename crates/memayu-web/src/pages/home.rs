use crate::auth::CurrentUser;
use crate::components;
use axum::extract::State;
use axum::response::Html;
use axum::Form;
use memayu_api::MemoryService;
use std::sync::Arc;

#[derive(serde::Deserialize)]
pub struct SearchForm {
    pub query: String,
}

pub async fn get_home(
    user: CurrentUser,
    State(service): State<Arc<MemoryService>>,
) -> Result<Html<String>, (axum::http::StatusCode, String)> {
    let memories = service.list_memories(&user.id, 100).await.map_err(|e| {
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
            // Memory list
            @if memories.is_empty() {
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
                    table class="table table-zebra table-sm" {
                        thead {
                            tr {
                                th { "Content" }
                                th { "Created" }
                            }
                        }
                        tbody {
                            @for mem in &memories {
                                tr {
                                    td class="max-w-xl" {
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
        },
    )))
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
                    table class="table table-zebra table-sm" {
                        thead {
                            tr {
                                th { "Content" }
                                @if has_scores {
                                    th class="w-24" { "Score" }
                                }
                                th { "Created" }
                            }
                        }
                        tbody {
                            @for r in &results {
                                tr {
                                    td class="max-w-xl" {
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
            }
        },
    )))
}

struct SearchResultRow {
    content: String,
    created_at: chrono::DateTime<chrono::Utc>,
    score: Option<f32>,
}
