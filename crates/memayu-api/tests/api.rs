#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use axum::Router;
    use memayu_api::{build_api_router, open_db, ConfigRegistry};
    use memayu_config::{ProviderConfig, StorageBackend, StorageConfig};
    use memayu_core::{
        EmbedError, EmbedderProvider, ExtractionDecision, ExtractionResult, LlmError, LlmProvider,
        Message, StorageError, StorageProvider,
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
            _user_id: &str,
            _vector: &[f32],
            limit: usize,
        ) -> Result<Vec<(Memory, f32)>, StorageError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .take(limit)
                .cloned()
                .map(|m| (m, 0.9))
                .collect())
        }
        async fn list_memories(
            &self,
            user_id: &str,
            limit: usize,
        ) -> Result<Vec<Memory>, StorageError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.user_id == user_id)
                .take(limit)
                .cloned()
                .collect())
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
        assert_eq!(parsed["status"], "success");
        assert_eq!(parsed["dimension"], 3);
        assert_eq!(parsed["metadata"]["source"], "test");
        assert!(parsed["memory_id"].is_string());
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
        let results = parsed["results"].as_array().unwrap();
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
        assert!(!parsed["results"].as_array().unwrap().is_empty());
        assert!(parsed["results"][0]["score"].is_number());
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
}
