use crate::{CoreError, ExtractionResult, Memory, Message};
use serde::Deserialize;

pub const DEFAULT_SIMILARITY_THRESHOLD: f32 = 0.55;

/// Stable system prompt, byte-identical across calls (cache-friendly, PRD-07 §4.2).
pub const SYSTEM_PROMPT: &str = r#"You are the extraction engine of a personal memory system.
The user provides new information that may relate to existing memories.
Decide whether it is a NEW fact (add) or REPLACES an existing memory (update).
Return ONLY JSON: {"decision": "add"|"update", "memory_id": null|"<id>", "content": "<normalized content>"}
- "add": store as a new, standalone memory.
- "update": this fact replaces the memory with the given memory_id; the content field holds the merged/normalized replacement.
For "update" you MUST provide a valid memory_id from the existing memories listed below."#;

#[derive(Deserialize)]
struct RawExtraction {
    decision: String,
    memory_id: Option<String>,
    content: String,
}

/// Build the user message: new content + candidate memories the LLM may update.
pub fn build_prompt(content: &str, candidates: &[&(Memory, f32)]) -> Vec<Message> {
    let mut body = format!("NEW INFORMATION:\n{content}\n\n");
    if candidates.is_empty() {
        body.push_str("EXISTING MEMORIES: none\n");
    } else {
        body.push_str("EXISTING MEMORIES (id, score, content):\n");
        for (mem, score) in candidates {
            body.push_str(&format!(
                "- {} (score {score:.3}) {}\n",
                mem.id, mem.content
            ));
        }
    }
    vec![Message::system(SYSTEM_PROMPT), Message::user(body)]
}

/// Parse and validate the LLM's raw response into a decision we can apply.
///
/// `candidate_ids` is the set of memory ids shown in the prompt; an "update"
/// decision must reference one of them, otherwise it is a hallucination.
pub fn parse_extraction(raw: &str, candidate_ids: &[&str]) -> Result<ExtractionResult, CoreError> {
    let parsed: RawExtraction = parse_raw(raw)?;

    match parsed.decision.as_str() {
        "add" => Ok(ExtractionResult::add(parsed.content)),
        "update" => match parsed.memory_id {
            Some(id) if candidate_ids.contains(&id.as_str()) => {
                Ok(ExtractionResult::update(id, parsed.content))
            }
            Some(id) => Err(CoreError::InvalidExtraction(format!(
                "update references memory_id {id} which is not among the candidates"
            ))),
            None => Err(CoreError::InvalidExtraction(
                "update decision without a memory_id".into(),
            )),
        },
        other => Err(CoreError::InvalidExtraction(format!(
            "unknown decision {other:?}"
        ))),
    }
}

/// Shape-only parse, for LLM provider adapters that don't know the candidate
/// set. Candidate membership is validated by the service layer instead.
pub fn parse_extraction_shape_only(raw: &str) -> Result<ExtractionResult, CoreError> {
    let parsed: RawExtraction = parse_raw(raw)?;

    match parsed.decision.as_str() {
        "add" => Ok(ExtractionResult::add(parsed.content)),
        "update" => match parsed.memory_id {
            Some(id) => Ok(ExtractionResult::update(id, parsed.content)),
            None => Err(CoreError::InvalidExtraction(
                "update decision without a memory_id".into(),
            )),
        },
        other => Err(CoreError::InvalidExtraction(format!(
            "unknown decision {other:?}"
        ))),
    }
}

fn parse_raw(raw: &str) -> Result<RawExtraction, CoreError> {
    serde_json::from_str(raw).map_err(|e| {
        CoreError::InvalidExtraction(format!("LLM did not return valid JSON ({e}): {raw}"))
    })
}

pub fn above_threshold(candidates: &[(Memory, f32)], floor: f32) -> Vec<&(Memory, f32)> {
    let Some(top) = candidates
        .iter()
        .map(|(_, s)| *s)
        .max_by(|a, b| a.total_cmp(b))
    else {
        return Vec::new();
    };
    let cut = floor.max(top * 0.80);
    candidates
        .iter()
        .filter(|(_, score)| *score >= cut)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExtractionDecision;
    use chrono::Utc;
    use std::collections::HashMap;

    fn mem(id: &str, content: &str) -> Memory {
        Memory {
            id: id.to_string(),
            user_id: "u1".to_string(),
            content: content.to_string(),
            vector: vec![],
            metadata: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn parse_add() {
        let r = parse_extraction(
            r#"{"decision":"add","memory_id":null,"content":"new fact"}"#,
            &[],
        )
        .unwrap();
        assert!(matches!(r.decision, ExtractionDecision::Add));
        assert_eq!(r.content, "new fact");
    }

    #[test]
    fn parse_update_valid_candidate() {
        let r = parse_extraction(
            r#"{"decision":"update","memory_id":"m1","content":"moved to Bandung"}"#,
            &["m1"],
        )
        .unwrap();
        assert!(matches!(r.decision, ExtractionDecision::Update));
        assert_eq!(r.updated_memory_id.as_deref(), Some("m1"));
    }

    #[test]
    fn update_with_unknown_memory_id_rejected() {
        let err = parse_extraction(
            r#"{"decision":"update","memory_id":"ghost","content":"x"}"#,
            &["m1"],
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::InvalidExtraction(_)));
    }

    #[test]
    fn update_without_memory_id_rejected() {
        let err = parse_extraction(r#"{"decision":"update","content":"x"}"#, &["m1"]).unwrap_err();
        assert!(matches!(err, CoreError::InvalidExtraction(_)));
    }

    #[test]
    fn malformed_json_rejected() {
        let err = parse_extraction("Extra data { not json", &[]).unwrap_err();
        assert!(matches!(err, CoreError::InvalidExtraction(_)));
    }

    #[test]
    fn unknown_decision_rejected() {
        let err = parse_extraction(r#"{"decision":"delete","content":"x"}"#, &[]).unwrap_err();
        assert!(matches!(err, CoreError::InvalidExtraction(_)));
    }

    #[test]
    fn threshold_filters_candidates() {
        let cands = vec![
            (mem("m1", "a"), 0.95_f32),
            (mem("m2", "b"), 0.72_f32),
            (mem("m3", "c"), 0.78_f32),
        ];
        let kept = above_threshold(&cands, 0.55);
        let ids: Vec<&str> = kept.iter().map(|(m, _)| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["m1", "m3"],
            "m2 at 0.72 < cut 0.76 should be dropped"
        );
    }

    #[test]
    fn threshold_empty_input_gives_empty_output() {
        let cands: Vec<(Memory, f32)> = vec![];
        let kept = above_threshold(&cands, 0.55);
        assert!(kept.is_empty());
    }

    #[test]
    fn threshold_all_pass_when_top_is_low() {
        let cands = vec![
            (mem("m1", "bandung"), 0.64_f32),
            (mem("m2", "coding"), 0.56_f32),
        ];
        let kept = above_threshold(&cands, 0.55);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn prompt_includes_candidates_and_scores() {
        let cands = [(mem("m1", "User lives in Jakarta"), 0.95_f32)];
        let refs: Vec<&(Memory, f32)> = cands.iter().collect();
        let messages = build_prompt("User moved to Bandung", &refs);
        assert_eq!(messages[0].role, "system");
        assert!(messages[1].content.contains("User lives in Jakarta"));
        assert!(messages[1].content.contains("0.950"));
        assert!(messages[1].content.contains("m1"));
    }

    // ── parse_extraction_shape_only ──

    #[test]
    fn shape_only_parse_add() {
        let r = parse_extraction_shape_only(
            r#"{"decision":"add","memory_id":null,"content":"new fact"}"#,
        )
        .unwrap();
        assert!(matches!(r.decision, ExtractionDecision::Add));
        assert_eq!(r.content, "new fact");
    }

    #[test]
    fn shape_only_parse_update() {
        let r = parse_extraction_shape_only(
            r#"{"decision":"update","memory_id":"m1","content":"updated"}"#,
        )
        .unwrap();
        assert!(matches!(r.decision, ExtractionDecision::Update));
        assert_eq!(r.updated_memory_id.as_deref(), Some("m1"));
    }

    #[test]
    fn shape_only_rejects_update_without_memory_id() {
        let err = parse_extraction_shape_only(r#"{"decision":"update","content":"missing id"}"#)
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidExtraction(_)));
    }

    #[test]
    fn shape_only_rejects_unknown_decision() {
        let err =
            parse_extraction_shape_only(r#"{"decision":"delete","content":"x"}"#).unwrap_err();
        assert!(matches!(err, CoreError::InvalidExtraction(_)));
    }

    #[test]
    fn shape_only_rejects_malformed_json() {
        let err = parse_extraction_shape_only("not json").unwrap_err();
        assert!(matches!(err, CoreError::InvalidExtraction(_)));
    }

    // ── build_prompt without candidates ──

    #[test]
    fn build_prompt_without_candidates() {
        let messages = build_prompt("new info", &[]);
        assert_eq!(messages[0].role, "system");
        assert!(messages[1].content.contains("EXISTING MEMORIES: none"));
        assert!(messages[1].content.contains("new info"));
    }
}
