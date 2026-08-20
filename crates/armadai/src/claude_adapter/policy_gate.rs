//! `armadai __claude-policy-gate` — the Claude Code `PreToolUse` adapter.
//!
//! Reads the hook payload on stdin, asks
//! `armadai_core::orchestration::policy::check_delegation` whether the
//! delegation is allowed, and writes a decision on stdout.
//!
//! Contract, and the reason this file is deliberately dull: **printing
//! nothing means "no opinion", which Claude Code treats as allowed.** So every
//! parse failure, missing field or absent config simply returns without
//! output. A gate that refuses because it did not understand is a gate that
//! gets uninstalled the same day.
//!
//! Nothing may be written to stdout except the decision JSON — the same
//! contract `__claude-register-session` observes (tracing goes to stderr).

use std::io::Read;
use std::path::Path;

use armadai_core::orchestration::policy::check_delegation;
use armadai_core::project::find_project_config_from;
use serde_json::Value;

/// Read a hook payload from stdin and emit a decision. Never fails the hook.
pub fn gate_from_stdin() -> anyhow::Result<()> {
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return Ok(());
    }
    if let Some(json) = decide(&raw) {
        println!("{json}");
    }
    Ok(())
}

/// The whole gate, as a pure string→string function so it can be tested
/// without a subprocess. `None` means "no opinion" (allowed).
/// The sub-agent Claude Code spawns when an `Agent` call omits
/// `subagent_type`. Observed on 2026-08-20, NOT a documented contract: two
/// such calls ran as `general-purpose` and slipped past the gate, because an
/// absent field read as "no target" and the gate stayed silent. Treating the
/// omission as this target is what stops omitting the field from being a way
/// around the policy — and if a project declares this agent, such calls are
/// allowed again, because the policy decides rather than the shape of the call.
const IMPLICIT_SUBAGENT: &str = "general-purpose";

pub fn decide(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    // Only a delegation carries a topology decision. The hook's matcher
    // should already guarantee this; checking here means a hook installed
    // without a matcher cannot have every tool judged as a delegation.
    let tool = v.get("tool_name").and_then(Value::as_str)?;
    if tool != "Agent" && tool != "Task" {
        return None;
    }
    let target = v
        .get("tool_input")?
        .get("subagent_type")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(IMPLICIT_SUBAGENT);
    // Claude Code sends an empty `agent_type` on the main thread; a sub-agent
    // sub-delegating carries its own name.
    let caller = v
        .get("agent_type")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let cwd = v.get("cwd").and_then(Value::as_str)?;
    let (_, project) = find_project_config_from(Path::new(cwd))?;
    let orchestration = project.orchestration.as_deref()?;

    match check_delegation(caller, target, orchestration) {
        Ok(()) => None,
        Err(violation) => Some(
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": violation.reason,
                }
            })
            .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Payload shaped like the ones captured from a real session during the
    /// feasibility spike — not invented.
    fn payload(agent_type: &str, subagent_type: &str, cwd: &str) -> String {
        serde_json::json!({
            "session_id": "s1",
            "cwd": cwd,
            "permission_mode": "default",
            "hook_event_name": "PreToolUse",
            "tool_name": "Agent",
            "agent_type": agent_type,
            "tool_input": {
                "description": "do work",
                "prompt": "...",
                "subagent_type": subagent_type,
            },
            "tool_use_id": "toolu_01",
        })
        .to_string()
    }

    fn project_with_strict_policy() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/config.yaml"),
            "orchestration:\n  policy: strict\n  coordinator: dev-lead\n  \
             teams:\n    - agents: [qa-specialist]\n  free_agents: [Explore]\n",
        )
        .unwrap();
        dir
    }

    /// Regression: an `Agent` call may omit `subagent_type` entirely — Claude
    /// Code then spawns its default agent. Observed on 2026-08-20: two such
    /// calls slipped past the gate and ran as `general-purpose`, because
    /// `as_str()` on a missing field yielded `None` and the gate returned "no
    /// opinion". Omitting the target must not be a way around the policy.
    #[test]
    fn an_agent_call_without_subagent_type_is_still_policed() {
        let dir = project_with_strict_policy();
        let raw = serde_json::json!({
            "cwd": dir.path().to_string_lossy(),
            "hook_event_name": "PreToolUse",
            "tool_name": "Agent",
            "agent_type": "qa-specialist",
            // No `subagent_type`: exactly the shape captured from the leak.
            "tool_input": { "description": "run the tests", "prompt": "..." },
        })
        .to_string();
        let out = decide(&raw).expect("an implicit target must still be judged");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
        let reason = v["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap();
        assert!(
            reason.contains("general-purpose"),
            "the reason must name the implicit default so the model can react: {reason}"
        );
    }

    /// The same call is allowed once the default agent is declared — the
    /// policy decides, not the shape of the call.
    #[test]
    fn an_implicit_target_passes_when_it_is_declared() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/config.yaml"),
            "orchestration:\n  policy: strict\n  coordinator: dev-lead\n  \
             teams:\n    - agents: [qa-specialist]\n  free_agents: [general-purpose]\n",
        )
        .unwrap();
        let raw = serde_json::json!({
            "cwd": dir.path().to_string_lossy(),
            "tool_name": "Agent",
            "agent_type": "qa-specialist",
            "tool_input": { "description": "d", "prompt": "p" },
        })
        .to_string();
        assert!(decide(&raw).is_none(), "declared default must be allowed");
    }

    #[test]
    fn malformed_payload_yields_no_opinion() {
        assert!(decide("not json").is_none());
        assert!(decide("").is_none());
        assert!(decide("{}").is_none());
    }

    #[test]
    fn main_thread_reaching_a_specialist_is_denied_with_an_actionable_reason() {
        let dir = project_with_strict_policy();
        let out = decide(&payload("", "qa-specialist", &dir.path().to_string_lossy()))
            .expect("a violation must produce a decision");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
        let reason = v["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap();
        assert!(
            reason.contains("dev-lead"),
            "must name the way through: {reason}"
        );
    }

    #[test]
    fn main_thread_reaching_the_coordinator_is_allowed() {
        let dir = project_with_strict_policy();
        assert!(decide(&payload("", "dev-lead", &dir.path().to_string_lossy())).is_none());
    }

    #[test]
    fn the_coordinator_reaches_its_team() {
        let dir = project_with_strict_policy();
        assert!(
            decide(&payload(
                "dev-lead",
                "qa-specialist",
                &dir.path().to_string_lossy()
            ))
            .is_none()
        );
    }

    #[test]
    fn a_free_agent_is_reachable_from_the_main_thread() {
        let dir = project_with_strict_policy();
        assert!(decide(&payload("", "Explore", &dir.path().to_string_lossy())).is_none());
    }

    #[test]
    fn a_project_without_config_yields_no_opinion() {
        let dir = tempfile::tempdir().unwrap();
        assert!(decide(&payload("", "anything", &dir.path().to_string_lossy())).is_none());
    }

    #[test]
    fn deciding_twice_on_the_same_payload_is_idempotent() {
        let dir = project_with_strict_policy();
        let p = payload("", "qa-specialist", &dir.path().to_string_lossy());
        assert_eq!(decide(&p), decide(&p));
    }
}
