use chrono::{DateTime, Utc};
use memayu_core::{Memory, StorageError};
use std::collections::HashMap;

/// Serialize a vector as little-endian f32 bytes (libSQL F32_BLOB format).
pub fn f32_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

pub fn blob_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub fn ts_to_str(ts: &DateTime<Utc>) -> String {
    ts.to_rfc3339()
}

fn str_to_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

pub fn metadata_to_json(meta: &HashMap<String, String>) -> String {
    serde_json::to_string(meta).unwrap_or_else(|_| "{}".to_string())
}

fn json_to_metadata(s: &str) -> HashMap<String, String> {
    serde_json::from_str(s).unwrap_or_default()
}

pub struct RowColumns {
    pub id: String,
    pub user_id: String,
    pub content: String,
    pub metadata: String,
    pub created_at: String,
    pub updated_at: String,
    pub vector: Vec<u8>,
}

pub fn memory_from_row(row: &libsql::Row) -> Result<Memory, StorageError> {
    let c = RowColumns {
        id: row
            .get(0)
            .map_err(|e| StorageError::Other(format!("read id: {e}")))?,
        user_id: row
            .get(1)
            .map_err(|e| StorageError::Other(format!("read user_id: {e}")))?,
        content: row
            .get(2)
            .map_err(|e| StorageError::Other(format!("read content: {e}")))?,
        vector: row
            .get(3)
            .map_err(|e| StorageError::Other(format!("read vector: {e}")))?,
        metadata: row
            .get(4)
            .map_err(|e| StorageError::Other(format!("read metadata: {e}")))?,
        created_at: row
            .get(5)
            .map_err(|e| StorageError::Other(format!("read created_at: {e}")))?,
        updated_at: row
            .get(6)
            .map_err(|e| StorageError::Other(format!("read updated_at: {e}")))?,
    };
    Ok(Memory {
        id: c.id,
        user_id: c.user_id,
        content: c.content,
        vector: blob_f32(&c.vector),
        metadata: json_to_metadata(&c.metadata),
        created_at: str_to_ts(&c.created_at),
        updated_at: str_to_ts(&c.updated_at),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── f32_blob / blob_f32 roundtrip ──

    #[test]
    fn f32_blob_roundtrip() {
        let v = vec![1.0_f32, 0.0, -3.5, 42.0];
        let blob = f32_blob(&v);
        assert_eq!(blob.len(), 16); // 4 * 4 bytes
        let back = blob_f32(&blob);
        assert_eq!(back, v);
    }

    #[test]
    fn f32_blob_empty() {
        let blob = f32_blob(&[]);
        assert!(blob.is_empty());
        let back = blob_f32(&blob);
        assert!(back.is_empty());
    }

    #[test]
    fn blob_f32_handles_exact_chunks() {
        let v = vec![0.5_f32, -0.25];
        let blob = f32_blob(&v);
        assert_eq!(blob.len(), 8);
        let back = blob_f32(&blob);
        assert_eq!(back, v);
    }

    // ── metadata serialization ──

    #[test]
    fn metadata_to_json_roundtrip() {
        let mut m = HashMap::new();
        m.insert("key".into(), "value".into());
        let json = metadata_to_json(&m);
        assert!(json.contains("key"));
        assert!(json.contains("value"));
    }

    #[test]
    fn metadata_to_json_empty() {
        let m: HashMap<String, String> = HashMap::new();
        let json = metadata_to_json(&m);
        assert_eq!(json, "{}");
    }

    // ── ts_to_str produces valid format ──

    #[test]
    fn ts_to_str_is_rfc3339() {
        let now = chrono::Utc::now();
        let s = ts_to_str(&now);
        assert!(chrono::DateTime::parse_from_rfc3339(&s).is_ok());
    }
}
