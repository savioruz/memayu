//! Reciprocal Rank Fusion (RRF) shared by every storage backend.
//!
//! Hybrid search combines two ranking signals (vector similarity and
//! full-text relevance) without mixing their incompatible score scales.
//! RRF scores a document by `1 / (k + rank)` per list, then sums the
//! contributions. `k` (default 60) dampens the influence of top-ranked
//! outliers. This module lives in `memayu-core` so both Postgres and libSQL
//! backends use byte-identical fusion logic (issue #20 requirement).

use crate::Memory;

/// Standard RRF damping constant.
pub const RRF_K: f64 = 60.0;

/// Fuse two ranked lists into a single `(Memory, score)` list, sorted by
/// fused score descending. Scores from the input lists are ignored; only
/// their rank order matters.
pub fn fuse(
    vector: &[(Memory, f32)],
    fulltext: &[(Memory, f32)],
    limit: usize,
) -> Vec<(Memory, f32)> {
    let mut scores: std::collections::HashMap<&str, (Memory, f64)> =
        std::collections::HashMap::new();

    for (rank, (mem, _)) in vector.iter().enumerate() {
        let entry = scores
            .entry(mem.id.as_str())
            .or_insert_with(|| (mem.clone(), 0.0));
        entry.1 += 1.0 / (RRF_K + rank as f64 + 1.0);
    }
    for (rank, (mem, _)) in fulltext.iter().enumerate() {
        let entry = scores
            .entry(mem.id.as_str())
            .or_insert_with(|| (mem.clone(), 0.0));
        entry.1 += 1.0 / (RRF_K + rank as f64 + 1.0);
    }

    let mut out: Vec<(Memory, f32)> = scores
        .into_values()
        .map(|(mem, score)| (mem, score as f32))
        .collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn mem(id: &str) -> Memory {
        Memory {
            id: id.to_string(),
            user_id: "u1".to_string(),
            content: id.to_string(),
            vector: vec![],
            metadata: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn ranked(ids: &[&str]) -> Vec<(Memory, f32)> {
        ids.iter()
            .enumerate()
            .map(|(i, id)| (mem(id), 100.0 - i as f32))
            .collect()
    }

    #[test]
    fn member_of_both_lists_ranks_first() {
        let vector = ranked(&["a", "b", "c"]);
        let fulltext = ranked(&["x", "a", "y"]);
        let fused = fuse(&vector, &fulltext, 10);
        assert_eq!(fused[0].0.id, "a", "document present in both lists wins");
    }

    #[test]
    fn respects_limit() {
        let vector = ranked(&["a", "b", "c", "d"]);
        let fulltext = ranked(&["e", "f", "g", "h"]);
        let fused = fuse(&vector, &fulltext, 3);
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn empty_lists_fuse_to_empty() {
        assert!(fuse(&[], &[], 10).is_empty());
    }

    #[test]
    fn single_list_is_passthrough_order() {
        let vector = ranked(&["a", "b", "c"]);
        let fused = fuse(&vector, &[], 10);
        let ids: Vec<&str> = fused.iter().map(|(m, _)| m.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn rrf_k_dampens_but_preserves_rank() {
        let vector = ranked(&["a", "b"]);
        let fulltext = ranked(&["b", "a"]);
        // Symmetric tie: a and b appear at rank 0 in one list and rank 1 in
        // the other, so their fused scores must be equal.
        let fused = fuse(&vector, &fulltext, 10);
        assert!((fused[0].1 - fused[1].1).abs() < f32::EPSILON);
    }
}
