use crate::extraction::{self, DEFAULT_SIMILARITY_THRESHOLD};
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
    similarity_threshold: f32,
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
            similarity_threshold: DEFAULT_SIMILARITY_THRESHOLD,
        }
    }

    pub fn with_similarity_threshold(
        storage: Arc<dyn StorageProvider>,
        embedder: Arc<dyn EmbedderProvider>,
        llm: Arc<dyn LlmProvider>,
        threshold: f32,
    ) -> Self {
        Self {
            storage,
            embedder,
            llm,
            similarity_threshold: threshold,
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

        let plausible: Vec<&(Memory, f32)> =
            extraction::above_threshold(&candidates, self.similarity_threshold);

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
        Ok(self.storage.search_memory(user_id, &vector, limit).await?)
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
    }

    impl MockLlm {
        fn scripted(responses: Vec<&'static str>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockLlm {
        async fn extract(&self, _messages: &[Message]) -> Result<ExtractionResult, LlmError> {
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
        assert_eq!(results[0].1, 0.9);
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
}
