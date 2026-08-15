use crate::schema::{current_dimension, set_dimension, table_is_empty};
use async_trait::async_trait;
use chrono::Utc;
use memayu_core::{
    decode_cursor, encode_cursor, Memory, MemoryPage, MetadataFilter, StorageError, StorageProvider,
};
use pgvector::Vector;
use sqlx::postgres::{PgPool, PgRow};
use sqlx::Row;

pub struct PostgresProvider {
    pool: PgPool,
    dimension: usize,
}

impl PostgresProvider {
    pub async fn connect(database_url: &str, dimension: usize) -> Result<Self, StorageError> {
        let pool = PgPool::connect(database_url)
            .await
            .map_err(|e| StorageError::Other(format!("connect postgres: {e}")))?;
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .map_err(|e| StorageError::Other(format!("run migrations: {e}")))?;

        let stored = current_dimension(&pool).await?;
        let dim = match stored {
            Some(d) if d == dimension => d,
            Some(_) if table_is_empty(&pool).await? => {
                set_dimension(&pool, dimension).await?;
                dimension
            }
            Some(d) => {
                return Err(StorageError::DimensionMismatch {
                    expected: d,
                    got: dimension,
                })
            }
            None => dimension,
        };

        Ok(Self {
            pool,
            dimension: dim,
        })
    }

    fn check_dim(&self, vector: &[f32]) -> Result<(), StorageError> {
        if vector.len() == self.dimension {
            Ok(())
        } else {
            Err(StorageError::DimensionMismatch {
                expected: self.dimension,
                got: vector.len(),
            })
        }
    }
}

fn memory_from_row(row: &PgRow) -> Result<Memory, StorageError> {
    let id: uuid::Uuid = row
        .try_get("id")
        .map_err(|e| StorageError::Other(format!("read id: {e}")))?;
    let user_id: String = row
        .try_get("user_id")
        .map_err(|e| StorageError::Other(format!("read user_id: {e}")))?;
    let content: String = row
        .try_get("content")
        .map_err(|e| StorageError::Other(format!("read content: {e}")))?;
    let embedding: Vector = row
        .try_get("embedding")
        .map_err(|e| StorageError::Other(format!("read embedding: {e}")))?;
    let metadata: serde_json::Value = row
        .try_get("metadata")
        .map_err(|e| StorageError::Other(format!("read metadata: {e}")))?;
    let created_at: chrono::DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|e| StorageError::Other(format!("read created_at: {e}")))?;
    let updated_at: chrono::DateTime<Utc> = row
        .try_get("updated_at")
        .map_err(|e| StorageError::Other(format!("read updated_at: {e}")))?;

    Ok(Memory {
        id: id.to_string(),
        user_id,
        content,
        vector: embedding.to_vec(),
        metadata: serde_json::from_value(metadata).unwrap_or_default(),
        created_at,
        updated_at,
    })
}

#[async_trait]
impl StorageProvider for PostgresProvider {
    async fn save_memory(&self, mem: &Memory) -> Result<(), StorageError> {
        self.check_dim(&mem.vector)?;
        let id: uuid::Uuid = mem
            .id
            .parse()
            .map_err(|e| StorageError::Other(format!("invalid memory id {}: {e}", mem.id)))?;
        let vector = Vector::from(mem.vector.clone());
        let metadata = serde_json::to_value(&mem.metadata)
            .map_err(|e| StorageError::Other(format!("serialize metadata: {e}")))?;

        sqlx::query(
            "INSERT INTO memories (id, user_id, content, embedding, metadata, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (id) DO UPDATE SET
               user_id = EXCLUDED.user_id,
               content = EXCLUDED.content,
               embedding = EXCLUDED.embedding,
               metadata = EXCLUDED.metadata,
               updated_at = EXCLUDED.updated_at",
        )
        .bind(id)
        .bind(&mem.user_id)
        .bind(&mem.content)
        .bind(&vector)
        .bind(metadata)
        .bind(mem.created_at)
        .bind(mem.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("save memory: {e}")))?;
        Ok(())
    }

    async fn search_memory(
        &self,
        user_id: &str,
        vector: &[f32],
        limit: usize,
        filter: Option<&MetadataFilter>,
    ) -> Result<Vec<(Memory, f32)>, StorageError> {
        self.check_dim(vector)?;
        let q = Vector::from(vector.to_vec());

        // $1 = query vector, $2 = user_id; a metadata filter, when present,
        // occupies the next placeholder and shifts LIMIT one slot later.
        let mut where_clause = String::from("user_id = $2");
        let mut filter_value: Option<serde_json::Value> = None;
        let mut limit_idx = 3usize;
        if let Some(f) = filter {
            filter_value = Some(
                serde_json::to_value(f)
                    .map_err(|e| StorageError::Other(format!("serialize filter: {e}")))?,
            );
            where_clause.push_str(&format!(" AND metadata @> ${limit_idx}::jsonb"));
            limit_idx += 1;
        }
        let sql = format!(
            "SELECT id, user_id, content, embedding, metadata, created_at, updated_at,
                    (embedding <=> $1) AS score
             FROM memories WHERE {where_clause}
             ORDER BY embedding <=> $1
             LIMIT ${limit_idx}"
        );
        let mut q = sqlx::query(&sql).bind(&q).bind(user_id);
        if let Some(v) = filter_value {
            q = q.bind(v);
        }
        let rows = q
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::Other(format!("search memories: {e}")))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let distance: f64 = row
                .try_get("score")
                .map_err(|e| StorageError::Other(format!("read score: {e}")))?;
            let mut mem = memory_from_row(&row)?;
            let embedding: Vector = row
                .try_get("embedding")
                .map_err(|e| StorageError::Other(format!("read embedding: {e}")))?;
            mem.vector = embedding.to_vec();
            out.push((mem, (1.0 - distance) as f32));
        }
        Ok(out)
    }

    async fn search_fulltext(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
        filter: Option<&MetadataFilter>,
    ) -> Result<Vec<(Memory, f32)>, StorageError> {
        // $1 = query, $2 = user_id; a metadata filter, when present, occupies
        // the next placeholder and shifts LIMIT one slot later.
        let mut where_clause =
            String::from("user_id = $2 AND content_tsv @@ plainto_tsquery('english', $1)");
        let mut filter_value: Option<serde_json::Value> = None;
        let mut limit_idx = 3usize;
        if let Some(f) = filter {
            filter_value = Some(
                serde_json::to_value(f)
                    .map_err(|e| StorageError::Other(format!("serialize filter: {e}")))?,
            );
            where_clause.push_str(&format!(" AND metadata @> ${limit_idx}::jsonb"));
            limit_idx += 1;
        }
        let sql = format!(
            "SELECT id, user_id, content, embedding, metadata, created_at, updated_at,
                    ts_rank(content_tsv, plainto_tsquery('english', $1)) AS score
             FROM memories WHERE {where_clause}
             ORDER BY score DESC
             LIMIT ${limit_idx}"
        );
        let mut q = sqlx::query(&sql).bind(query).bind(user_id);
        if let Some(v) = filter_value {
            q = q.bind(v);
        }
        let rows = q
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::Other(format!("full-text search: {e}")))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            // ts_rank returns `real` (FLOAT4), so decode directly as f32 to
            // match the StorageProvider contract instead of a FLOAT8 cast.
            let score: f32 = row
                .try_get("score")
                .map_err(|e| StorageError::Other(format!("read full-text score: {e}")))?;
            let mem = memory_from_row(&row)?;
            out.push((mem, score));
        }
        Ok(out)
    }

    async fn list_memories(
        &self,
        user_id: &str,
        limit: usize,
        cursor: Option<&str>,
        filter: Option<&MetadataFilter>,
    ) -> Result<MemoryPage, StorageError> {
        // Fetch one extra row to detect another page and derive the next
        // opaque cursor from the last visible row.
        let fetch = limit as i64 + 1;

        let mut where_clause = String::from("user_id = $1");
        let mut param_idx = 2usize;
        let mut filter_value: Option<serde_json::Value> = None;
        if let Some(f) = filter {
            filter_value = Some(
                serde_json::to_value(f)
                    .map_err(|e| StorageError::Other(format!("serialize filter: {e}")))?,
            );
            where_clause.push_str(&format!(" AND metadata @> ${param_idx}::jsonb"));
            param_idx += 1;
        }
        let mut cursor_value: Option<(chrono::DateTime<Utc>, uuid::Uuid)> = None;
        if let Some(c) = cursor {
            let (ts, id) = decode_cursor(c).ok_or_else(|| {
                StorageError::InvalidCursor("invalid pagination cursor".to_string())
            })?;
            let id: uuid::Uuid = id
                .parse()
                .map_err(|e| StorageError::InvalidCursor(format!("invalid cursor id: {e}")))?;
            where_clause.push_str(&format!(
                " AND (created_at < ${param_idx} OR (created_at = ${param_idx} AND id < ${}))",
                param_idx + 1
            ));
            cursor_value = Some((ts, id));
            param_idx += 2;
        }
        let sql = format!(
            "SELECT id, user_id, content, embedding, metadata, created_at, updated_at
             FROM memories WHERE {where_clause}
             ORDER BY created_at DESC, id DESC
             LIMIT ${param_idx}"
        );

        let mut q = sqlx::query(&sql).bind(user_id);
        if let Some(v) = filter_value {
            q = q.bind(v);
        }
        if let Some((ts, id)) = cursor_value {
            q = q.bind(ts).bind(id);
        }
        let rows = q
            .bind(fetch)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::Other(format!("list memories: {e}")))?;

        let mut out: Vec<Memory> = rows.iter().map(memory_from_row).collect::<Result<_, _>>()?;
        let next_cursor = if out.len() > limit {
            let last = &out[limit - 1];
            let cursor = encode_cursor(&last.created_at, &last.id);
            out.truncate(limit);
            Some(cursor)
        } else {
            None
        };
        Ok(MemoryPage::new(out, next_cursor))
    }

    async fn get_memory(&self, memory_id: &str) -> Result<Memory, StorageError> {
        let id: uuid::Uuid = memory_id
            .parse()
            .map_err(|e| StorageError::Other(format!("invalid memory id {memory_id}: {e}")))?;
        let row = sqlx::query(
            "SELECT id, user_id, content, embedding, metadata, created_at, updated_at
             FROM memories WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Other(format!("get memory: {e}")))?;
        row.map(|r| memory_from_row(&r))
            .ok_or_else(|| StorageError::Other(format!("memory {memory_id} not found")))?
    }

    async fn delete_memory(&self, memory_id: &str) -> Result<(), StorageError> {
        let id: uuid::Uuid = memory_id
            .parse()
            .map_err(|e| StorageError::Other(format!("invalid memory id {memory_id}: {e}")))?;
        sqlx::query("DELETE FROM memories WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Other(format!("delete memory: {e}")))?;
        Ok(())
    }
}

#[cfg(all(test, feature = "integration"))]
mod tests {
    use super::*;
    use memayu_core::Memory;
    use std::collections::HashMap;
    use std::env;

    fn test_url() -> Option<String> {
        env::var("MEMAYU_TEST_DATABASE_URL").ok()
    }

    #[tokio::test]
    async fn crud_roundtrip() {
        let url = match test_url() {
            Some(u) => u,
            None => return,
        };
        let provider = PostgresProvider::connect(&url, 3).await.unwrap();
        let m = Memory {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: "u1".into(),
            content: "User lives in Jakarta".into(),
            vector: vec![1.0, 0.0, 0.0],
            metadata: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        provider.save_memory(&m).await.unwrap();
        let list = provider
            .list_memories("u1", 10, None, None)
            .await
            .unwrap()
            .memories;
        assert_eq!(list.len(), 1);
        provider.delete_memory(&m.id).await.unwrap();
        assert!(provider
            .list_memories("u1", 10, None, None)
            .await
            .unwrap()
            .memories
            .is_empty());
    }

    #[tokio::test]
    async fn search_returns_ranked_scores() {
        let url = match test_url() {
            Some(u) => u,
            None => return,
        };
        let provider = PostgresProvider::connect(&url, 3).await.unwrap();
        let m1 = Memory {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: "u1".into(),
            content: "jakarta".into(),
            vector: vec![1.0, 0.0, 0.0],
            metadata: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let m2 = Memory {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: "u1".into(),
            content: "bandung".into(),
            vector: vec![0.0, 1.0, 0.0],
            metadata: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        provider.save_memory(&m1).await.unwrap();
        provider.save_memory(&m2).await.unwrap();
        let results = provider
            .search_memory("u1", &[0.9, 0.1, 0.0], 3, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0.content, "jakarta");
        assert!(results[0].1 > results[1].1, "similarity descending");
        provider.delete_memory(&m1.id).await.unwrap();
        provider.delete_memory(&m2.id).await.unwrap();
    }

    #[tokio::test]
    async fn fulltext_search_special_characters_are_literal() {
        let url = match test_url() {
            Some(u) => u,
            None => return,
        };
        let provider = PostgresProvider::connect(&url, 3).await.unwrap();
        let m1 = Memory {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: "u1".into(),
            content: "User's project costs $500!".into(),
            vector: vec![1.0, 0.0, 0.0],
            metadata: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        provider.save_memory(&m1).await.unwrap();

        // plainto_tsquery must treat FTS-reserved characters as literal text,
        // never raising, and still match the row that contains them.
        let results = provider
            .search_fulltext("u1", "User's project costs $500!", 5, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 1, "special-char query is literal, no crash");
        assert_eq!(results[0].0.id, m1.id);
        provider.delete_memory(&m1.id).await.unwrap();
    }

    #[tokio::test]
    async fn fulltext_search_emoji_and_non_ascii_input() {
        let url = match test_url() {
            Some(u) => u,
            None => return,
        };
        let provider = PostgresProvider::connect(&url, 3).await.unwrap();
        let m1 = Memory {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: "u1".into(),
            content: "plan 🚀 Q3 launch 💡 ideas".into(),
            vector: vec![1.0, 0.0, 0.0],
            metadata: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        provider.save_memory(&m1).await.unwrap();

        let results = provider
            .search_fulltext("u1", "plan 🚀 Q3 launch 💡 ideas!", 5, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 1, "emoji query is literal, no crash");
        assert_eq!(results[0].0.id, m1.id);
        provider.delete_memory(&m1.id).await.unwrap();
    }
}
