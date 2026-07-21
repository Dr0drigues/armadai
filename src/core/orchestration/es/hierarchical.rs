//! Pure transducer: LLM response → planned intentions (hierarchical pattern).
//!
//! `plan_from_response` reuses the existing (pure) delegation parser in
//! `crate::core::orchestration::protocol` and maps each parsed
//! `DelegationAction` onto a `PlannedStep` carrying the `ExecutionEvent` that
//! should be appended for it. No I/O, no async — this is the decision half
//! of the event-sourced hierarchical engine; effects (actually invoking
//! agents) are wired by a later lot.

use super::event::ExecutionEvent;
use crate::core::orchestration::protocol::{DelegationAction, parse_delegations};
use crate::core::orchestration::{OrchestrationConfig, OrchestrationPattern};

/// A single step planned from an LLM response, before any effect has run.
///
/// This is the pure output of `plan_from_response`: either an agent to
/// invoke next (paired with the `ExecutionEvent` that documents *why*, e.g.
/// a delegation or an escalation), or a terminal `Complete` when the
/// response carried no delegation directive at all.
#[derive(Debug, Clone)]
pub enum PlannedStep {
    /// Invoke `agent` with `task`, recording `event` alongside the
    /// resulting `AgentInvoked` (see `es::engine::Action::Invoke` +
    /// `Action::Emit`).
    Invoke {
        agent: String,
        task: String,
        event: ExecutionEvent,
    },
    /// The response was a final answer — no further delegation.
    Complete { content: String },
}

/// Turn `sender`'s `response` into an ordered list of planned steps.
///
/// Reuses `parse_delegations` (already pure) to extract `@agent: message`
/// directives, then maps each `DelegationAction` onto a `PlannedStep`:
/// - `Delegate { target, task }` → `Invoke` + `ExecutionEvent::Delegated`
///   (`from: sender`, `to: target`, `depth: depth + 1`).
/// - `AskPeer { target, question }` → `Invoke` + `ExecutionEvent::AskedPeer`.
/// - `Escalate { target, message }` → `Invoke` + `ExecutionEvent::Escalated`.
/// - `FinalAnswer { content }` → `Complete { content }`.
///
/// The order of the returned steps follows the order of the lines in
/// `response` (same guarantee as `parse_delegations`), so replay is
/// deterministic.
///
/// `parse_delegations` takes a full `OrchestrationConfig` (it needs team
/// topology to classify sender→target as Superior/Peer/Subordinate), but
/// this transducer's signature only carries `coordinator` — team topology is
/// not yet threaded through at this lot. We build a minimal config carrying
/// just the coordinator name; this correctly classifies the coordinator's
/// own outgoing lines as delegations and any line addressed to the
/// coordinator as an escalation (the cases this lot's tests exercise), but
/// falls back to `Unknown` (→ treated as `Delegate`, matching
/// `parse_delegations`'s own fallback) for peer relationships that would
/// otherwise require team data. A later lot that threads the full
/// `OrchestrationConfig` through can tighten this.
pub fn plan_from_response(
    response: &str,
    sender: &str,
    coordinator: &str,
    depth: u32,
) -> Vec<PlannedStep> {
    let config = OrchestrationConfig {
        enabled: true,
        pattern: OrchestrationPattern::Hierarchical,
        coordinator: Some(coordinator.to_string()),
        ..Default::default()
    };

    parse_delegations(response, sender, &config)
        .into_iter()
        .map(|action| match action {
            DelegationAction::Delegate { target, task } => PlannedStep::Invoke {
                agent: target.clone(),
                task: task.clone(),
                event: ExecutionEvent::Delegated {
                    from: sender.to_string(),
                    to: target,
                    task,
                    depth: depth + 1,
                },
            },
            DelegationAction::AskPeer { target, question } => PlannedStep::Invoke {
                agent: target.clone(),
                task: question.clone(),
                event: ExecutionEvent::AskedPeer {
                    from: sender.to_string(),
                    to: target,
                    question,
                },
            },
            DelegationAction::Escalate { target, message } => PlannedStep::Invoke {
                agent: target.clone(),
                task: message.clone(),
                event: ExecutionEvent::Escalated {
                    from: sender.to_string(),
                    to: target,
                    message,
                },
            },
            DelegationAction::FinalAnswer { content } => PlannedStep::Complete { content },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::orchestration::es::event::ExecutionEvent;

    #[test]
    fn final_answer_becomes_complete() {
        let steps = plan_from_response("Voici la réponse finale.", "dev-lead", "dev-lead", 0);
        assert_eq!(steps.len(), 1);
        matches!(&steps[0], PlannedStep::Complete { content } if content.contains("finale"))
            .then_some(())
            .expect("expected Complete");
    }

    #[test]
    fn delegation_lines_become_invokes_with_events() {
        let resp = "@core-specialist: implémente X\n@qa-specialist: teste X";
        let steps = plan_from_response(resp, "dev-lead", "dev-lead", 0);
        assert_eq!(steps.len(), 2);
        // ordre = ordre des lignes (déterminisme)
        match &steps[0] {
            PlannedStep::Invoke { agent, event, .. } => {
                assert_eq!(agent, "core-specialist");
                assert!(
                    matches!(event, ExecutionEvent::Delegated { to, depth, .. } if to == "core-specialist" && *depth == 1)
                );
            }
            _ => panic!("expected Invoke"),
        }
        match &steps[1] {
            PlannedStep::Invoke { agent, .. } => assert_eq!(agent, "qa-specialist"),
            _ => panic!("expected Invoke"),
        }
    }
}
