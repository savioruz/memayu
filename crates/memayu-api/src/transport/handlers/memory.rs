/// Axum handlers for memory routes — thin wrappers that delegate to
/// the core MemoryService.
use crate::error::{ApiError, ApiErrorBody};
use crate::modules::memory::dto::{
    AddMemoryRequest, AddMemoryResponse, ListMemoryResponse, ListQuery, ListedMemory,
    SearchMemoryRequest, SearchMemoryResponse, SearchResult, UpdateMemoryRequest,
    UpdateMemoryResponse,
};
use crate::transport::middleware::{AccountId, ApiState};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;

/// Add a new memory for a user
#[utoipa::path(
    post,
    path = "/api/memories/add",
    request_body = AddMemoryRequest,
    responses(
        (status = 200, description = "Memory added successfully", body = AddMemoryResponse),
        (status = 400, description = "Bad request", body = ApiErrorBody),
    )
)]
pub async fn add_memory(
    State(state): State<ApiState>,
    _account: AccountId,
    Json(req): Json<AddMemoryRequest>,
) -> Result<(StatusCode, Json<AddMemoryResponse>), ApiError> {
    let user_id = &_account.0;
    if req.content.trim().is_empty() {
        return Err(ApiError::bad_request("content is required"));
    }

    let mem = state
        .service
        .add_memory(user_id, &req.content, &req.metadata)
        .await?;
    Ok((
        StatusCode::OK,
        Json(AddMemoryResponse {
            status: "success".into(),
            memory_id: mem.id,
            dimension: mem.vector.len(),
        }),
    ))
}

/// Search memories by semantic similarity
#[utoipa::path(
    post,
    path = "/api/memories/search",
    request_body = SearchMemoryRequest,
    responses(
        (status = 200, description = "Search results", body = SearchMemoryResponse),
        (status = 400, description = "Bad request", body = ApiErrorBody),
    )
)]
pub async fn search_memory(
    State(state): State<ApiState>,
    _account: AccountId,
    Json(req): Json<SearchMemoryRequest>,
) -> Result<Json<SearchMemoryResponse>, ApiError> {
    let user_id = &_account.0;
    if req.query.trim().is_empty() {
        return Err(ApiError::bad_request("query is required"));
    }
    if req.limit == 0 {
        return Err(ApiError::bad_request("limit must be > 0"));
    }

    let results = state
        .service
        .search_memory(user_id, &req.query, req.limit)
        .await?;
    Ok(Json(SearchMemoryResponse {
        results: results
            .into_iter()
            .map(|(m, score)| SearchResult {
                memory_id: m.id,
                content: m.content,
                score,
                created_at: m.created_at,
            })
            .collect(),
    }))
}

/// List memories for a user
#[utoipa::path(
    get,
    path = "/api/memories/list",
    params(
        ("limit" = Option<usize>, Query, description = "Max results (default 100)"),
    ),
    responses(
        (status = 200, description = "List of memories", body = ListMemoryResponse),
        (status = 400, description = "Bad request", body = ApiErrorBody),
    )
)]
pub async fn list_memories(
    State(state): State<ApiState>,
    _account: AccountId,
    Query(q): Query<ListQuery>,
) -> Result<Json<ListMemoryResponse>, ApiError> {
    let user_id = &_account.0;
    let memories = state.service.list_memories(user_id, q.limit).await?;
    Ok(Json(ListMemoryResponse {
        memories: memories
            .into_iter()
            .map(|m| ListedMemory {
                memory_id: m.id,
                content: m.content,
                created_at: m.created_at,
                updated_at: m.updated_at,
            })
            .collect(),
    }))
}

/// Delete a memory by ID
#[utoipa::path(
    delete,
    path = "/api/memories/{id}",
    params(
        ("id" = String, Path, description = "Memory ID"),
    ),
    responses(
        (status = 204, description = "Memory deleted"),
        (status = 404, description = "Memory not found", body = ApiErrorBody),
    )
)]
pub async fn delete_memory(
    State(state): State<ApiState>,
    _account: AccountId,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.service.delete_memory(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Update a memory's content
#[utoipa::path(
    patch,
    path = "/api/memories/{id}",
    params(
        ("id" = String, Path, description = "Memory ID"),
    ),
    request_body = UpdateMemoryRequest,
    responses(
        (status = 200, description = "Memory updated", body = UpdateMemoryResponse),
        (status = 400, description = "Bad request", body = ApiErrorBody),
        (status = 404, description = "Memory not found", body = ApiErrorBody),
    )
)]
pub async fn update_memory(
    State(state): State<ApiState>,
    _account: AccountId,
    Path(id): Path<String>,
    Json(req): Json<UpdateMemoryRequest>,
) -> Result<Json<UpdateMemoryResponse>, ApiError> {
    if req.content.trim().is_empty() {
        return Err(ApiError::bad_request("content is required"));
    }
    let mem = state.service.update_memory(&id, &req.content).await?;
    Ok(Json(UpdateMemoryResponse {
        status: "success".into(),
        memory_id: mem.id,
        content: mem.content,
    }))
}
