use crate::row::memory_from_row;
use crate::schema::create_schema;
use async_trait::async_trait;
use libsql::Connection;
use memayu_core::{
    decode_cursor, encode_cursor, Memory, MemoryPage, MetadataFilter, StorageError, StorageProvider,
};

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

fn sanitize_fts5_query(raw: &str) -> String {
    format!("\"{}\"", raw.replace('"', "\"\""))
}

/// SQL fragment for exact key=value metadata predicates, using one `?` per
/// path and per value. Must be paired with a matching sequence of bound params
/// in the same order (path, value, path, value, ...).
fn metadata_filter_clause(filter: &MetadataFilter) -> String {
    let mut clause = String::new();
    for _ in filter.keys() {
        clause.push_str(" AND json_extract(metadata, ?) = ?");
    }
    clause
}

fn push_metadata_params(params: &mut Vec<libsql::Value>, filter: &MetadataFilter) {
    for (key, value) in filter {
        // Quote the key so dots, spaces, and other path characters are treated
        // as literal JSON member names rather than path separators.
        params.push(format!("$.\"{key}\"").into());
        params.push(value.as_str().into());
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let (dot, norm_a, norm_b) = a
        .iter()
        .zip(b.iter())
        .fold((0.0f32, 0.0f32, 0.0f32), |(d, na, nb), (&x, &y)| {
            (d + x * y, na + x * x, nb + y * y)
        });
    let denom = (norm_a * norm_b).sqrt();
    if denom == 0.0 {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

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
        self.upsert_fts(mem).await?;
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
        let blob = crate::row::f32_blob(vector);
        let mut sql = format!(
            "SELECT {COLUMNS}, vector_distance_cos(embedding, ?1) AS score
             FROM memories WHERE user_id = ?2"
        );
        let mut params: Vec<libsql::Value> = vec![blob.clone().into(), user_id.into()];
        if let Some(f) = filter {
            sql.push_str(&metadata_filter_clause(f));
            push_metadata_params(&mut params, f);
        }
        sql.push_str(" ORDER BY score ASC LIMIT ?");
        params.push((limit as i64).into());

        let mut rows = self
            .conn
            .query(&sql, params)
            .await
            .map_err(|e| StorageError::Other(format!("search memories: {e}")))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| StorageError::Other(format!("iterate search: {e}")))?
        {
            let mut mem = memory_from_row(&row)?;
            mem.vector = crate::row::blob_f32(
                &row.get::<Vec<u8>>(3)
                    .map_err(|e| StorageError::Other(format!("read vector from search: {e}")))?,
            );
            // Compute true cosine similarity from the returned vectors.
            // vector_distance_cos is only used for ORDER BY (ANN index scan);
            // the actual score is computed directly from the vectors here.
            let similarity = cosine_similarity(vector, &mem.vector);
            out.push((mem, similarity));
        }
        // Re-sort by similarity descending since we computed our own scores.
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(out)
    }

    async fn search_fulltext(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
        filter: Option<&MetadataFilter>,
    ) -> Result<Vec<(Memory, f32)>, StorageError> {
        // BM25 is the native FTS5 relevance signal (higher = better). The
        // column ORDER must match `COLUMNS` so `memory_from_row` can read it.
        let mut sql = "SELECT m.id, m.user_id, m.content, m.embedding, m.metadata,
                          m.created_at, m.updated_at, bm25(memories_fts) AS score
                   FROM memories_fts
                   JOIN memories m ON m.id = memories_fts.id
                   WHERE memories_fts MATCH ?1 AND m.user_id = ?2"
            .to_string();
        let mut params: Vec<libsql::Value> = vec![query.into(), user_id.into()];
        if let Some(f) = filter {
            sql.push_str(&metadata_filter_clause(f));
            push_metadata_params(&mut params, f);
        }
        sql.push_str(" ORDER BY score ASC LIMIT ?");
        params.push((limit as i64).into());
        // Treat the query as a literal phrase so FTS5 special characters in
        // ordinary user input ($, !, ", *, :, ^, AND/OR/NOT) cannot crash MATCH.
        let fts_query = sanitize_fts5_query(query);
        params[0] = fts_query.into();
        let mut rows = self
            .conn
            .query(&sql, params)
            .await
            .map_err(|e| StorageError::Other(format!("full-text search: {e}")))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| StorageError::Other(format!("iterate full-text search: {e}")))?
        {
            let mut mem = memory_from_row(&row)?;
            mem.vector = crate::row::blob_f32(
                &row.get::<Vec<u8>>(3)
                    .map_err(|e| StorageError::Other(format!("read vector from fts: {e}")))?,
            );
            let score: f64 = row
                .get(7)
                .map_err(|e| StorageError::Other(format!("read fts score: {e}")))?;
            // BM25 returns smaller scores for better matches; negate so the
            // fused RRF ordering is "higher = better" like the vector leg.
            out.push((mem, (-score) as f32));
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
        // Fetch one extra row so we can detect whether another page exists and
        // derive the opaque next cursor from the last visible row.
        let fetch = limit as i64 + 1;
        let mut sql = format!("SELECT {COLUMNS} FROM memories WHERE user_id = ?1");
        let mut params: Vec<libsql::Value> = vec![user_id.into()];
        if let Some(f) = filter {
            sql.push_str(&metadata_filter_clause(f));
            push_metadata_params(&mut params, f);
        }
        if let Some(c) = cursor {
            let (ts, id) = decode_cursor(c).ok_or_else(|| {
                StorageError::InvalidCursor("invalid pagination cursor".to_string())
            })?;
            let ts_str = crate::row::ts_to_str(&ts);
            sql.push_str(
                " AND (julianday(created_at) < julianday(?) \
                 OR (julianday(created_at) = julianday(?) AND id < ?))",
            );
            params.push(ts_str.clone().into());
            params.push(ts_str.into());
            params.push(id.into());
        }
        sql.push_str(" ORDER BY julianday(created_at) DESC, id DESC LIMIT ?");
        params.push(fetch.into());

        let mut rows = self
            .conn
            .query(&sql, params)
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
        self.conn
            .execute("DELETE FROM memories_fts WHERE id = ?1", vec![memory_id])
            .await
            .map_err(|e| StorageError::Other(format!("delete memory from fts: {e}")))?;
        Ok(())
    }
}

impl LibsqlProvider {
    /// Keep the FTS5 index in lockstep with `memories`. Delete-then-insert is
    /// idempotent for both INSERT and UPDATE paths, so a single helper covers
    /// both (issue #20).
    async fn upsert_fts(&self, mem: &Memory) -> Result<(), StorageError> {
        self.conn
            .execute(
                "DELETE FROM memories_fts WHERE id = ?1",
                vec![mem.id.as_str()],
            )
            .await
            .map_err(|e| StorageError::Other(format!("clear fts row: {e}")))?;
        self.conn
            .execute(
                "INSERT INTO memories_fts (content, id, user_id) VALUES (?1, ?2, ?3)",
                (mem.content.as_str(), mem.id.as_str(), mem.user_id.as_str()),
            )
            .await
            .map_err(|e| StorageError::Other(format!("index memory in fts: {e}")))?;
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

        let list = provider
            .list_memories("u1", 10, None, None)
            .await
            .unwrap()
            .memories;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].content, "User lives in Jakarta");
        assert_eq!(list[0].vector, vec![1.0, 0.0, 0.0]);

        provider.delete_memory("m1").await.unwrap();
        assert!(provider
            .list_memories("u1", 10, None, None)
            .await
            .unwrap()
            .memories
            .is_empty());
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
        let list = provider
            .list_memories("u1", 10, None, None)
            .await
            .unwrap()
            .memories;
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
            .search_memory("u1", &[0.9, 0.1, 0.0], 3, None)
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

    #[tokio::test]
    async fn fulltext_search_matches_and_ranks() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        provider
            .save_memory(&mem("m1", "u1", "User lives in Jakarta", &[1.0, 0.0, 0.0]))
            .await
            .unwrap();
        provider
            .save_memory(&mem(
                "m2",
                "u1",
                "User likes Bandung coffee",
                &[0.0, 1.0, 0.0],
            ))
            .await
            .unwrap();
        provider
            .save_memory(&mem(
                "m3",
                "u2",
                "User lives in Jakarta too",
                &[0.0, 0.0, 1.0],
            ))
            .await
            .unwrap();

        let results = provider
            .search_fulltext("u1", "Jakarta", 5, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 1, "only u1 rows match, u2 is excluded");
        assert_eq!(results[0].0.id, "m1");
    }

    #[test]
    fn sanitize_fts5_query_wraps_query_in_escaped_literal_phrase() {
        assert_eq!(sanitize_fts5_query("Jakarta"), "\"Jakarta\"");
        assert_eq!(
            sanitize_fts5_query(r#"he said "hi" $5*:^! OR"#),
            r#""he said ""hi"" $5*:^! OR""#
        );
        assert_eq!(sanitize_fts5_query(""), "\"\"");
    }

    #[tokio::test]
    async fn fulltext_search_special_characters_are_literal() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        provider
            .save_memory(&mem(
                "m1",
                "u1",
                "User's project costs $500! It has an AND and OR and NOT flag",
                &[1.0, 0.0, 0.0],
            ))
            .await
            .unwrap();
        provider
            .save_memory(&mem(
                "m2",
                "u1",
                "plain unrelated content",
                &[0.0, 1.0, 0.0],
            ))
            .await
            .unwrap();

        let results = provider
            .search_fulltext("u1", "User's project costs $500!", 5, None)
            .await
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "special-char query is treated as literal text"
        );
        assert_eq!(results[0].0.id, "m1");

        let results = provider
            .search_fulltext("u1", "cats AND dogs OR birds NOT fish", 5, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 0, "boolean keywords are literal, no crash");
    }

    #[tokio::test]
    async fn fulltext_search_emoji_and_non_ascii_input() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        provider
            .save_memory(&mem(
                "m1",
                "u1",
                "plan 🚀 Q3 launch 💡 ideas",
                &[1.0, 0.0, 0.0],
            ))
            .await
            .unwrap();

        let results = provider
            .search_fulltext("u1", "plan 🚀 Q3 launch 💡 ideas!", 5, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 1, "emoji query is treated as literal text");
        assert_eq!(results[0].0.id, "m1");
    }

    #[tokio::test]
    async fn hybrid_search_fuses_vector_and_fulltext_signals() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        // m1 is the top vector hit but does not match the full-text keyword;
        // m2 is a weaker vector hit that the full-text leg also matches.
        provider
            .save_memory(&mem("m1", "u1", "alpha bravo", &[1.0, 0.0, 0.0]))
            .await
            .unwrap();
        provider
            .save_memory(&mem("m2", "u1", "charlie keyword", &[0.0, 1.0, 0.0]))
            .await
            .unwrap();

        let vector_hits = provider
            .search_memory("u1", &[0.9, 0.1, 0.0], 10, None)
            .await
            .unwrap();
        assert_eq!(vector_hits[0].0.id, "m1", "m1 is the top vector hit");
        let fulltext_hits = provider
            .search_fulltext("u1", "charlie", 10, None)
            .await
            .unwrap();
        assert_eq!(fulltext_hits[0].0.id, "m2", "full-text matches m2 only");

        let fused = memayu_core::fusion::fuse(&vector_hits, &fulltext_hits, 10);
        assert_eq!(
            fused[0].0.id, "m2",
            "RRF boost from the full-text leg reorders m2 above the top vector hit"
        );
    }

    #[tokio::test]
    async fn fulltext_index_updates_with_content() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        let m = mem("m1", "u1", "original wording", &[1.0, 0.0, 0.0]);
        provider.save_memory(&m).await.unwrap();
        assert!(
            provider
                .search_fulltext("u1", "wording", 5, None)
                .await
                .unwrap()
                .len()
                == 1
        );

        let updated = Memory {
            content: "renamed phrasing".into(),
            updated_at: Utc::now(),
            ..m.clone()
        };
        provider.save_memory(&updated).await.unwrap();

        assert!(
            provider
                .search_fulltext("u1", "wording", 5, None)
                .await
                .unwrap()
                .is_empty(),
            "stale content must be removed from the index"
        );
        assert_eq!(
            provider
                .search_fulltext("u1", "phrasing", 5, None)
                .await
                .unwrap()[0]
                .0
                .id,
            "m1"
        );
    }

    #[tokio::test]
    async fn backfills_fts_for_preexisting_memories() {
        // Simulate an existing self-hosted database created before the FTS5
        // table existed: write the old `memories` schema plus one row, then
        // reopen through LibsqlProvider. The upgrade path must backfill the
        // row so the full-text leg can find it without a re-save.
        let path = std::env::temp_dir().join(format!(
            "memayu-fts-backfill-{}-{}.db",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let path_str = path.to_string_lossy().to_string();

        {
            let db = libsql::Builder::new_local(&path_str).build().await.unwrap();
            let conn = db.connect().unwrap();
            conn.execute(
                "CREATE TABLE memories (
                    id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    content TEXT NOT NULL,
                    embedding FLOAT32(3) NOT NULL,
                    metadata TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                )",
                (),
            )
            .await
            .unwrap();
            conn.execute(
                "INSERT INTO memories
                    (id, user_id, content, embedding, metadata, created_at, updated_at)
                 VALUES
                    ('legacy1', 'u1', 'legacy jakarta note', X'0000803F0000000000000000',
                     '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                (),
            )
            .await
            .unwrap();
        } // drop the old-schema database and close the file

        let provider = LibsqlProvider::open(&path_str, 3).await.unwrap();
        let results = provider
            .search_fulltext("u1", "jakarta", 5, None)
            .await
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "pre-upgrade row must be backfilled into the FTS index"
        );
        assert_eq!(results[0].0.id, "legacy1");

        drop(provider);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn porter_tokenizer_stems_common_inflections() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        provider
            .save_memory(&mem("m1", "u1", "She works in Jakarta", &[1.0, 0.0, 0.0]))
            .await
            .unwrap();
        provider
            .save_memory(&mem(
                "m2",
                "u1",
                "I am running the marathon",
                &[0.0, 1.0, 0.0],
            ))
            .await
            .unwrap();
        provider
            .save_memory(&mem(
                "m3",
                "u1",
                "We preferred this option",
                &[0.0, 0.0, 1.0],
            ))
            .await
            .unwrap();

        let work = provider
            .search_fulltext("u1", "work", 5, None)
            .await
            .unwrap();
        assert!(
            work.iter().any(|(m, _)| m.id == "m1"),
            "query 'work' must match stored 'works' via the full-text leg"
        );

        let run = provider
            .search_fulltext("u1", "run", 5, None)
            .await
            .unwrap();
        assert!(
            run.iter().any(|(m, _)| m.id == "m2"),
            "query 'run' must match stored 'running' via the full-text leg"
        );

        let prefer = provider
            .search_fulltext("u1", "prefer", 5, None)
            .await
            .unwrap();
        assert!(
            prefer.iter().any(|(m, _)| m.id == "m3"),
            "query 'prefer' must match stored 'preferred' via the full-text leg"
        );
    }

    #[tokio::test]
    async fn porter_tokenizer_migrates_old_table_and_is_idempotent() {
        let path = std::env::temp_dir().join(format!(
            "memayu-fts-porter-{}-{}.db",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let path_str = path.to_string_lossy().to_string();

        {
            let db = libsql::Builder::new_local(&path_str).build().await.unwrap();
            let conn = db.connect().unwrap();
            conn.execute(
                "CREATE TABLE memories (
                    id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    content TEXT NOT NULL,
                    embedding FLOAT32(3) NOT NULL,
                    metadata TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                )",
                (),
            )
            .await
            .unwrap();
            conn.execute(
                "CREATE VIRTUAL TABLE memories_fts
                 USING fts5(content, id UNINDEXED, user_id UNINDEXED)",
                (),
            )
            .await
            .unwrap();
            conn.execute(
                "INSERT INTO memories
                    (id, user_id, content, embedding, metadata, created_at, updated_at)
                 VALUES
                    ('legacy1', 'u1', 'She works in Jakarta', X'0000803F0000000000000000',
                     '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                (),
            )
            .await
            .unwrap();
            conn.execute(
                "INSERT INTO memories_fts (content, id, user_id)
                 VALUES ('She works in Jakarta', 'legacy1', 'u1')",
                (),
            )
            .await
            .unwrap();
        }

        let provider = LibsqlProvider::open(&path_str, 3).await.unwrap();
        let results = provider
            .search_fulltext("u1", "work", 5, None)
            .await
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "pre-existing rows must be re-indexed under the porter tokenizer"
        );
        assert_eq!(results[0].0.id, "legacy1");
        drop(provider);

        let provider = LibsqlProvider::open(&path_str, 3).await.unwrap();
        let results = provider
            .search_fulltext("u1", "work", 5, None)
            .await
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "repeated open must preserve the migrated, stemming index"
        );
        assert_eq!(results[0].0.id, "legacy1");
        drop(provider);

        let _ = std::fs::remove_file(&path);
    }

    fn mem_with_meta(
        id: &str,
        user_id: &str,
        content: &str,
        v: &[f32],
        meta: &[(&str, &str)],
    ) -> Memory {
        let mut metadata = HashMap::new();
        for (k, val) in meta {
            metadata.insert((*k).to_string(), (*val).to_string());
        }
        Memory {
            metadata,
            ..mem(id, user_id, content, v)
        }
    }

    #[tokio::test]
    async fn metadata_filter_restricts_vector_search() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        provider
            .save_memory(&mem_with_meta(
                "m1",
                "u1",
                "alpha",
                &[1.0, 0.0, 0.0],
                &[("project", "onchain")],
            ))
            .await
            .unwrap();
        provider
            .save_memory(&mem_with_meta(
                "m2",
                "u1",
                "beta",
                &[0.0, 1.0, 0.0],
                &[("project", "mobile")],
            ))
            .await
            .unwrap();

        let mut filter = HashMap::new();
        filter.insert("project".to_string(), "onchain".to_string());
        let results = provider
            .search_memory("u1", &[0.9, 0.1, 0.0], 10, Some(&filter))
            .await
            .unwrap();
        assert_eq!(results.len(), 1, "filter narrows vector hits");
        assert_eq!(results[0].0.id, "m1");
    }

    #[tokio::test]
    async fn metadata_filter_restricts_fulltext_search() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        provider
            .save_memory(&mem_with_meta(
                "m1",
                "u1",
                "jakarta trip",
                &[1.0, 0.0, 0.0],
                &[("tier", "gold")],
            ))
            .await
            .unwrap();
        provider
            .save_memory(&mem_with_meta(
                "m2",
                "u1",
                "jakarta trip",
                &[0.0, 1.0, 0.0],
                &[("tier", "silver")],
            ))
            .await
            .unwrap();

        let mut filter = HashMap::new();
        filter.insert("tier".to_string(), "gold".to_string());
        let results = provider
            .search_fulltext("u1", "jakarta", 10, Some(&filter))
            .await
            .unwrap();
        assert_eq!(results.len(), 1, "filter narrows full-text hits");
        assert_eq!(results[0].0.id, "m1");
    }

    #[tokio::test]
    async fn metadata_filter_restricts_list() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        provider
            .save_memory(&mem_with_meta(
                "m1",
                "u1",
                "one",
                &[1.0, 0.0, 0.0],
                &[("env", "prod")],
            ))
            .await
            .unwrap();
        provider
            .save_memory(&mem_with_meta(
                "m2",
                "u1",
                "two",
                &[0.0, 1.0, 0.0],
                &[("env", "dev")],
            ))
            .await
            .unwrap();

        let mut filter = HashMap::new();
        filter.insert("env".to_string(), "prod".to_string());
        let page = provider
            .list_memories("u1", 10, None, Some(&filter))
            .await
            .unwrap();
        assert_eq!(page.memories.len(), 1);
        assert_eq!(page.memories[0].id, "m1");
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn list_paginates_with_opaque_cursor() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        for i in 0..5 {
            let m = Memory {
                id: format!("m{i}"),
                created_at: Utc::now() + chrono::Duration::seconds(i),
                updated_at: Utc::now() + chrono::Duration::seconds(i),
                ..mem(
                    &format!("m{i}"),
                    "u1",
                    &format!("note {i}"),
                    &[1.0, 0.0, 0.0],
                )
            };
            provider.save_memory(&m).await.unwrap();
        }

        let first = provider.list_memories("u1", 2, None, None).await.unwrap();
        assert_eq!(first.memories.len(), 2, "first page holds `limit` rows");
        let cursor = first.next_cursor.expect("more rows remain");
        let ids_first: Vec<&str> = first.memories.iter().map(|m| m.id.as_str()).collect();

        let second = provider
            .list_memories("u1", 2, Some(&cursor), None)
            .await
            .unwrap();
        assert_eq!(second.memories.len(), 2);
        let ids_second: Vec<&str> = second.memories.iter().map(|m| m.id.as_str()).collect();
        // Ordered created_at DESC across pages, no overlap.
        let mut all = ids_first.clone();
        all.extend(ids_second.iter().copied());
        assert_eq!(all, vec!["m4", "m3", "m2", "m1"]);

        let cursor2 = second.next_cursor.expect("more rows remain");
        let third = provider
            .list_memories("u1", 2, Some(&cursor2), None)
            .await
            .unwrap();
        assert_eq!(third.memories.len(), 1);
        assert_eq!(third.memories[0].id, "m0");
        assert!(third.next_cursor.is_none(), "last page has no cursor");
    }

    #[tokio::test]
    async fn list_rejects_invalid_cursor() {
        let provider = LibsqlProvider::open(":memory:", 3).await.unwrap();
        provider
            .save_memory(&mem("m1", "u1", "one", &[1.0, 0.0, 0.0]))
            .await
            .unwrap();
        let err = provider
            .list_memories("u1", 10, Some("not-a-valid-cursor"), None)
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::InvalidCursor(_)));
    }
}
