use libsql::Connection;
use memayu_core::StorageError;

/// Create the schema if missing; return the actual stored embedding dimension
/// (parsed from the column type, which may predate this open).
pub async fn create_schema(conn: &Connection, dimension: usize) -> Result<usize, StorageError> {
    let dim = dimension.max(1);
    let mut existing = None;
    let mut rows = conn
        .query(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories'",
            (),
        )
        .await
        .map_err(|e| StorageError::Other(format!("query schema: {e}")))?;
    if let Some(row) = rows
        .next()
        .await
        .map_err(|e| StorageError::Other(format!("read schema row: {e}")))?
    {
        let sql: String = row
            .get(0)
            .map_err(|e| StorageError::Other(format!("read schema sql: {e}")))?;
        existing = parse_dim_from_ddl(&sql);
    }
    while rows
        .next()
        .await
        .map_err(|e| StorageError::Other(format!("drain schema rows: {e}")))?
        .is_some()
    {}

    let stored_dim = match existing {
        Some(d) if d == dim => d,
        Some(d) => {
            return Err(StorageError::DimensionMismatch {
                expected: d,
                got: dim,
            })
        }
        None => {
            let ddl = format!(
                "CREATE TABLE memories (
                    id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    content TEXT NOT NULL,
                    embedding FLOAT32({dim}) NOT NULL,
                    metadata TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                )"
            );
            conn.execute(&ddl, ())
                .await
                .map_err(|e| StorageError::Other(format!("create memories table: {e}")))?;
            // DiskANN index for ANN search (ARCHITECTURE.md §2/§3).
            conn.execute(
                "CREATE INDEX memories_idx ON memories (libsql_vector_idx(embedding))",
                (),
            )
            .await
            .map_err(|e| StorageError::Other(format!("create vector index: {e}")))?;
            dim
        }
    };
    create_fts(conn).await?;
    Ok(stored_dim)
}

/// Create the FTS5 virtual table used for the full-text leg of hybrid search.
pub async fn create_fts(conn: &Connection) -> Result<(), StorageError> {
    const FTS_DDL: &str = "CREATE VIRTUAL TABLE memories_fts \
        USING fts5(content, id UNINDEXED, user_id UNINDEXED, tokenize = 'porter unicode61')";

    let existing: Option<String> = {
        let mut rows = conn
            .query(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories_fts'",
                (),
            )
            .await
            .map_err(|e| StorageError::Other(format!("query memories_fts schema: {e}")))?;
        let mut existing = None;
        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| StorageError::Other(format!("read memories_fts schema: {e}")))?
        {
            existing = Some(
                row.get(0)
                    .map_err(|e| StorageError::Other(format!("read memories_fts ddl: {e}")))?,
            );
        }
        while rows
            .next()
            .await
            .map_err(|e| StorageError::Other(format!("drain memories_fts schema: {e}")))?
            .is_some()
        {}
        existing
    };

    match existing.as_deref() {
        Some(ddl) if ddl.contains("porter") => {}
        Some(_) => {
            let tx = conn
                .transaction()
                .await
                .map_err(|e| StorageError::Other(format!("begin fts migration txn: {e}")))?;
            tx.execute("DROP TABLE memories_fts", ())
                .await
                .map_err(|e| StorageError::Other(format!("drop memories_fts: {e}")))?;
            tx.execute(FTS_DDL, ())
                .await
                .map_err(|e| StorageError::Other(format!("create memories_fts table: {e}")))?;
            tx.commit()
                .await
                .map_err(|e| StorageError::Other(format!("commit fts migration txn: {e}")))?;
            backfill_fts(conn).await?;
        }
        None => {
            conn.execute(FTS_DDL, ())
                .await
                .map_err(|e| StorageError::Other(format!("create memories_fts table: {e}")))?;
            backfill_fts(conn).await?;
        }
    }
    Ok(())
}

async fn backfill_fts(conn: &Connection) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO memories_fts (content, id, user_id)
         SELECT content, id, user_id FROM memories
         WHERE id NOT IN (SELECT id FROM memories_fts)",
        (),
    )
    .await
    .map_err(|e| StorageError::Other(format!("backfill memories_fts: {e}")))?;
    Ok(())
}

/// Extract the dimension from `embedding FLOAT32(N)` in a CREATE TABLE DDL.
fn parse_dim_from_ddl(sql: &str) -> Option<usize> {
    let lower = sql.to_ascii_lowercase();
    let pos = lower.find("embedding float32(")?;
    let rest = &lower[pos + "embedding float32(".len()..];
    let end = rest.find(')')?;
    rest[..end].trim().parse().ok()
}

/// Read the stored embedding dimension from an existing `memories` table
/// without creating the schema. Returns `None` if the table does not exist.
pub async fn stored_dimension(conn: &Connection) -> Result<Option<usize>, StorageError> {
    let mut rows = conn
        .query(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories'",
            (),
        )
        .await
        .map_err(|e| StorageError::Other(format!("query schema: {e}")))?;
    if let Some(row) = rows
        .next()
        .await
        .map_err(|e| StorageError::Other(format!("read schema row: {e}")))?
    {
        let sql: String = row
            .get(0)
            .map_err(|e| StorageError::Other(format!("read schema sql: {e}")))?;
        return Ok(parse_dim_from_ddl(&sql));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dim_from_ddl_works() {
        let ddl = "CREATE TABLE memories (\n  embedding FLOAT32(768) NOT NULL,\n)";
        assert_eq!(parse_dim_from_ddl(ddl), Some(768));
        assert_eq!(parse_dim_from_ddl("CREATE TABLE x (a TEXT)"), None);
    }
}
