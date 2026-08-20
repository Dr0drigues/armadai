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

/// Resolve the project config for `cwd`, reaching into the main checkout when
/// `cwd` sits in a git worktree.
///
/// `find_project_config_from` stops at a `.git` boundary, and `.armadai/` is
/// gitignored on this project — so it lives only in the main checkout. Without
/// this fallback, opening a session in a worktree disables the gate entirely
/// and in silence, which is exactly how this project's own agents work
/// (`isolation: worktree`).
fn resolve_project(
    cwd: &Path,
) -> Option<(std::path::PathBuf, armadai_core::project::ProjectConfig)> {
    if let Some(found) = find_project_config_from(cwd) {
        return Some(found);
    }
    find_project_config_from(&main_checkout_of(cwd)?)
}

/// The main checkout behind a git worktree, or `None` if `cwd` is not in one.
///
/// A worktree's `.git` is a *file* reading `gitdir: <repo>/.git/worktrees/<name>`,
/// so the main checkout is the ancestor of that `.git` directory. Parsed by
/// hand rather than shelling out to git: the gate runs on every delegation and
/// must stay cheap.
fn main_checkout_of(cwd: &Path) -> Option<std::path::PathBuf> {
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let dot_git = d.join(".git");
        if dot_git.is_file() {
            let raw = std::fs::read_to_string(&dot_git).ok()?;
            let gitdir = raw.trim().strip_prefix("gitdir:")?.trim();
            // <repo>/.git/worktrees/<name>  ->  <repo>
            let marker = std::path::MAIN_SEPARATOR_STR.to_string() + ".git";
            let cut = gitdir.find(&marker)?;
            return Some(std::path::PathBuf::from(&gitdir[..cut]));
        }
        if dot_git.is_dir() {
            return None; // a real checkout, not a worktree
        }
        dir = d.parent();
    }
    None
}

/// The sub-agent Claude Code spawns when an `Agent` call omits
/// `subagent_type`. Observed on 2026-08-20, NOT a documented contract: two
/// such calls ran as `general-purpose` and slipped past the gate, because an
/// absent field read as "no target" and the gate stayed silent. Treating the
/// omission as this target is what stops omitting the field from being a way
/// around the policy — and if a project declares this agent, such calls are
/// allowed again, because the policy decides rather than the shape of the call.
const IMPLICIT_SUBAGENT: &str = "general-purpose";

/// The whole gate, as a pure string→string function so it can be tested
/// without a subprocess. `None` means "no opinion", which Claude Code
/// treats as allowed.
pub fn decide(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    // Only a delegation carries a topology decision. The hook's matcher
    // should already guarantee this; checking here means a hook installed
    // without a matcher cannot have every tool judged as a delegation.
    let tool = v.get("tool_name").and_then(Value::as_str)?;
    if tool != "Agent" && tool != "Task" {
        return None;
    }
    // `.and_then`, not `?`: an absent `tool_input` must be judged exactly like
    // `tool_input: null`, or the shape of the call decides again — the very
    // asymmetry the implicit-target fix was meant to close.
    let target = v
        .get("tool_input")
        .and_then(|i| i.get("subagent_type"))
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
    let (_, project) = resolve_project(Path::new(cwd))?;
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
            // Main thread, so the refusal names the target — which is what
            // proves the implicit default was resolved rather than ignored.
            "agent_type": "",
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

    /// Regression for the worktree hole: `.armadai/` is gitignored, so it only
    /// exists in the main checkout. A session opened in a worktree previously
    /// found no config and the gate went silent — disabling itself in exactly
    /// the workflow this project uses for its own agents.
    #[test]
    fn a_worktree_session_is_policed_via_the_main_checkout() {
        let root = tempfile::tempdir().unwrap();
        // Main checkout: a real `.git` directory plus the project config.
        let main = root.path().join("repo");
        std::fs::create_dir_all(main.join(".git")).unwrap();
        std::fs::create_dir_all(main.join(".armadai")).unwrap();
        std::fs::write(
            main.join(".armadai/config.yaml"),
            "orchestration:\n  policy: strict\n  coordinator: dev-lead\n  \
             teams:\n    - agents: [qa-specialist]\n",
        )
        .unwrap();
        // Worktree: `.git` is a FILE pointing into the main repo, no config.
        let wt = root.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}/.git/worktrees/wt\n", main.to_string_lossy()),
        )
        .unwrap();

        let raw = serde_json::json!({
            "cwd": wt.to_string_lossy(),
            "tool_name": "Agent",
            "agent_type": "",
            "tool_input": { "subagent_type": "qa-specialist" },
        })
        .to_string();
        assert!(
            decide(&raw).is_some(),
            "a worktree session must still be policed via the main checkout"
        );
    }

    /// An absent `tool_input` must be judged exactly like `tool_input: null`.
    /// Leaving it as "no opinion" reopened the bypass one level up the JSON
    /// tree: the shape of the call would decide again.
    #[test]
    fn an_absent_tool_input_is_judged_like_a_null_one() {
        let dir = project_with_strict_policy();
        let cwd = dir.path().to_string_lossy().to_string();
        let absent = serde_json::json!({
            "cwd": cwd, "tool_name": "Agent", "agent_type": ""
        })
        .to_string();
        let null = serde_json::json!({
            "cwd": cwd, "tool_name": "Agent", "agent_type": "", "tool_input": null
        })
        .to_string();
        assert_eq!(
            decide(&absent),
            decide(&null),
            "absent and null tool_input must reach the same verdict"
        );
        assert!(
            decide(&absent).is_some(),
            "and that verdict must be a denial"
        );
    }

    /// The `Task` alias is the tool's former name and still reaches the gate.
    #[test]
    fn the_task_alias_is_policed_too() {
        let dir = project_with_strict_policy();
        let raw = serde_json::json!({
            "cwd": dir.path().to_string_lossy(),
            "tool_name": "Task",
            "agent_type": "",
            "tool_input": { "subagent_type": "qa-specialist" },
        })
        .to_string();
        assert!(decide(&raw).is_some(), "Task must be policed like Agent");
    }

    /// A tool that is not a delegation carries no topology decision — the
    /// guard that keeps a matcher-less hook from denying unrelated tools.
    #[test]
    fn a_non_delegation_tool_is_never_judged() {
        let dir = project_with_strict_policy();
        let raw = serde_json::json!({
            "cwd": dir.path().to_string_lossy(),
            "tool_name": "Bash",
            "tool_input": { "command": "ls" },
        })
        .to_string();
        assert!(decide(&raw).is_none());
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

    /// Was `assert_eq!(decide(&p), decide(&p))` — a stateless function
    /// compared to itself, which no regression could ever redden. What can
    /// actually change is the config underneath: the gate reads it per call,
    /// so a rewrite must be picked up rather than cached.
    #[test]
    fn a_config_rewritten_between_calls_changes_the_verdict() {
        let dir = project_with_strict_policy();
        let raw = payload("", "qa-specialist", &dir.path().to_string_lossy());
        assert!(decide(&raw).is_some(), "strict: the call is refused");
        std::fs::write(
            dir.path().join(".armadai/config.yaml"),
            "orchestration:\n  policy: off\n  coordinator: dev-lead\n",
        )
        .unwrap();
        assert!(
            decide(&raw).is_none(),
            "switching to off must take effect on the next call"
        );
    }
}
