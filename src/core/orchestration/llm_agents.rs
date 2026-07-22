use std::sync::Arc;

use super::blackboard::EntryKind;
use super::ring::ContributionAction;
use crate::core::routing::{BudgetState, RoutingRules};

/// Routing context threaded through the orchestration engines so
/// `LlmBoardAgent`/`LlmRingAgent` can route `latest:auto` models the same way
/// `run_single_agent` does (spec: OH4 router in orchestration).
///
/// `rules` is shared via `Arc` since one context is cloned into every agent
/// built for a run. `total_budget` is the board/ring's *configured*
/// `token_budget` (constant for the run) — not the remaining amount, which
/// changes every round/lap and is read from the snapshot at route time.
/// `None` (or a configured budget of `0`) disables budget-aware downgrade
/// entirely: `route()` is still called, just with `budget: None`.
#[derive(Clone)]
pub struct RoutingCtx {
    rules: Arc<RoutingRules>,
    total_budget: Option<u64>,
}

impl RoutingCtx {
    /// `total_budget` is the board/ring's configured token budget (e.g.
    /// `BlackboardConfig::token_budget` / `RingConfig::token_budget`).
    pub fn new(rules: RoutingRules, total_budget: u64) -> Self {
        Self {
            rules: Arc::new(rules),
            total_budget: (total_budget > 0).then_some(total_budget),
        }
    }

    /// Derive the router's `BudgetState` from tokens remaining vs. the
    /// configured total. `None` when no budget is configured for this run.
    fn budget_state(&self, remaining: u64) -> Option<BudgetState> {
        self.total_budget.map(|total| BudgetState {
            remaining_ratio: remaining as f64 / total as f64,
        })
    }
}

impl Default for RoutingCtx {
    fn default() -> Self {
        Self {
            rules: Arc::new(RoutingRules::default()),
            total_budget: None,
        }
    }
}

// ── Structured-response parsers ─────────────────────────────────

/// Prompt suffix appended to board agent messages so the LLM returns a
/// structured action header we can parse.
///
/// `pub(crate)` (rather than private) so `es::blackboard::BlackboardEffectRunner`
/// (OH1 Lot 3, Task 4) can reuse the exact same instructions when assembling
/// its own board prompt, instead of duplicating this string.
pub(crate) const BOARD_ACTION_INSTRUCTIONS: &str = "\n\n\
Respond with the following structured header, then your content:\n\
ACTION: <type> (one of: FINDING, CHALLENGE, CONFIRMATION, SYNTHESIS, QUESTION, ANSWER)\n\
TARGET: <index> (required for CHALLENGE, CONFIRMATION, ANSWER; comma-separated for SYNTHESIS)\n\
CONFIDENCE: <0.0-1.0>\n\
CONTENT: <your actual response>\n";

/// Parse a board agent's structured response into (EntryKind, confidence, content).
///
/// Falls back to `EntryKind::Finding` with confidence 0.8 if the header cannot
/// be parsed (e.g. the LLM ignores the instructions).
pub(crate) fn parse_board_action(response: &str) -> (EntryKind, f32, String) {
    let mut action_str = None;
    let mut target_str = None;
    let mut confidence: f32 = 0.8;
    let mut content_start = None;

    for (i, line) in response.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("ACTION:") {
            action_str = Some(rest.trim().to_uppercase());
        } else if let Some(rest) = trimmed.strip_prefix("TARGET:") {
            target_str = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("CONFIDENCE:") {
            if let Ok(c) = rest.trim().parse::<f32>() {
                confidence = c.clamp(0.0, 1.0);
            }
        } else if let Some(rest) = trimmed.strip_prefix("CONTENT:") {
            // Everything from here onward is the content body.
            let remainder: String = std::iter::once(rest.trim().to_string())
                .chain(response.lines().skip(i + 1).map(|l| l.to_string()))
                .collect::<Vec<_>>()
                .join("\n");
            content_start = Some(remainder);
            break;
        }
    }

    let content = content_start.unwrap_or_else(|| response.to_string());

    // Actions that require a TARGET fall back to Finding when the index is
    // absent — this avoids silently pointing at entry 0.
    let kind = match action_str.as_deref() {
        Some(a) if a.starts_with("CHALLENGE") => match parse_single_index(&target_str) {
            Some(target) => EntryKind::Challenge { target },
            None => EntryKind::Finding,
        },
        Some(a) if a.starts_with("CONFIRMATION") => match parse_single_index(&target_str) {
            Some(target) => EntryKind::Confirmation { target },
            None => EntryKind::Finding,
        },
        Some(a) if a.starts_with("SYNTHESIS") => {
            let sources = parse_index_list(&target_str);
            EntryKind::Synthesis { sources }
        }
        Some(a) if a.starts_with("QUESTION") => EntryKind::Question,
        Some(a) if a.starts_with("ANSWER") => match parse_single_index(&target_str) {
            Some(question) => EntryKind::Answer { question },
            None => EntryKind::Finding,
        },
        // FINDING or anything unrecognised → default
        _ => EntryKind::Finding,
    };

    (kind, confidence, content)
}

/// Prompt suffix for ring agent process messages.
pub(crate) const RING_ACTION_INSTRUCTIONS: &str = "\n\n\
Respond with the following structured header, then your content:\n\
ACTION: <type> (one of: PROPOSE, ENRICH, CONTEST, ENDORSE, SYNTHESIZE, PASS)\n\
TARGET: <index> (required for ENRICH, CONTEST, ENDORSE)\n\
CONTENT: <your actual response>\n";

/// Parse a ring agent's structured response into (ContributionAction, content).
///
/// Falls back to `ContributionAction::Propose` if parsing fails.
pub(crate) fn parse_ring_action(response: &str) -> (ContributionAction, String) {
    let mut action_str = None;
    let mut target_str = None;
    let mut content_start = None;

    for (i, line) in response.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("ACTION:") {
            action_str = Some(rest.trim().to_uppercase());
        } else if let Some(rest) = trimmed.strip_prefix("TARGET:") {
            target_str = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("CONTENT:") {
            let remainder: String = std::iter::once(rest.trim().to_string())
                .chain(response.lines().skip(i + 1).map(|l| l.to_string()))
                .collect::<Vec<_>>()
                .join("\n");
            content_start = Some(remainder);
            break;
        }
    }

    let content = content_start.unwrap_or_else(|| response.to_string());

    // Actions that require a TARGET fall back to Propose when the index is absent.
    let action = match action_str.as_deref() {
        Some(a) if a.starts_with("ENRICH") => match parse_single_index(&target_str) {
            Some(target) => ContributionAction::Enrich { target },
            None => ContributionAction::Propose,
        },
        Some(a) if a.starts_with("CONTEST") => match parse_single_index(&target_str) {
            Some(target) => ContributionAction::Contest {
                target,
                counter_argument: String::new(),
            },
            None => ContributionAction::Propose,
        },
        Some(a) if a.starts_with("ENDORSE") => match parse_single_index(&target_str) {
            Some(target) => ContributionAction::Endorse { target },
            None => ContributionAction::Propose,
        },
        Some(a) if a.starts_with("SYNTHESIZE") => ContributionAction::Synthesize,
        Some(a) if a.starts_with("PASS") => ContributionAction::Pass {
            reason: content.clone(),
        },
        // PROPOSE or anything unrecognised → default
        _ => ContributionAction::Propose,
    };

    (action, content)
}

/// Parse a confidence value from the first line of a vote response.
///
/// Falls back to 0.8 if the header is absent or malformed.
pub(crate) fn parse_vote_confidence(response: &str) -> (f32, String) {
    if let Some(first_line) = response.lines().next() {
        let trimmed = first_line.trim();
        if let Some(rest) = trimmed.strip_prefix("CONFIDENCE:")
            && let Ok(c) = rest.trim().parse::<f32>()
        {
            let body = response.lines().skip(1).collect::<Vec<_>>().join("\n");
            return (c.clamp(0.0, 1.0), body);
        }
    }
    (0.8, response.to_string())
}

fn parse_single_index(s: &Option<String>) -> Option<usize> {
    s.as_deref()
        .and_then(|v| v.trim().split(',').next())
        .and_then(|v| v.trim().parse::<usize>().ok())
}

fn parse_index_list(s: &Option<String>) -> Vec<usize> {
    s.as_deref()
        .map(|v| {
            v.split(',')
                .filter_map(|p| p.trim().parse::<usize>().ok())
                .collect()
        })
        .unwrap_or_default()
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Parser unit tests ────────────────────────────────────────

    // -- parse_board_action --

    #[test]
    fn test_parse_board_action_complete_header() {
        let response = "ACTION: CHALLENGE\nTARGET: 3\nCONFIDENCE: 0.9\nCONTENT: I disagree";
        let (kind, conf, content) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Challenge { target: 3 }));
        assert!((conf - 0.9).abs() < f32::EPSILON);
        assert_eq!(content, "I disagree");
    }

    #[test]
    fn test_parse_board_action_confirmation() {
        let response = "ACTION: CONFIRMATION\nTARGET: 0\nCONFIDENCE: 0.95\nCONTENT: Agreed";
        let (kind, conf, _) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Confirmation { target: 0 }));
        assert!((conf - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn test_parse_board_action_synthesis_multi_target() {
        let response = "ACTION: SYNTHESIS\nTARGET: 0, 2, 5\nCONTENT: Combined view";
        let (kind, _, content) = parse_board_action(response);
        match kind {
            EntryKind::Synthesis { sources } => assert_eq!(sources, vec![0, 2, 5]),
            other => panic!("Expected Synthesis, got {other:?}"),
        }
        assert_eq!(content, "Combined view");
    }

    #[test]
    fn test_parse_board_action_question() {
        let response = "ACTION: QUESTION\nCONTENT: What about edge cases?";
        let (kind, _, content) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Question));
        assert_eq!(content, "What about edge cases?");
    }

    #[test]
    fn test_parse_board_action_answer() {
        let response = "ACTION: ANSWER\nTARGET: 4\nCONTENT: Here is the answer";
        let (kind, _, _) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Answer { question: 4 }));
    }

    #[test]
    fn test_parse_board_action_no_header_fallback() {
        let response = "Just some plain text without any structured header";
        let (kind, conf, content) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Finding));
        assert!((conf - 0.8).abs() < f32::EPSILON);
        assert_eq!(content, response);
    }

    #[test]
    fn test_parse_board_action_challenge_no_target_fallback() {
        // CHALLENGE without TARGET should fallback to Finding
        let response = "ACTION: CHALLENGE\nCONFIDENCE: 0.7\nCONTENT: I disagree";
        let (kind, _, _) = parse_board_action(response);
        assert!(
            matches!(kind, EntryKind::Finding),
            "CHALLENGE without TARGET should fallback to Finding, got {kind:?}"
        );
    }

    #[test]
    fn test_parse_board_action_confirmation_no_target_fallback() {
        let response = "ACTION: CONFIRMATION\nCONTENT: Looks good";
        let (kind, _, _) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Finding));
    }

    #[test]
    fn test_parse_board_action_multiline_content() {
        let response = "ACTION: FINDING\nCONFIDENCE: 0.6\nCONTENT: Line one\nLine two\nLine three";
        let (kind, conf, content) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Finding));
        assert!((conf - 0.6).abs() < f32::EPSILON);
        assert_eq!(content, "Line one\nLine two\nLine three");
    }

    #[test]
    fn test_parse_board_action_confidence_clamped() {
        let response = "ACTION: FINDING\nCONFIDENCE: 5.0\nCONTENT: high";
        let (_, conf, _) = parse_board_action(response);
        assert!((conf - 1.0).abs() < f32::EPSILON);

        let response = "ACTION: FINDING\nCONFIDENCE: -2.0\nCONTENT: low";
        let (_, conf, _) = parse_board_action(response);
        assert!((conf - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_parse_board_action_invalid_confidence_keeps_default() {
        let response = "ACTION: FINDING\nCONFIDENCE: not_a_number\nCONTENT: text";
        let (_, conf, _) = parse_board_action(response);
        assert!((conf - 0.8).abs() < f32::EPSILON);
    }

    // -- parse_ring_action --

    #[test]
    fn test_parse_ring_action_propose() {
        let response = "ACTION: PROPOSE\nCONTENT: Use Rust for this";
        let (action, content) = parse_ring_action(response);
        assert!(matches!(action, ContributionAction::Propose));
        assert_eq!(content, "Use Rust for this");
    }

    #[test]
    fn test_parse_ring_action_enrich() {
        let response = "ACTION: ENRICH\nTARGET: 2\nCONTENT: Adding error handling";
        let (action, _) = parse_ring_action(response);
        assert!(matches!(action, ContributionAction::Enrich { target: 2 }));
    }

    #[test]
    fn test_parse_ring_action_contest() {
        let response = "ACTION: CONTEST\nTARGET: 1\nCONTENT: Performance concern";
        let (action, content) = parse_ring_action(response);
        match action {
            ContributionAction::Contest {
                target,
                counter_argument,
            } => {
                assert_eq!(target, 1);
                // counter_argument is empty (content is in the Contribution.content field)
                assert!(counter_argument.is_empty());
            }
            other => panic!("Expected Contest, got {other:?}"),
        }
        assert_eq!(content, "Performance concern");
    }

    #[test]
    fn test_parse_ring_action_endorse() {
        let response = "ACTION: ENDORSE\nTARGET: 0\nCONTENT: Fully agree";
        let (action, _) = parse_ring_action(response);
        assert!(matches!(action, ContributionAction::Endorse { target: 0 }));
    }

    #[test]
    fn test_parse_ring_action_synthesize() {
        let response = "ACTION: SYNTHESIZE\nCONTENT: Combining all views";
        let (action, _) = parse_ring_action(response);
        assert!(matches!(action, ContributionAction::Synthesize));
    }

    #[test]
    fn test_parse_ring_action_pass() {
        let response = "ACTION: PASS\nCONTENT: Nothing to add";
        let (action, _) = parse_ring_action(response);
        match action {
            ContributionAction::Pass { reason } => assert_eq!(reason, "Nothing to add"),
            other => panic!("Expected Pass, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_ring_action_no_header_fallback() {
        let response = "Just a plain response";
        let (action, content) = parse_ring_action(response);
        assert!(matches!(action, ContributionAction::Propose));
        assert_eq!(content, response);
    }

    #[test]
    fn test_parse_ring_action_enrich_no_target_fallback() {
        let response = "ACTION: ENRICH\nCONTENT: More detail";
        let (action, _) = parse_ring_action(response);
        assert!(
            matches!(action, ContributionAction::Propose),
            "ENRICH without TARGET should fallback to Propose"
        );
    }

    #[test]
    fn test_parse_ring_action_contest_no_target_fallback() {
        let response = "ACTION: CONTEST\nCONTENT: I disagree";
        let (action, _) = parse_ring_action(response);
        assert!(matches!(action, ContributionAction::Propose));
    }

    // -- parse_vote_confidence --

    #[test]
    fn test_parse_vote_confidence_valid() {
        let response = "CONFIDENCE: 0.75\nI agree with the proposal";
        let (conf, body) = parse_vote_confidence(response);
        assert!((conf - 0.75).abs() < f32::EPSILON);
        assert_eq!(body, "I agree with the proposal");
    }

    #[test]
    fn test_parse_vote_confidence_clamped() {
        let response = "CONFIDENCE: 99.0\nOverconfident";
        let (conf, _) = parse_vote_confidence(response);
        assert!((conf - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_parse_vote_confidence_missing() {
        let response = "I just have an opinion";
        let (conf, body) = parse_vote_confidence(response);
        assert!((conf - 0.8).abs() < f32::EPSILON);
        assert_eq!(body, response);
    }

    #[test]
    fn test_parse_vote_confidence_malformed() {
        let response = "CONFIDENCE: high\nMy position";
        let (conf, body) = parse_vote_confidence(response);
        assert!((conf - 0.8).abs() < f32::EPSILON);
        assert_eq!(body, response); // entire response since parse failed
    }

    // -- parse_single_index / parse_index_list --

    #[test]
    fn test_parse_single_index_valid() {
        assert_eq!(parse_single_index(&Some("5".to_string())), Some(5));
        assert_eq!(parse_single_index(&Some(" 3 ".to_string())), Some(3));
    }

    #[test]
    fn test_parse_single_index_from_list() {
        // Takes first index from comma-separated
        assert_eq!(parse_single_index(&Some("2, 5, 7".to_string())), Some(2));
    }

    #[test]
    fn test_parse_single_index_none() {
        assert_eq!(parse_single_index(&None), None);
    }

    #[test]
    fn test_parse_single_index_invalid() {
        assert_eq!(parse_single_index(&Some("abc".to_string())), None);
    }

    #[test]
    fn test_parse_index_list_valid() {
        assert_eq!(
            parse_index_list(&Some("0, 2, 5".to_string())),
            vec![0, 2, 5]
        );
    }

    #[test]
    fn test_parse_index_list_none() {
        assert!(parse_index_list(&None).is_empty());
    }

    #[test]
    fn test_parse_index_list_mixed_invalid() {
        // Skips invalid entries
        assert_eq!(parse_index_list(&Some("1, abc, 3".to_string())), vec![1, 3]);
    }
}
