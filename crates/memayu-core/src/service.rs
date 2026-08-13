use crate::extraction;
use crate::{
    CoreError, EmbedderProvider, ExtractionDecision, ExtractionResult, LlmProvider, Memory,
    Message, Metadata, StorageError, StorageProvider,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

pub struct MemoryService {
    storage: Arc<dyn StorageProvider>,
    embedder: Arc<dyn EmbedderProvider>,
    llm: Arc<dyn LlmProvider>,
}

impl MemoryService {
    pub fn new(
        storage: Arc<dyn StorageProvider>,
        embedder: Arc<dyn EmbedderProvider>,
        llm: Arc<dyn LlmProvider>,
    ) -> Self {
        Self {
            storage,
            embedder,
            llm,
        }
    }

    #[deprecated(note = "similarity_threshold is no longer used; use new()")]
    pub fn with_similarity_threshold(
        storage: Arc<dyn StorageProvider>,
        embedder: Arc<dyn EmbedderProvider>,
        llm: Arc<dyn LlmProvider>,
        _threshold: f32,
    ) -> Self {
        Self {
            storage,
            embedder,
            llm,
        }
    }

    pub async fn add_memory(
        &self,
        user_id: &str,
        content: &str,
        metadata: &Metadata,
    ) -> Result<Memory, CoreError> {
        let vector = self.embedder.embed(content).await?;

        const SEARCH_LIMIT: usize = 5;
        let mut candidates = self
            .storage
            .search_memory(user_id, &vector, SEARCH_LIMIT)
            .await?;

        // Always send all candidates to the LLM; let it decide ADD vs UPDATE.
        let plausible: Vec<&(Memory, f32)> = candidates.iter().collect();

        let messages: Vec<Message> = extraction::build_prompt(content, &plausible);
        let result: ExtractionResult = self.llm.extract(&messages).await?;

        let now = Utc::now();
        let memory = match result {
            ExtractionResult {
                decision: ExtractionDecision::Add,
                ..
            } => Memory {
                id: Uuid::new_v4().to_string(),
                user_id: user_id.to_string(),
                content: result.content,
                vector,
                metadata: metadata.clone(),
                created_at: now,
                updated_at: now,
            },
            ExtractionResult {
                decision: ExtractionDecision::Update,
                updated_memory_id,
                ..
            } => {
                let target_id = updated_memory_id.ok_or_else(|| {
                    CoreError::InvalidExtraction("update decision without a memory_id".into())
                })?;
                let target = candidates
                    .iter_mut()
                    .find(|(m, _)| m.id == target_id)
                    .ok_or_else(|| {
                        CoreError::InvalidExtraction(format!(
                            "update references memory_id {target_id} which is not among the candidates"
                        ))
                    })?;
                target.0.content = result.content;
                target.0.vector = vector;
                target.0.metadata = metadata.clone();
                target.0.updated_at = now;
                target.0.clone()
            }
        };

        self.storage.save_memory(&memory).await?;
        Ok(memory)
    }

    pub async fn search_memory(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(Memory, f32)>, CoreError> {
        let vector = self.embedder.embed(query).await?;
        let vector_hits = self.storage.search_memory(user_id, &vector, limit).await?;
        let fulltext_hits = self.storage.search_fulltext(user_id, query, limit).await?;
        Ok(crate::fusion::fuse(&vector_hits, &fulltext_hits, limit))
    }

    pub async fn list_memories(
        &self,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<Memory>, CoreError> {
        Ok(self.storage.list_memories(user_id, limit).await?)
    }

    pub async fn delete_memory(&self, memory_id: &str) -> Result<(), CoreError> {
        Ok(self.storage.delete_memory(memory_id).await?)
    }

    pub async fn update_memory(&self, memory_id: &str, content: &str) -> Result<Memory, CoreError> {
        let mut mem = self.storage.get_memory(memory_id).await.map_err(|e| {
            if matches!(e, StorageError::Other(ref s) if s.contains("not found")) {
                CoreError::NotFound(format!("memory {memory_id} not found"))
            } else {
                CoreError::from(e)
            }
        })?;
        let vector = self.embedder.embed(content).await?;
        mem.content = content.to_string();
        mem.vector = vector;
        mem.updated_at = Utc::now();
        self.storage.save_memory(&mem).await?;
        Ok(mem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EmbedError, LlmError, StorageError};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn mem(id: &str, user_id: &str, content: &str) -> Memory {
        Memory {
            id: id.to_string(),
            user_id: user_id.to_string(),
            content: content.to_string(),
            vector: vec![],
            metadata: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// In-memory storage: search returns all rows with a canned score.
    struct MockStorage {
        rows: Mutex<Vec<Memory>>,
        score: f32,
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
                .map(|m| (m, self.score))
                .collect())
        }
        async fn search_fulltext(
            &self,
            _user_id: &str,
            query: &str,
            limit: usize,
        ) -> Result<Vec<(Memory, f32)>, StorageError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.content.to_lowercase().contains(&query.to_lowercase()))
                .take(limit)
                .cloned()
                .map(|m| (m, self.score))
                .collect())
        }
        async fn list_memories(
            &self,
            _user_id: &str,
            limit: usize,
        ) -> Result<Vec<Memory>, StorageError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
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

    /// Embedder that returns the content bytes as the vector (any dim works).
    struct MockEmbedder;

    #[async_trait]
    impl EmbedderProvider for MockEmbedder {
        async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
            Ok(text.bytes().map(f32::from).collect())
        }
    }

    /// LLM with a scripted response per call.
    struct MockLlm {
        responses: Mutex<Vec<&'static str>>,
        last_prompt: Mutex<Option<String>>,
    }

    impl MockLlm {
        fn scripted(responses: Vec<&'static str>) -> Self {
            Self {
                responses: Mutex::new(responses),
                last_prompt: Mutex::new(None),
            }
        }

        fn last_prompt(&self) -> String {
            self.last_prompt.lock().unwrap().clone().unwrap_or_default()
        }
    }

    #[async_trait]
    impl LlmProvider for MockLlm {
        async fn extract(&self, messages: &[Message]) -> Result<ExtractionResult, LlmError> {
            *self.last_prompt.lock().unwrap() = Some(
                messages
                    .iter()
                    .map(|m| m.content.as_str())
                    .collect::<String>(),
            );
            let next = self.responses.lock().unwrap().remove(0);
            let parsed: serde_json::Value = serde_json::from_str(next).unwrap();
            let decision = parsed["decision"].as_str().unwrap();
            if decision == "add" {
                Ok(ExtractionResult::add(parsed["content"].as_str().unwrap()))
            } else {
                Ok(ExtractionResult::update(
                    parsed["memory_id"].as_str().unwrap(),
                    parsed["content"].as_str().unwrap(),
                ))
            }
        }
    }

    fn service_with(storage: MockStorage, llm: MockLlm) -> MemoryService {
        MemoryService::new(Arc::new(storage), Arc::new(MockEmbedder), Arc::new(llm))
    }

    #[tokio::test]
    async fn update_replaces_conflicting_fact() {
        // UAT #3: add "lives in Jakarta" then "moved to Bandung" -> 1 memory, content "Bandung".
        let storage = MockStorage {
            rows: Mutex::new(vec![mem("m1", "u1", "User lives in Jakarta")]),
            score: 0.95, // plausible candidate
        };
        let llm = MockLlm::scripted(vec![
            r#"{"decision":"update","memory_id":"m1","content":"User lives in Bandung"}"#,
        ]);
        let svc = service_with(storage, llm);

        let result = svc
            .add_memory("u1", "User moved to Bandung", &HashMap::new())
            .await
            .unwrap();

        assert_eq!(result.id, "m1");
        assert_eq!(result.content, "User lives in Bandung");
        let rows = svc.list_memories("u1", 10).await.unwrap();
        assert_eq!(rows.len(), 1, "must be 1 memory, not 2");
        assert_eq!(rows[0].content, "User lives in Bandung");
    }

    #[tokio::test]
    async fn new_fact_is_added_not_updated() {
        // UAT #4: genuinely new fact -> new record.
        let storage = MockStorage {
            rows: Mutex::new(vec![mem("m1", "u1", "User lives in Jakarta")]),
            score: 0.5, // below threshold, not plausible
        };
        let llm = MockLlm::scripted(vec![
            r#"{"decision":"add","memory_id":null,"content":"User prefers coffee over tea"}"#,
        ]);
        let svc = service_with(storage, llm);

        let result = svc
            .add_memory("u1", "User prefers coffee", &HashMap::new())
            .await
            .unwrap();

        assert_ne!(result.id, "m1");
        assert_eq!(result.content, "User prefers coffee over tea");
        assert_eq!(svc.list_memories("u1", 10).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn search_returns_scores() {
        let storage = MockStorage {
            rows: Mutex::new(vec![mem("m1", "u1", "User lives in Jakarta")]),
            score: 0.9,
        };
        let llm = MockLlm::scripted(vec![]);
        let svc = service_with(storage, llm);

        let results = svc
            .search_memory("u1", "where does user live", 3)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        // RRF: only the vector leg matches here (rank 0), so the fused score
        // is 1 / (k + 1).
        let expected = (1.0 / (crate::fusion::RRF_K + 1.0)) as f32;
        assert!((results[0].1 - expected).abs() < 1e-6);
    }

    #[tokio::test]
    async fn delete_removes_memory() {
        let storage = MockStorage {
            rows: Mutex::new(vec![mem("m1", "u1", "User lives in Jakarta")]),
            score: 0.0,
        };
        let llm = MockLlm::scripted(vec![]);
        let svc = service_with(storage, llm);

        svc.delete_memory("m1").await.unwrap();
        assert!(svc.list_memories("u1", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn merge_conceptually_conflicting_fact_despite_low_similarity() {
        // "favorite language is Rust" then "prefers Go for backend"
        // -> UPDATE, not ADD. Embeddings may differ, but the LLM
        // must see the candidate and judge the conceptual conflict.
        let storage = MockStorage {
            rows: Mutex::new(vec![mem(
                "m1",
                "u1",
                "Favorite programming language is Rust",
            )]),
            score: 0.5, // low similarity but conceptually related
        };
        let llm = MockLlm::scripted(vec![
            r#"{"decision":"update","memory_id":"m1","content":"Favorite programming language is Go (for backend)"}"#,
        ]);
        let svc = service_with(storage, llm);

        let result = svc
            .add_memory("u1", "Prefers Go for backend work", &HashMap::new())
            .await
            .unwrap();

        assert_eq!(result.id, "m1", "must update m1, not create a new memory");
        assert!(
            result.content.contains("Go"),
            "content must reflect the update to Go"
        );
        let rows = svc.list_memories("u1", 10).await.unwrap();
        assert_eq!(rows.len(), 1, "must be exactly 1 memory after update");
    }

    #[tokio::test]
    async fn unrelated_fact_stays_separate() {
        // "lives in Jakarta" then "prefers dark mode" -> 2 separate memories.
        let storage = MockStorage {
            rows: Mutex::new(vec![mem("m1", "u1", "User lives in Jakarta")]),
            score: 0.5,
        };
        let llm = MockLlm::scripted(vec![
            r#"{"decision":"add","memory_id":null,"content":"User prefers dark mode"}"#,
        ]);
        let svc = service_with(storage, llm);

        let result = svc
            .add_memory("u1", "Prefers dark mode in editors", &HashMap::new())
            .await
            .unwrap();

        assert_ne!(
            result.id, "m1",
            "must be a new memory, not overwrite Jakarta"
        );
        let rows = svc.list_memories("u1", 10).await.unwrap();
        assert_eq!(rows.len(), 2, "must have 2 separate memories");
        let contents: Vec<&str> = rows.iter().map(|m| m.content.as_str()).collect();
        assert!(contents.iter().any(|c| c.contains("Jakarta")));
        assert!(contents.iter().any(|c| c.contains("dark mode")));
    }

    // ── issue #21: ADD/UPDATE conflict-resolution coverage ──

    #[tokio::test]
    async fn multiple_candidates_llm_picks_correct_one() {
        // Three similar existing memories; the LLM must update the right one.
        let storage = MockStorage {
            rows: Mutex::new(vec![
                mem("m1", "u1", "User lives in Jakarta"),
                mem("m2", "u1", "Favorite programming language is Rust"),
                mem("m3", "u1", "User works at Acme Corp"),
            ]),
            score: 0.9,
        };
        let llm = MockLlm::scripted(vec![
            r#"{"decision":"update","memory_id":"m2","content":"Favorite programming language is Go (for backend)"}"#,
        ]);
        let svc = service_with(storage, llm);

        let result = svc
            .add_memory("u1", "Prefers Go for backend work", &HashMap::new())
            .await
            .unwrap();

        assert_eq!(result.id, "m2", "must update m2, not m1 or m3");
        let rows = svc.list_memories("u1", 10).await.unwrap();
        assert_eq!(rows.len(), 3, "update must not create or drop memories");
        let by_id = |id: &str| {
            rows.iter()
                .find(|m| m.id == id)
                .map(|m| m.content.as_str())
                .unwrap_or("<missing>")
        };
        assert!(by_id("m2").contains("Go"), "m2 content must reflect update");
        assert!(by_id("m1").contains("Jakarta"), "m1 must be untouched");
        assert!(by_id("m3").contains("Acme"), "m3 must be untouched");
    }

    #[tokio::test]
    async fn multiple_candidates_all_are_shown_to_llm() {
        // The LLM cannot pick correctly if candidates are truncated or missing
        // from the prompt; verify all three ids reach the prompt.
        let storage = MockStorage {
            rows: Mutex::new(vec![
                mem("m1", "u1", "User lives in Jakarta"),
                mem("m2", "u1", "Favorite programming language is Rust"),
                mem("m3", "u1", "User works at Acme Corp"),
            ]),
            score: 0.9,
        };
        let llm = Arc::new(MockLlm::scripted(vec![
            r#"{"decision":"update","memory_id":"m2","content":"updated"}"#,
        ]));
        let llm_provider: Arc<dyn LlmProvider> = llm.clone();
        let svc = MemoryService::new(Arc::new(storage), Arc::new(MockEmbedder), llm_provider);

        svc.add_memory("u1", "Prefers Go", &HashMap::new())
            .await
            .unwrap();

        let prompt = llm.last_prompt();
        assert!(prompt.contains("m1"), "m1 must be offered to the LLM");
        assert!(prompt.contains("m2"), "m2 must be offered to the LLM");
        assert!(prompt.contains("m3"), "m3 must be offered to the LLM");
    }

    #[tokio::test]
    async fn update_preserves_immutable_attributes() {
        // Updating content must not rewrite created_at (immutable), even though
        // content, vector, metadata and updated_at are refreshed.
        let created = Utc::now();
        let original = Memory {
            id: "m1".to_string(),
            user_id: "u1".to_string(),
            content: "User lives in Jakarta".to_string(),
            vector: vec![],
            metadata: HashMap::new(),
            created_at: created,
            updated_at: created,
        };
        let storage = MockStorage {
            rows: Mutex::new(vec![original.clone()]),
            score: 0.95,
        };
        let llm = MockLlm::scripted(vec![
            r#"{"decision":"update","memory_id":"m1","content":"User lives in Bandung"}"#,
        ]);
        let svc = service_with(storage, llm);

        let result = svc
            .add_memory("u1", "User moved to Bandung", &HashMap::new())
            .await
            .unwrap();

        assert_eq!(result.content, "User lives in Bandung");
        assert_eq!(
            result.created_at, created,
            "created_at must be preserved across updates"
        );
        let rows = svc.list_memories("u1", 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].created_at, created);
    }

    #[tokio::test]
    async fn ambiguous_input_adds_safe_fallback() {
        // Two facts share a keyword ("Java") but are unrelated: the LLM opts
        // for ADD as the safe fallback instead of forcing a merge (PRD-01 §6).
        let storage = MockStorage {
            rows: Mutex::new(vec![mem("m1", "u1", "User programs in Java")]),
            score: 0.8,
        };
        let llm = MockLlm::scripted(vec![
            r#"{"decision":"add","memory_id":null,"content":"User enjoys Java coffee beans"}"#,
        ]);
        let svc = service_with(storage, llm);

        let result = svc
            .add_memory("u1", "User enjoys Java coffee", &HashMap::new())
            .await
            .unwrap();

        assert_ne!(result.id, "m1", "ambiguous overlap must not force a merge");
        let rows = svc.list_memories("u1", 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        let contents: Vec<&str> = rows.iter().map(|m| m.content.as_str()).collect();
        assert!(contents.iter().any(|c| c.contains("programs in Java")));
        assert!(contents.iter().any(|c| c.contains("coffee")));
    }

    #[tokio::test]
    async fn low_lexical_overlap_conflict_still_updates() {
        // "favorite language Rust" → "prefers Go" has almost no shared tokens;
        // the LLM must still recognize the conceptual conflict and update.
        let storage = MockStorage {
            rows: Mutex::new(vec![mem(
                "m1",
                "u1",
                "Favorite programming language is Rust",
            )]),
            score: 0.45, // low similarity, but semantically conflicting
        };
        let llm = MockLlm::scripted(vec![
            r#"{"decision":"update","memory_id":"m1","content":"Favorite programming language is Go (for backend)"}"#,
        ]);
        let svc = service_with(storage, llm);

        let result = svc
            .add_memory("u1", "Prefers Go for backend work", &HashMap::new())
            .await
            .unwrap();

        assert_eq!(result.id, "m1");
        assert!(result.content.contains("Go"));
        let rows = svc.list_memories("u1", 10).await.unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn empty_candidate_set_forces_add() {
        // With no plausible candidates, the LLM should never be offered a
        // memory_id, and ADD is the only valid outcome.
        let storage = MockStorage {
            rows: Mutex::new(vec![]),
            score: 0.0,
        };
        let llm = Arc::new(MockLlm::scripted(vec![
            r#"{"decision":"add","memory_id":null,"content":"First memory"}"#,
        ]));
        let llm_provider: Arc<dyn LlmProvider> = llm.clone();
        let svc = MemoryService::new(Arc::new(storage), Arc::new(MockEmbedder), llm_provider);

        let result = svc
            .add_memory("u1", "First memory", &HashMap::new())
            .await
            .unwrap();

        assert_eq!(result.content, "First memory");
        assert_eq!(svc.list_memories("u1", 10).await.unwrap().len(), 1);
        assert!(
            llm.last_prompt().contains("EXISTING MEMORIES: none"),
            "prompt must reflect the empty candidate set"
        );
    }
}
