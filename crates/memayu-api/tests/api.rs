#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use axum::Router;
    use memayu_api::{build_api_router, open_db, ConfigRegistry};
    use memayu_config::{EmbedderBackend, ProviderConfig, StorageBackend, StorageConfig};
    use memayu_core::{
        EmbedError, EmbedderProvider, ExtractionDecision, ExtractionResult, LlmError, LlmProvider,
        MemoryPage, Message, MetadataFilter, StorageError, StorageProvider,
    };
    use memayu_core::{Memory, MemoryService};
    use std::sync::{Arc, Mutex};

    struct MockStorage {
        rows: Mutex<Vec<Memory>>,
    }

    impl MockStorage {
        fn with(rows: Vec<Memory>) -> Self {
            Self {
                rows: Mutex::new(rows),
            }
        }
    }

    #[async_trait]
    impl StorageProvider for MockStorage {
        async fn save_memory(&self, mem: &Memory) -> Result<(), StorageError> {
            let mut rows = self.rows.lock().unwrap();
            if let Some(existing) = rows.iter_mut().find(|m| m.id == mem.id) {
                *existing = mem.clone();
            } else {
                rows.push(mem.clone());
            }
            Ok(())
        }
        async fn search_memory(
            &self,
            user_id: &str,
            _vector: &[f32],
            limit: usize,
            filter: Option<&MetadataFilter>,
        ) -> Result<Vec<(Memory, f32)>, StorageError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|m| {
                    m.user_id == user_id
                        && filter
                            .map(|f| f.iter().all(|(k, v)| m.metadata.get(k) == Some(v)))
                            .unwrap_or(true)
                })
                .take(limit)
                .cloned()
                .map(|m| (m, 0.9))
                .collect())
        }
        async fn search_fulltext(
            &self,
            user_id: &str,
            query: &str,
            limit: usize,
            filter: Option<&MetadataFilter>,
        ) -> Result<Vec<(Memory, f32)>, StorageError> {
            let needle = query.to_lowercase();
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|m| {
                    m.user_id == user_id
                        && m.content.to_lowercase().contains(&needle)
                        && filter
                            .map(|f| f.iter().all(|(k, v)| m.metadata.get(k) == Some(v)))
                            .unwrap_or(true)
                })
                .take(limit)
                .cloned()
                .map(|m| (m, 1.0))
                .collect())
        }
        async fn list_memories(
            &self,
            user_id: &str,
            limit: usize,
            cursor: Option<&str>,
            filter: Option<&MetadataFilter>,
        ) -> Result<MemoryPage, StorageError> {
            use memayu_core::decode_cursor;
            let mut rows: Vec<Memory> = self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|m| {
                    m.user_id == user_id
                        && filter
                            .map(|f| f.iter().all(|(k, v)| m.metadata.get(k) == Some(v)))
                            .unwrap_or(true)
                })
                .cloned()
                .collect();
            rows.sort_by(|a, b| {
                b.created_at
                    .cmp(&a.created_at)
                    .then_with(|| b.id.cmp(&a.id))
            });
            let total = rows.len();
            if let Some(c) = cursor {
                let (ts, id) = decode_cursor(c)
                    .ok_or_else(|| StorageError::InvalidCursor("invalid cursor".to_string()))?;
                rows.retain(|m| m.created_at < ts || (m.created_at == ts && m.id < id));
            }
            let has_more = rows.len() > limit;
            rows.truncate(limit);
            let next_cursor = if has_more {
                let last = rows.last().unwrap();
                Some(memayu_core::encode_cursor(&last.created_at, &last.id))
            } else {
                None
            };
            Ok(MemoryPage::new(rows, next_cursor, total))
        }
        async fn get_memory(&self, memory_id: &str) -> Result<Memory, StorageError> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .find(|m| m.id == memory_id)
                .cloned()
                .ok_or_else(|| StorageError::Other(format!("memory {memory_id} not found")))
        }
        async fn delete_memory(&self, memory_id: &str) -> Result<(), StorageError> {
            self.rows.lock().unwrap().retain(|m| m.id != memory_id);
            Ok(())
        }
    }

    struct MockEmbedder;

    #[async_trait]
    impl EmbedderProvider for MockEmbedder {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
            Ok(vec![1.0, 0.0, 0.0])
        }
    }

    struct MockLlm;

    #[async_trait]
    impl LlmProvider for MockLlm {
        async fn extract(&self, _messages: &[Message]) -> Result<ExtractionResult, LlmError> {
            Ok(ExtractionResult {
                decision: ExtractionDecision::Add,
                updated_memory_id: None,
                content: "User prefers coffee".into(),
            })
        }
    }

    fn test_config() -> ProviderConfig {
        ProviderConfig {
            backend: EmbedderBackend::Remote,
            base_url: "http://localhost:11434".into(),
            api_key: None,
            model: "test-model".into(),
        }
    }

    async fn build_test_app() -> Router {
        let storage = StorageConfig {
            backend: StorageBackend::Libsql,
            libsql_path: ":memory:".to_string(),
            database_url: None,
        };
        let db = open_db(&storage).await.unwrap();
        let registry = ConfigRegistry::new(test_config(), test_config());
        let service = Arc::new(MemoryService::new(
            Arc::new(MockStorage::with(vec![])),
            Arc::new(MockEmbedder),
            Arc::new(MockLlm),
        ));
        build_api_router(db, service, registry)
    }

    /// Like `build_test_app`, but returns the router alongside its in-memory
    /// DbClient so tests can inspect the persisted provider_config rows.
    async fn build_test_app_with_db() -> (Router, memayu_api::DbClient) {
        let storage = StorageConfig {
            backend: StorageBackend::Libsql,
            libsql_path: ":memory:".to_string(),
            database_url: None,
        };
        let db = open_db(&storage).await.unwrap();
        let registry = ConfigRegistry::new(test_config(), test_config());
        let service = Arc::new(MemoryService::new(
            Arc::new(MockStorage::with(vec![])),
            Arc::new(MockEmbedder),
            Arc::new(MockLlm),
        ));
        (build_api_router(db.clone(), service, registry), db)
    }

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::util::ServiceExt;

    /// Helper: run setup and return session cookie
    async fn setup_and_login() -> (Router, String) {
        let app = build_test_app().await;

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/setup")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"email":"test@memayu.dev","password":"Secret12","confirm":"Secret12"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cookie = resp
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        (app, cookie)
    }

    #[tokio::test]
    async fn add_memory_returns_memory_id_and_dimension() {
        let (app, cookie) = setup_and_login().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/add")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        r#"{"content":"User prefers coffee","metadata":{"source":"test"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        assert_eq!(parsed["status"], "success");
        assert_eq!(parsed["dimension"], 3);
        assert_eq!(parsed["metadata"]["source"], "test");
        assert!(parsed["memory_id"].is_string());
    }

    #[tokio::test]
    async fn add_memories_batch_stores_all() {
        let (app, cookie) = setup_and_login().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/batch")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        r#"{
                            "memories": [
                                {"content":"loves hiking","metadata":{"source":"batch"}},
                                {"content":"prefers coffee"},
                                {"content":"works remote"}
                            ]
                        }"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        assert_eq!(parsed["status"], "success");
        assert_eq!(parsed["added"], 3);
        assert_eq!(parsed["memory_ids"].as_array().unwrap().len(), 3);
        assert!(parsed["errors"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn add_memories_batch_reports_per_item_errors() {
        let (app, cookie) = setup_and_login().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/batch")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        r#"{
                            "memories": [
                                {"content":"valid one"},
                                {"content":"   "},
                                {"content":"valid two"}
                            ]
                        }"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        assert_eq!(parsed["status"], "success");
        assert_eq!(parsed["added"], 2, "valid items must be stored");
        assert_eq!(parsed["memory_ids"].as_array().unwrap().len(), 2);
        let errors = parsed["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1, "blank item must be reported as a failure");
        assert_eq!(errors[0]["index"], 1);
        assert!(errors[0]["error"]
            .as_str()
            .unwrap()
            .contains("content is required"));
    }

    #[tokio::test]
    async fn add_memories_batch_rejects_empty() {
        let (app, cookie) = setup_and_login().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/batch")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(r#"{"memories":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn metadata_round_trips_in_search_and_list() {
        let (app, cookie) = setup_and_login().await;

        // Add a memory with metadata
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/add")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        r#"{"content":"User lives in Jakarta","metadata":{"source":"telegram","tag":"location"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        // add response echoes metadata
        assert_eq!(parsed["metadata"]["source"], "telegram");
        assert_eq!(parsed["metadata"]["tag"], "location");

        // Search: verify metadata in results (mock LLM normalizes to "User prefers coffee")
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/search")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(r#"{"query":"coffee","limit":3}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        let results = parsed["memories"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["metadata"]["source"], "telegram");
        assert_eq!(results[0]["metadata"]["tag"], "location");

        // List: verify metadata in list results
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/memories/list?limit=100")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        let memories = parsed["memories"].as_array().unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0]["metadata"]["source"], "telegram");
        assert_eq!(memories[0]["metadata"]["tag"], "location");
    }

    #[tokio::test]
    async fn add_memory_rejects_empty_content() {
        let (app, cookie) = setup_and_login().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/add")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(r#"{"content":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn search_memory_returns_ranked_results() {
        let (app, cookie) = setup_and_login().await;

        // Add a memory first
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/add")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(r#"{"content":"User lives in Jakarta"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/search")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        r#"{"query":"where does the user live","limit":3}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        assert!(!parsed["memories"].as_array().unwrap().is_empty());
        assert!(parsed["memories"][0]["score"].is_number());
    }

    #[tokio::test]
    async fn list_memories_and_delete() {
        let (app, cookie) = setup_and_login().await;

        // Add a memory
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/add")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(r#"{"content":"jakarta"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        let memory_id = parsed["memory_id"].as_str().unwrap();

        // List memories
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/memories/list")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        assert_eq!(parsed["memories"].as_array().unwrap().len(), 1);

        // Delete the memory
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/memories/{}", memory_id))
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn update_memory_changes_content() {
        let (app, cookie) = setup_and_login().await;

        // Add a memory first
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/add")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(r#"{"content":"User lives in Jakarta"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        let memory_id = parsed["memory_id"].as_str().unwrap();

        // Update it
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/memories/{}", memory_id))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(r#"{"content":"User moved to Bandung"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        assert_eq!(parsed["status"], "success");
        assert_eq!(parsed["memory_id"], memory_id);
        assert_eq!(parsed["content"], "User moved to Bandung");

        // Verify via list
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/memories/list")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        let memories = parsed["memories"].as_array().unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0]["content"], "User moved to Bandung");
    }

    #[tokio::test]
    async fn update_memory_rejects_empty_content() {
        let (app, cookie) = setup_and_login().await;

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/memories/nonexistent")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(r#"{"content":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn update_memory_returns_404_for_missing_id() {
        let (app, cookie) = setup_and_login().await;

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/memories/00000000-0000-0000-0000-000000000000")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(r#"{"content":"some content"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// Add a memory with the given metadata, returning its `memory_id`.
    async fn add_memory_with_meta(app: &Router, cookie: &str, content: &str, meta: &str) -> String {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/add")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(Body::from(format!(
                        r#"{{"content":"{content}","metadata":{meta}}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        parsed["memory_id"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn search_respects_metadata_filter() {
        let (app, cookie) = setup_and_login().await;
        add_memory_with_meta(
            &app,
            &cookie,
            "user lives in jakarta",
            r#"{"source":"telegram"}"#,
        )
        .await;
        add_memory_with_meta(&app, &cookie, "user likes tea", r#"{"source":"cli"}"#).await;

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories/search")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        r#"{"query":"tea","limit":10,"metadata_filter":{"source":"cli"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        let results = parsed["memories"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["metadata"]["source"], "cli");
    }

    #[tokio::test]
    async fn list_respects_metadata_filter() {
        let (app, cookie) = setup_and_login().await;
        add_memory_with_meta(
            &app,
            &cookie,
            "user lives in jakarta",
            r#"{"source":"telegram"}"#,
        )
        .await;
        add_memory_with_meta(&app, &cookie, "user likes tea", r#"{"source":"cli"}"#).await;

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/memories/list?limit=10&metadata_filter.source=telegram")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        let memories = parsed["memories"].as_array().unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0]["metadata"]["source"], "telegram");
    }

    #[tokio::test]
    async fn list_paginates_with_cursor() {
        let (app, cookie) = setup_and_login().await;
        add_memory_with_meta(&app, &cookie, "m0", r#"{"tag":"m0"}"#).await;
        add_memory_with_meta(&app, &cookie, "m1", r#"{"tag":"m1"}"#).await;
        add_memory_with_meta(&app, &cookie, "m2", r#"{"tag":"m2"}"#).await;

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/memories/list?limit=2")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        let page1 = parsed["memories"].as_array().unwrap();
        let tags1: Vec<&str> = page1
            .iter()
            .map(|m| m["metadata"]["tag"].as_str().unwrap())
            .collect();
        assert_eq!(tags1, vec!["m2", "m1"]);
        let cursor = parsed["next_cursor"].as_str().unwrap().to_string();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/memories/list?limit=2&cursor={}",
                        urlencoding(&cursor)
                    ))
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        let page2 = parsed["memories"].as_array().unwrap();
        let tags2: Vec<&str> = page2
            .iter()
            .map(|m| m["metadata"]["tag"].as_str().unwrap())
            .collect();
        assert_eq!(tags2, vec!["m0"]);
        assert!(parsed["next_cursor"].is_null());
    }

    fn urlencoding(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{:02X}", b),
            })
            .collect()
    }

    #[tokio::test]
    async fn list_rejects_invalid_cursor_with_400() {
        let (app, cookie) = setup_and_login().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/memories/list?limit=10&cursor=not-a-valid-cursor")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_defaults_to_50_when_limit_omitted() {
        let (app, cookie) = setup_and_login().await;
        // MockLlm always decides `Add`, so each POST creates a distinct row.
        for i in 0..55 {
            add_memory_with_meta(
                &app,
                &cookie,
                &format!("mem {i}"),
                &format!(r#"{{"i":"{i}"}}"#),
            )
            .await;
        }
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/memories/list")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let parsed = parsed["result"].clone();
        // Omitted limit defaults to 50, not the full 55.
        assert_eq!(parsed["memories"].as_array().unwrap().len(), 50);
        assert_eq!(parsed["total_data"], 55);
        assert!(parsed["next_cursor"].is_string());
    }

    #[tokio::test]
    async fn list_rejects_over_max_limit_with_400() {
        let (app, cookie) = setup_and_login().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/memories/list?limit=101")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_providers_local_embedder_clears_stale_base_url() {
        let (app, db) = build_test_app_with_db().await;

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/setup")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"email":"test@memayu.dev","password":"Secret12","confirm":"Secret12"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cookie = resp
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        // Post a local embedder carrying a stale remote base_url/api_key.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/providers")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        r#"{"embedder":{"backend":"local","base_url":"https://api.openai.com/v1","api_key":"sk-stale","model":"mini"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // The response reflects the cleared (normalized) registry config.
        assert_eq!(json["embedder"]["base_url"], "");
        assert_eq!(json["embedder"]["api_key"], serde_json::Value::Null);

        // The persisted row is also cleared.
        let rows = db.provider_configs().await.unwrap();
        let (backend, base_url, api_key, _) = &rows["embedder"];
        assert_eq!(backend, "local");
        assert_eq!(
            base_url, "",
            "stale base_url must not be persisted via POST"
        );
        assert_eq!(api_key, "", "stale api_key must not be persisted via POST");

        // A subsequent GET reflects the cleared value.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/providers")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["embedder"]["base_url"], "");
        assert_eq!(json["embedder"]["api_key"], serde_json::Value::Null);
    }

    // ── /api/health (issue #47) ──

    #[tokio::test]
    async fn health_is_unauthenticated_and_setup_required_on_fresh_instance() {
        let app = build_test_app().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "setup_required");
    }

    #[tokio::test]
    async fn health_is_setup_required_with_admin_but_no_providers() {
        let (app, _cookie) = setup_and_login().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "setup_required");
    }

    #[tokio::test]
    async fn health_is_ready_after_setup_and_provider_config() {
        let (app, cookie) = setup_and_login().await;

        // Persist a provider row the same way the dashboard does.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/providers")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        r#"{"embedder":{"backend":"local","base_url":"","api_key":null,"model":"mini"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ready");
    }
}
