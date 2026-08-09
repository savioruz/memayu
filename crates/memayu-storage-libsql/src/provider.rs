use crate::row::memory_from_row;
use crate::schema::create_schema;
use async_trait::async_trait;
use libsql::Connection;
use memayu_core::{Memory, StorageError, StorageProvider};

pub struct LibsqlProvider {
    conn: Connection,
    dimension: usize,
}

impl LibsqlProvider {
    pub async fn open(path: &str, dimension: usize) -> Result<Self, StorageError> {
        let db = libsql::Builder::new_local(path)
            .build()
            .await
            .map_err(|e| StorageError::Other(format!("open libsql db at {path}: {e}")))?;
        let conn = db
            .connect()
            .map_err(|e| StorageError::Other(format!("connect to libsql db: {e}")))?;

        let stored_dim = create_schema(&conn, dimension).await?;

        Ok(Self {
            conn,
            dimension: stored_dim,
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
const COLUMNS: &str = "id, user_id, content, embedding, metadata, created_at, updated_at";

#[async_trait]
impl StorageProvider for LibsqlProvider {
    async fn save_memory(&self, mem: &Memory) -> Result<(), StorageError> {
        self.check_dim(&mem.vector)?;
        let blob = crate::row::f32_blob(&mem.vector);
        self.conn
            .execute(
                &format!(
                    "INSERT INTO memories ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT (id) DO UPDATE SET
                       user_id = excluded.user_id,
                       content = excluded.content,
                       embedding = excluded.embedding,
                       metadata = excluded.metadata,
                       updated_at = excluded.updated_at"
                ),
                (
                    mem.id.as_str(),
                    mem.user_id.as_str(),
                    mem.content.as_str(),
                    blob.as_slice(),
                    crate::row::metadata_to_json(&mem.metadata).as_str(),
                    crate::row::ts_to_str(&mem.created_at).as_str(),
                    crate::row::ts_to_str(&mem.updated_at).as_str(),
                ),
            )
            .await
            .map_err(|e| StorageError::Other(format!("save memory: {e}")))?;
        Ok(())
    }

    async fn search_memory(
        &self,
        user_id: &str,
        vector: &[f32],
        limit: usize,
    ) -> Result<Vec<(Memory, f32)>, StorageError> {
        self.check_dim(vector)?;
        let blob = crate::row::f32_blob(vector);
        let sql = format!(
            "SELECT {COLUMNS}, vector_distance_cos(embedding, ?1) AS score
             FROM memories WHERE user_id = ?2
             ORDER BY score ASC LIMIT ?3"
        );
        let mut rows = self
            .conn
            .query(&sql, (blob.as_slice(), user_id, limit as i64))
            .await
            .map_err(|e| StorageError::Other(format!("search memories: {e}")))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| StorageError::Other(format!("iterate search: {e}")))?
        {
            // vector_distance_cos returns cosine DISTANCE (0 = identical);
            // the core contract is SIMILARITY (1 = identical, thresholded at 0.85).
            let distance: f64 = row
                .get(7)
                .map_err(|e| StorageError::Other(format!("read score: {e}")))?;
            let mut mem = memory_from_row(&row)?;
            mem.vector = crate::row::blob_f32(
                &row.get::<Vec<u8>>(3)
                    .map_err(|e| StorageError::Other(format!("read vector from search: {e}")))?,
            );
            out.push((mem, (1.0 - distance) as f32));
        }
        Ok(out)
    }

    async fn list_memories(
        &self,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<Memory>, StorageError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM memories WHERE user_id = ?1 ORDER BY created_at DESC LIMIT ?2"
        );
        let mut rows = self
            .conn
            .query(&sql, (user_id, limit as i64))
            .await
            .map_err(|e| StorageError::Other(format!("list memories: {e}")))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| StorageError::Other(format!("iterate list: {e}")))?
        {
            out.push(memory_from_row(&row)?);
        }
        Ok(out)
    }

    async fn get_memory(&self, memory_id: &str) -> Result<Memory, StorageError> {
        let sql = format!("SELECT {COLUMNS} FROM memories WHERE id = ?1");
        let mut rows = self
            .conn
            .query(&sql, vec![memory_id])
            .await
            .map_err(|e| StorageError::Other(format!("get memory: {e}")))?;
        match rows
            .next()
            .await
            .map_err(|e| StorageError::Other(format!("iterate get: {e}")))?
        {
            Some(row) => memory_from_row(&row),
            None => Err(StorageError::Other(format!("memory {memory_id} not found"))),
        }
    }

    async fn delete_memory(&self, memory_id: &str) -> Result<(), StorageError> {
        self.conn
            .execute("DELETE FROM memories WHERE id = ?1", vec![memory_id])
            .await
            .map_err(|e| StorageError::Other(format!("delete memory: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use memayu_core::{Memory, StorageProvider};
    use std::collections::HashMap;

    fn mem(id: &str, user_id: &str, content: &str, v: &[f32]) -> Memory {
        Memory {
            id: id.to_string(),
            user_id: user_id.to_string(),
            content: content.to_string(),
            vector: v.to_vec(),
            metadata: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn crud_roundtrip() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        let m = mem("m1", "u1", "User lives in Jakarta", &[1.0, 0.0, 0.0]);
        provider.save_memory(&m).await.unwrap();

        let list = provider.list_memories("u1", 10).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].content, "User lives in Jakarta");
        assert_eq!(list[0].vector, vec![1.0, 0.0, 0.0]);

        provider.delete_memory("m1").await.unwrap();
        assert!(provider.list_memories("u1", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn upsert_replaces_by_id() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        let m = mem("m1", "u1", "old", &[1.0, 0.0, 0.0]);
        provider.save_memory(&m).await.unwrap();
        let m2 = Memory {
            content: "User moved to Bandung".into(),
            vector: vec![0.0, 1.0, 0.0],
            updated_at: Utc::now(),
            ..m.clone()
        };
        provider.save_memory(&m2).await.unwrap();
        let list = provider.list_memories("u1", 10).await.unwrap();
        assert_eq!(list.len(), 1, "upsert must not create a second row");
        assert_eq!(list[0].content, "User moved to Bandung");
    }

    #[tokio::test]
    async fn search_returns_ranked_scores() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        provider
            .save_memory(&mem("m1", "u1", "jakarta", &[1.0, 0.0, 0.0]))
            .await
            .unwrap();
        provider
            .save_memory(&mem("m2", "u1", "bandung", &[0.0, 1.0, 0.0]))
            .await
            .unwrap();
        provider
            .save_memory(&mem("m3", "u2", "other user", &[0.0, 0.0, 1.0]))
            .await
            .unwrap();

        let results = provider
            .search_memory("u1", &[0.9, 0.1, 0.0], 3)
            .await
            .unwrap();
        assert_eq!(results.len(), 2, "scoped to user u1 only");
        assert_eq!(results[0].0.id, "m1");
        assert!(
            results[0].1 > results[1].1,
            "similarity descending, m1 most similar"
        );
        assert!(
            results[0].1 > 0.8,
            "identical vector has near-1.0 similarity"
        );
    }

    #[tokio::test]
    async fn dimension_mismatch_rejected() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        let m = mem("m1", "u1", "wrong dim", &[1.0, 0.0]);
        let err = provider.save_memory(&m).await.unwrap_err();
        assert!(matches!(
            err,
            StorageError::DimensionMismatch {
                expected: 3,
                got: 2
            }
        ));
    }
}
