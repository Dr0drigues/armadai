//! `armadai __claude-policy-gate` — the Claude Code `PreToolUse` adapter.
//!
//! Reads the hook payload on stdin, judges the delegation against the flows of
//! `.armadai/project.yaml` (`policy.delegation: strict`), and writes a decision on
//! stdout.
//!
//! Contract, and the reason this file is deliberately dull: **printing
//! nothing means "no opinion"**: Claude Code then goes on with its normal
//! permission flow (not an approval). So every parse failure, missing field or
//! absent project simply returns without output. A gate that refuses because it
//! did not understand is a gate that gets uninstalled the same day.
//!
//! Nothing may be written to stdout except the decision JSON (tracing and the
//! invalid-project warning go to stderr).

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use armadai_model::{Delegation, Project};
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

/// The nearest ancestor of `cwd` holding `.armadai/project.yaml`, without
/// crossing a `.git` boundary.
fn find_root(cwd: &Path) -> Option<PathBuf> {
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        if d.join(".armadai/project.yaml").is_file() {
            return Some(d.to_path_buf());
        }
        if d.join(".git").exists() {
            return None;
        }
        dir = d.parent();
    }
    None
}

/// The nearest ancestor of `cwd` holding a legacy project file (no `.git` crossing).
fn legacy_root(cwd: &Path) -> Option<PathBuf> {
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        if (crate::compile::LEGACY_CONFIGS.iter()).any(|c| d.join(c).is_file()) {
            return Some(d.to_path_buf());
        }
        if d.join(".git").exists() {
            return None;
        }
        dir = d.parent();
    }
    None
}

/// The project for `cwd`, reaching into the main checkout when `cwd` sits in a git
/// worktree that has no `.armadai/` of its own (an untracked or older checkout).
pub(crate) fn resolve_project(cwd: &Path) -> Option<PathBuf> {
    find_root(cwd).or_else(|| find_root(&main_checkout_of(cwd)?))
}

/// The main checkout behind a git worktree, or `None` if `cwd` is not in one.
///
/// A worktree's `.git` is a *file* reading `gitdir: <repo>/.git/worktrees/<name>`,
/// so the main checkout is the ancestor of that `.git` directory. Parsed by
/// hand rather than shelling out to git: the gate runs on every delegation and
/// must stay cheap.
fn main_checkout_of(cwd: &Path) -> Option<PathBuf> {
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let dot_git = d.join(".git");
        if dot_git.is_file() {
            let raw = std::fs::read_to_string(&dot_git).ok()?;
            let gitdir = raw.trim().strip_prefix("gitdir:")?.trim();
            // <repo>/.git/worktrees/<name>  ->  <repo>
            let marker = std::path::MAIN_SEPARATOR_STR.to_string() + ".git";
            let cut = gitdir.find(&marker)?;
            return Some(PathBuf::from(&gitdir[..cut]));
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

/// Agents `caller` may delegate to (`None` = the main thread).
///
/// - Main thread: with a coordinator, it IS the coordinator (it lives in
///   `CLAUDE.md`) and drives the flows: every step agent. Without one, the
///   agents of entry steps (`armadai_model::entry_steps`, the validator's own).
/// - The coordinator as `--agent`: same as the main thread.
/// - Agent X: the agents of the steps a step of X leads to (link or `on_failure`).
/// - `policy.free_agents`: always.
/// - The coordinator never, as a step agent: it is no sub-agent.
pub fn allowed_targets(project: &Project, caller: Option<&str>) -> BTreeSet<String> {
    let coordinator = project.manifest.coordinator.as_deref();
    let main = caller.is_none() || caller == coordinator;
    let mut out: BTreeSet<String> = project
        .manifest
        .policy
        .free_agents
        .iter()
        .cloned()
        .collect();
    for flow in project.flows.values() {
        let edges: Vec<(&str, &str)> = (flow.links.iter())
            .map(|l| (l.from.as_str(), l.to.as_str()))
            .chain(
                flow.steps
                    .iter()
                    .filter_map(|(sid, s)| s.on_failure.as_deref().map(|t| (sid.as_str(), t))),
            )
            .collect();
        let entries = armadai_model::entry_steps(flow);
        for (sid, step) in &flow.steps {
            let reached = if main {
                coordinator.is_some() || entries.contains(sid)
            } else {
                edges.iter().any(|(from, to)| {
                    to == sid
                        && flow
                            .steps
                            .get(*from)
                            .is_some_and(|s| Some(s.agent.as_str()) == caller)
                })
            };
            // the coordinator lives in CLAUDE.md: never a sub-agent to delegate to
            if reached && Some(step.agent.as_str()) != coordinator {
                out.insert(step.agent.clone());
            }
        }
    }
    out
}

/// The whole gate, as a pure string→string function so it can be tested
/// without a subprocess. `None` means "no opinion".
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
    // `tool_input: null`, or the shape of the call decides again.
    let target = v
        .get("tool_input")
        .and_then(|i| i.get("subagent_type"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(IMPLICIT_SUBAGENT);
    // Absent or empty on the main thread; a sub-agent carries its own name.
    let caller = v
        .get("agent_type")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let cwd = v.get("cwd").and_then(Value::as_str)?;
    let Some(root) = resolve_project(Path::new(cwd)) else {
        // an unmigrated project would otherwise lose its gate in silence
        if let Some(legacy) = legacy_root(Path::new(cwd)) {
            eprintln!(
                "armadai policy gate: {} uses the legacy format, run armadai migrate; gate inactive",
                legacy.display()
            );
        }
        return None;
    };
    let project = match armadai_model::load_checked(&root) {
        Ok((p, _)) => p,
        Err(ds) => {
            let first = ds
                .iter()
                .find(|d| d.severity == armadai_model::Severity::Error);
            eprintln!(
                "armadai policy gate: {} is invalid, the delegation gate is inactive{}",
                root.join(".armadai").display(),
                first
                    .map(|d| format!(": {}", d.render()))
                    .unwrap_or_default()
            );
            return None;
        }
    };
    if project.manifest.policy.delegation != Delegation::Strict {
        return None;
    }
    let allowed = allowed_targets(&project, caller);
    if allowed.contains(target) {
        return None;
    }
    let from = caller.map_or("the main thread".to_string(), |c| format!("agent `{c}`"));
    let reason = if allowed.is_empty() {
        // About the CALLER: naming the target would advise an action with no effect.
        format!(
            "{from} may not delegate under the declared flows; hand the work back, \
             or add a link from {from} in .armadai/flows/"
        )
    } else {
        format!(
            "the declared flows allow only [{}] from {from}; hand the work to one of \
             those, or add `{target}` to policy.free_agents in .armadai/project.yaml",
            allowed.into_iter().collect::<Vec<_>>().join(", ")
        )
    };
    Some(
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Payload shaped like the ones captured from a real session during the
    /// feasibility spike — not invented.
    fn payload(agent_type: &str, subagent_type: &str, cwd: &Path) -> String {
        serde_json::json!({
            "session_id": "s1",
            "cwd": cwd.to_string_lossy(),
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

    const AGENTS: [&str; 5] = ["lead", "dev", "qa", "doc", "helper"];

    /// `lead` coordinates; flow ship: build(dev) -> test(qa); `doc` is in no flow.
    /// `Explore` is a Claude Code built-in, not a project agent: allowed as a free agent.
    fn write_project(dir: &Path, manifest: &str) {
        std::fs::create_dir_all(dir.join(".armadai/agents")).unwrap();
        std::fs::create_dir_all(dir.join(".armadai/flows")).unwrap();
        std::fs::write(dir.join(".armadai/project.yaml"), manifest).unwrap();
        for a in AGENTS {
            std::fs::write(
                dir.join(format!(".armadai/agents/{a}.md")),
                "---\ndescription: d\n---\nB\n",
            )
            .unwrap();
        }
        std::fs::write(
            dir.join(".armadai/flows/ship.yaml"),
            "steps:\n  build: { agent: dev }\n  test: { agent: qa }\nlinks:\n  - { from: build, to: test }\n",
        )
        .unwrap();
    }

    const STRICT: &str = "schema_version: 1\ncoordinator: lead\npolicy:\n  delegation: strict\n  free_agents: [helper, Explore]\n";

    fn strict() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write_project(dir.path(), STRICT);
        dir
    }

    fn reason(out: Option<String>) -> String {
        let v: Value = serde_json::from_str(&out.expect("expected a denial")).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
        v["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn the_main_thread_reaches_every_step_agent_when_a_coordinator_leads() {
        let dir = strict();
        assert!(decide(&payload("", "dev", dir.path())).is_none());
        assert!(decide(&payload("", "qa", dir.path())).is_none());
    }

    #[test]
    fn an_agent_outside_the_flows_is_refused_with_the_permitted_ones() {
        let dir = strict();
        let r = reason(decide(&payload("", "doc", dir.path())));
        assert!(r.contains("[Explore, dev, helper, qa]"), "{r}");
        assert!(r.contains("free_agents"), "{r}");
    }

    /// The coordinator lives in CLAUDE.md, it is no sub-agent: a step it runs is
    /// never offered as a delegation target, from the main thread or an agent.
    #[test]
    fn the_coordinator_is_never_listed_as_a_target() {
        let dir = strict();
        std::fs::write(
            dir.path().join(".armadai/flows/plan.yaml"),
            "steps:\n  plan: { agent: lead }\n  build: { agent: dev }\n  review: { agent: lead }\nlinks:\n  - { from: plan, to: build }\n  - { from: build, to: review }\n",
        )
        .unwrap();
        let r = reason(decide(&payload("", "doc", dir.path())));
        assert!(r.contains("[Explore, dev, helper, qa]"), "{r}");
        let r = reason(decide(&payload("dev", "doc", dir.path())));
        assert!(!r.contains("lead"), "{r}");
    }

    #[test]
    fn a_step_reaches_only_its_successors() {
        let dir = strict();
        assert!(decide(&payload("dev", "qa", dir.path())).is_none());
        let r = reason(decide(&payload("qa", "dev", dir.path())));
        assert!(r.contains("agent `qa`"), "{r}");
        // `doc` is the source of no link: reaching `qa` is not its to do
        assert!(decide(&payload("doc", "qa", dir.path())).is_some());
    }

    #[test]
    fn the_coordinator_as_agent_is_the_main_thread() {
        let dir = strict();
        assert!(decide(&payload("lead", "qa", dir.path())).is_none());
    }

    #[test]
    fn a_free_agent_is_reachable_from_anywhere() {
        let dir = strict();
        assert!(decide(&payload("", "helper", dir.path())).is_none());
        assert!(decide(&payload("qa", "helper", dir.path())).is_none());
        assert!(decide(&payload("qa", "Explore", dir.path())).is_none());
    }

    #[test]
    fn without_a_coordinator_the_main_thread_reaches_entry_steps_only() {
        let dir = tempfile::tempdir().unwrap();
        write_project(
            dir.path(),
            "schema_version: 1\npolicy:\n  delegation: strict\n",
        );
        assert!(decide(&payload("", "dev", dir.path())).is_none());
        assert!(decide(&payload("", "qa", dir.path())).is_some());
    }

    /// Without a coordinator the main thread starts a review loop at its author:
    /// the `rejected` path back does not hide the entry, and the `approved`
    /// step is where the loop ends, not a way in.
    #[test]
    fn without_a_coordinator_a_review_loop_starts_at_its_author() {
        let dir = tempfile::tempdir().unwrap();
        write_project(
            dir.path(),
            "schema_version: 1\npolicy:\n  delegation: strict\n",
        );
        std::fs::remove_file(dir.path().join(".armadai/flows/ship.yaml")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/flows/review.yaml"),
            "steps:\n  write: { agent: dev }\n  review: { agent: qa, states: [approved, rejected] }\n  ship: { agent: doc }\nlinks:\n  - { from: write, to: review }\n  - { from: review, to: write, when: rejected }\n  - { from: review, to: ship, when: approved }\n",
        )
        .unwrap();
        assert!(decide(&payload("", "dev", dir.path())).is_none());
        let r = reason(decide(&payload("", "doc", dir.path())));
        assert!(r.contains("[dev]"), "{r}");
    }

    /// The review's counter-example: a cycle holding an unconditional link
    /// (`fix -> write`). With `start: write` the main thread starts at `dev`.
    #[test]
    fn a_declared_start_is_where_the_main_thread_begins() {
        let dir = tempfile::tempdir().unwrap();
        write_project(
            dir.path(),
            "schema_version: 1\npolicy:\n  delegation: strict\n",
        );
        std::fs::remove_file(dir.path().join(".armadai/flows/ship.yaml")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/flows/fix.yaml"),
            "start: write\nsteps:\n  write: { agent: dev }\n  review: { agent: qa, states: [rejected] }\n  fix: { agent: doc }\nlinks:\n  - { from: write, to: review }\n  - { from: review, to: fix, when: rejected }\n  - { from: fix, to: write }\n",
        )
        .unwrap();
        assert!(decide(&payload("", "dev", dir.path())).is_none());
        let r = reason(decide(&payload("", "doc", dir.path())));
        assert!(r.contains("[dev]"), "{r}");
    }

    #[test]
    fn on_failure_counts_as_a_path() {
        let dir = strict();
        std::fs::write(
            dir.path().join(".armadai/flows/fix.yaml"),
            "steps:\n  check: { agent: qa, on_failure: write }\n  write: { agent: doc }\n",
        )
        .unwrap();
        assert!(decide(&payload("qa", "doc", dir.path())).is_none());
    }

    /// Regression: an `Agent` call may omit `subagent_type` entirely — Claude
    /// Code then spawns its default agent. Omitting the target must not be a
    /// way around the policy.
    #[test]
    fn an_agent_call_without_subagent_type_is_still_policed() {
        let dir = strict();
        let raw = serde_json::json!({
            "cwd": dir.path().to_string_lossy(),
            "hook_event_name": "PreToolUse",
            "tool_name": "Agent",
            "agent_type": "",
            "tool_input": { "description": "run the tests", "prompt": "..." },
        })
        .to_string();
        let r = reason(decide(&raw));
        assert!(r.contains("general-purpose"), "{r}");
    }

    #[test]
    /// `general-purpose` is Claude Code's own agent: a free agent, not a project one.
    fn an_implicit_target_passes_when_it_is_declared() {
        let dir = tempfile::tempdir().unwrap();
        write_project(
            dir.path(),
            "schema_version: 1\ncoordinator: lead\npolicy:\n  delegation: strict\n  free_agents: [general-purpose]\n",
        );
        let raw = serde_json::json!({
            "cwd": dir.path().to_string_lossy(),
            "tool_name": "Agent",
            "agent_type": "qa",
            "tool_input": { "description": "d", "prompt": "p" },
        })
        .to_string();
        assert!(decide(&raw).is_none());
    }

    /// An absent `tool_input` must be judged exactly like `tool_input: null`.
    #[test]
    fn an_absent_tool_input_is_judged_like_a_null_one() {
        let dir = strict();
        let cwd = dir.path().to_string_lossy().to_string();
        let absent =
            serde_json::json!({ "cwd": cwd, "tool_name": "Agent", "agent_type": "" }).to_string();
        let null = serde_json::json!({
            "cwd": cwd, "tool_name": "Agent", "agent_type": "", "tool_input": null
        })
        .to_string();
        assert_eq!(decide(&absent), decide(&null));
        assert!(decide(&absent).is_some());
    }

    /// The `Task` alias is the tool's former name and still reaches the gate.
    #[test]
    fn the_task_alias_is_policed_too() {
        let dir = strict();
        let raw = payload("", "doc", dir.path()).replace("\"Agent\"", "\"Task\"");
        assert!(decide(&raw).is_some(), "Task must be policed like Agent");
    }

    #[test]
    fn a_non_delegation_tool_is_never_judged() {
        let dir = strict();
        let raw = payload("", "doc", dir.path()).replace("\"Agent\"", "\"Bash\"");
        assert!(decide(&raw).is_none());
    }

    #[test]
    fn malformed_or_truncated_payload_yields_no_opinion() {
        let dir = strict();
        let full = payload("", "doc", dir.path());
        assert!(decide(&full[..full.len() / 2]).is_none());
        assert!(decide("not json").is_none());
        assert!(decide("").is_none());
        assert!(decide("{}").is_none());
    }

    #[test]
    fn policy_off_yields_no_opinion() {
        let dir = tempfile::tempdir().unwrap();
        write_project(dir.path(), "schema_version: 1\ncoordinator: lead\n");
        assert!(decide(&payload("", "doc", dir.path())).is_none());
    }

    #[test]
    fn an_empty_subagent_type_is_the_implicit_target() {
        let dir = strict();
        let r = reason(decide(&payload("", "", dir.path())));
        assert!(r.contains("general-purpose"), "{r}");
    }

    #[test]
    fn a_legacy_project_yields_no_opinion() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(dir.path().join(".armadai/config.yaml"), "agents: []\n").unwrap();
        assert!(decide(&payload("", "doc", dir.path())).is_none());
        assert!(legacy_root(dir.path()).is_some());
        assert!(legacy_root(tempfile::tempdir().unwrap().path()).is_none());
    }

    #[test]
    fn outside_any_project_yields_no_opinion() {
        let dir = tempfile::tempdir().unwrap();
        assert!(decide(&payload("", "doc", dir.path())).is_none());
    }

    #[test]
    fn an_invalid_project_yields_no_opinion() {
        let dir = strict();
        // `ghost` is not an agent: a validation error
        std::fs::write(
            dir.path().join(".armadai/flows/bad.yaml"),
            "steps:\n  x: { agent: ghost }\n",
        )
        .unwrap();
        assert!(decide(&payload("", "doc", dir.path())).is_none());
    }

    #[test]
    fn a_subdirectory_finds_the_project() {
        let dir = strict();
        let sub = dir.path().join("src/deep");
        std::fs::create_dir_all(&sub).unwrap();
        assert!(decide(&payload("", "doc", &sub)).is_some());
    }

    #[test]
    fn a_worktree_session_is_policed_via_the_main_checkout() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("repo");
        std::fs::create_dir_all(main.join(".git")).unwrap();
        write_project(&main, STRICT);
        let wt = root.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}/.git/worktrees/wt\n", main.to_string_lossy()),
        )
        .unwrap();
        assert!(decide(&payload("", "doc", &wt)).is_some());
    }

    /// The gate reads the project per call: a rewrite must be picked up.
    #[test]
    fn a_project_rewritten_between_calls_changes_the_verdict() {
        let dir = strict();
        let raw = payload("", "doc", dir.path());
        assert!(decide(&raw).is_some());
        std::fs::write(
            dir.path().join(".armadai/project.yaml"),
            "schema_version: 1\ncoordinator: lead\n",
        )
        .unwrap();
        assert!(decide(&raw).is_none());
    }
}
