use crate::auth::CurrentUser;
use crate::components;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use chrono::DateTime;
use memayu_api::WebServices;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub page: usize,
}

const PER_PAGE: usize = 25;

pub async fn get_requests(
    user: CurrentUser,
    State(services): State<WebServices>,
    Query(q): Query<PageQuery>,
) -> Result<Html<String>, (StatusCode, String)> {
    let page = q.page.max(1);
    let offset = (page - 1) * PER_PAGE;

    let (total, avg_latency, success_rate) = services
        .request_log_stats()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let logs = services
        .request_log_list(PER_PAGE, offset)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let total_pages = (total as usize).max(1).div_ceil(PER_PAGE);

    let body = maud::html! {
        // Stats cards
        div class="grid grid-cols-1 md:grid-cols-3 gap-4 mb-6" {
            div class="card bg-base-100 shadow-sm" {
                div class="card-body p-4" {
                    h3 class="text-sm font-medium text-base-content/60" { "Total Requests" }
                    p class="text-2xl font-bold mt-1" { (total) }
                }
            }
            div class="card bg-base-100 shadow-sm" {
                div class="card-body p-4" {
                    h3 class="text-sm font-medium text-base-content/60" { "Avg Latency" }
                    p class="text-2xl font-bold mt-1" { (format!("{:.1}ms", avg_latency)) }
                }
            }
            div class="card bg-base-100 shadow-sm" {
                div class="card-body p-4" {
                    h3 class="text-sm font-medium text-base-content/60" { "Success Rate" }
                    p class="text-2xl font-bold mt-1" { (format!("{:.1}%", success_rate)) }
                }
            }
        }
        // Table
        @if logs.is_empty() {
            div class="card bg-base-100 shadow-sm" {
                div class="card-body py-12 text-center" {
                    p class="text-sm text-base-content/50" { "No request logs yet." }
                }
            }
        } @else {
            div class="overflow-x-auto card bg-base-100 shadow-sm" {
                table class="table table-zebra table-sm" {
                    thead {
                        tr {
                            th { "Time" }
                            th { "Method" }
                            th { "Path" }
                            th { "Status" }
                            th { "Latency" }
                            th { "Auth" }
                        }
                    }
                    tbody {
                        @for log in &logs {
                            tr {
                                td class="text-xs text-base-content/60 whitespace-nowrap" {
                                    (fmt_time(&log.created_at))
                                }
                                td {
                                    span class="badge badge-sm badge-outline font-mono" { (log.method) }
                                }
                                td class="text-xs max-w-xs truncate" { (log.path) }
                                td {
                                    span class=(status_class(log.status)) { (log.status) }
                                }
                                td class="text-xs font-mono" { (format!("{:.1}ms", log.latency_ms)) }
                                td class="text-xs text-base-content/40" { (log.auth) }
                            }
                        }
                    }
                }
            }
        }
        // Pagination
        @if total_pages > 1 {
            div class="join mt-4 justify-center w-full" {
                @for p in 1..=total_pages {
                    a href=(format!("?page={p}"))
                       class=(format!("join-item btn btn-sm {}", if p == page { "btn-active" } else { "" })) {
                        (p)
                    }
                }
            }
        }
    };

    Ok(Html(components::render_page(
        "requests",
        Some(&user.email),
        "Requests",
        "Requests",
        body,
    )))
}

fn status_class(status: i64) -> &'static str {
    match status {
        200..=299 => "badge badge-sm badge-success",
        400..=499 => "badge badge-sm badge-warning",
        _ => "badge badge-sm badge-error",
    }
}

fn fmt_time(rfc3339: &str) -> String {
    DateTime::parse_from_rfc3339(rfc3339)
        .map(|dt| dt.format("%b %d, %Y %H:%M UTC").to_string())
        .unwrap_or_else(|_| rfc3339.to_string())
}
