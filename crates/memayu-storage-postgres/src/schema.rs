use memayu_core::StorageError;
use sqlx::postgres::PgPool;

pub async fn current_dimension(pool: &PgPool) -> Result<Option<usize>, StorageError> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT format_type(a.atttypid, a.atttypmod)
         FROM pg_attribute a
         WHERE a.attrelid = 'memories'::regclass
           AND a.attname = 'embedding'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| StorageError::Other(format!("read column type: {e}")))?;
    Ok(row.and_then(|(t,)| {
        // format: "vector(1536)"
        t.and_then(|t| {
            let rest = t.strip_prefix("vector(")?;
            let end = rest.find(')')?;
            rest[..end].trim().parse().ok()
        })
    }))
}

pub async fn table_is_empty(pool: &PgPool) -> Result<bool, StorageError> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM memories")
        .fetch_one(pool)
        .await
        .map_err(|e| StorageError::Other(format!("count memories: {e}")))?;
    Ok(count == 0)
}

pub async fn set_dimension(pool: &PgPool, dimension: usize) -> Result<(), StorageError> {
    let sql = format!("ALTER TABLE memories ALTER COLUMN embedding TYPE vector({dimension})");
    sqlx::query(&sql)
        .execute(pool)
        .await
        .map_err(|e| StorageError::Other(format!("set vector dimension: {e}")))?;
    // Recreate the HNSW index for the new dimension.
    sqlx::query("DROP INDEX IF EXISTS memories_embedding_idx")
        .execute(pool)
        .await
        .map_err(|e| StorageError::Other(format!("drop hnsw index: {e}")))?;
    let sql =
        "CREATE INDEX memories_embedding_idx ON memories USING hnsw (embedding vector_cosine_ops)";
    sqlx::query(sql)
        .execute(pool)
        .await
        .map_err(|e| StorageError::Other(format!("recreate hnsw index: {e}")))?;
    Ok(())
}
