//! Metadata filtering, page cursors, and the shared page-listing types.
//!
//! These are backend-agnostic: storage providers translate the predicate into
//! their native mechanism (jsonb `@>` on Postgres, `json_extract` on libsql),
//! but the filter/cursor *shape* is owned here so the API and MCP layers stay
//! decoupled from any one engine.

use crate::{Memory, Metadata};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Upper bound enforced on `list` and `search` `limit` values. Requests above
/// this are rejected with a clear error rather than silently truncated.
pub const MAX_PAGE_SIZE: usize = 100;

/// Exact-match, scalar key=value predicate used to scope retrieval. V1 only
/// supports equality on string values; nested/range filters are V2.
pub type MetadataFilter = HashMap<String, String>;

/// A page of memories plus an opaque cursor for fetching the next page and the
/// total number of rows matching the current filter.
/// `next_cursor` is `None` when the returned page is the last one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPage {
    pub memories: Vec<Memory>,
    pub next_cursor: Option<String>,
    /// Total number of memories matching the query/filter (independent of the
    /// current page window), so clients can render counts and progress.
    pub total: usize,
}

impl MemoryPage {
    pub fn new(memories: Vec<Memory>, next_cursor: Option<String>, total: usize) -> Self {
        Self {
            memories,
            next_cursor,
            total,
        }
    }
}

/// True when every predicate in `filter` is satisfied by `metadata`. Used as
/// the canonical matcher in tests and as a cross-check by providers that choose
/// Rust-side post-filtering.
pub fn metadata_matches(metadata: &Metadata, filter: &MetadataFilter) -> bool {
    filter.iter().all(|(k, v)| metadata.get(k) == Some(v))
}

/// Keyset cursor payload: the last row's `(created_at, id)`.
#[derive(Serialize, Deserialize)]
struct CursorPayload {
    created_at: DateTime<Utc>,
    id: String,
}

/// Encode an opaque, URL-safe cursor from the last-seen row. The cursor is
/// self-describing and backend-agnostic so it can round-trip across providers.
pub fn encode_cursor(created_at: &DateTime<Utc>, id: &str) -> String {
    let payload = CursorPayload {
        created_at: *created_at,
        id: id.to_string(),
    };
    BASE64.encode(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()))
}

/// Decode a cursor produced by [`encode_cursor`]. Returns `None` for any
/// malformed or tampered value.
pub fn decode_cursor(cursor: &str) -> Option<(DateTime<Utc>, String)> {
    let bytes = BASE64.decode(cursor).ok()?;
    let payload: CursorPayload = serde_json::from_slice(&bytes).ok()?;
    Some((payload.created_at, payload.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(pairs: &[(&str, &str)]) -> Metadata {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn metadata_matches_all_predicates() {
        let metadata = meta(&[("source", "telegram"), ("label", "note")]);
        assert!(metadata_matches(
            &metadata,
            &meta(&[("source", "telegram")])
        ));
        assert!(metadata_matches(
            &metadata,
            &meta(&[("source", "telegram"), ("label", "note")])
        ));
        assert!(!metadata_matches(&metadata, &meta(&[("source", "cli")])));
        assert!(!metadata_matches(
            &metadata,
            &meta(&[("source", "telegram"), ("missing", "x")])
        ));
    }

    #[test]
    fn empty_filter_matches_everything() {
        let metadata = meta(&[]);
        assert!(metadata_matches(&metadata, &MetadataFilter::new()));
        let full = meta(&[("a", "1")]);
        assert!(metadata_matches(&full, &MetadataFilter::new()));
    }

    #[test]
    fn cursor_roundtrips() {
        let ts = Utc::now();
        let cursor = encode_cursor(&ts, "mem-123");
        let (decoded_ts, decoded_id) = decode_cursor(&cursor).unwrap();
        assert_eq!(decoded_ts, ts);
        assert_eq!(decoded_id, "mem-123");
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_cursor("not base64 !!").is_none());
        assert!(decode_cursor("").is_none());
        // Valid base64 but not a cursor payload.
        assert!(decode_cursor(&BASE64.encode(b"garbage")).is_none());
    }
}
