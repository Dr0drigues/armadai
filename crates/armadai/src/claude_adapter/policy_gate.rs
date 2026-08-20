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
pub fn decide(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let target = v
        .get("tool_input")
        .and_then(|i| i.get("subagent_type"))
        .and_then(Value::as_str)?;
    if target.is_empty() {
        return None;
    }
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
