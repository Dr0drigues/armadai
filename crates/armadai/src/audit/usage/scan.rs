use std::collections::HashMap;
use std::io::BufRead;
use std::path::Path;

use serde_json::Value;

use crate::claude_adapter::transcript::{Block, RelevantEntry, parse_value};

use super::discovery::transcript_files;
use super::facts::{ROOT_AGENT, UsageFacts};

/// Aggregate every transcript belonging to `root`.
///
/// Streams line by line: transcripts reach hundreds of megabytes, so no file
/// is ever held in memory. Unreadable files and malformed lines are skipped —
/// a partial transcript still yields usable facts.
///
/// Unwired until a later task calls it from a CLI surface (`armadai audit`),
/// hence the per-item `#[allow(dead_code)]` — same convention as
/// `discovery.rs`/`facts.rs`.
#[allow(dead_code)]
pub fn scan(root: &Path) -> UsageFacts {
    let mut facts = UsageFacts {
        root_agent: ROOT_AGENT.to_string(),
        ..Default::default()
    };
    for file in transcript_files(root) {
        let Ok(handle) = std::fs::File::open(&file) else {
            continue;
        };
        facts.sessions += 1;
        // Per-file state: each entry's parent, and every agent spawn opened
        // in a given entry uuid (kept as (tool_use_id, agent) pairs — see
        // `enclosing_agent` for why the tool_use_id is worth keeping even
        // though the walk itself only ever looks up by uuid), so a sidechain
        // delegation can be attributed.
        let mut parent_of: HashMap<String, String> = HashMap::new();
        let mut agent_at: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for line in std::io::BufReader::new(handle)
            .lines()
            .map_while(Result::ok)
        {
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            scan_entry(&v, &mut facts, &mut parent_of, &mut agent_at);
        }
    }
    facts
}

#[allow(dead_code)]
fn str_field<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key).and_then(Value::as_str)
}

#[allow(dead_code)]
fn scan_entry(
    v: &Value,
    facts: &mut UsageFacts,
    parent_of: &mut HashMap<String, String>,
    agent_at: &mut HashMap<String, Vec<(String, String)>>,
) {
    if let Some(ts) = str_field(v, "timestamp") {
        facts.observe_timestamp(ts);
    }
    if let Some(skill) = str_field(v, "attributionSkill") {
        facts.record_skill_turn(skill);
    }
    let uuid = str_field(v, "uuid").unwrap_or("").to_string();
    if let Some(parent) = str_field(v, "parentUuid") {
        parent_of.insert(uuid.clone(), parent.to_string());
    }
    let is_sidechain = v
        .get("isSidechain")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let Some(RelevantEntry::Assistant { model, blocks, .. }) = parse_value(v) else {
        return;
    };

    // Who is delegating in this entry: the root on the main thread, else the
    // nearest enclosing agent found by walking the parentUuid chain.
    let delegator = if is_sidechain {
        enclosing_agent(&uuid, parent_of, agent_at).unwrap_or_else(|| ROOT_AGENT.to_string())
    } else {
        ROOT_AGENT.to_string()
    };

    let mut fanout = 0;
    for block in &blocks {
        match block {
            // The description is the per-call label (e.g. "run gate") and is
            // not needed here — usage counts the reusable identity, the
            // subagent_type, not the label attached to one invocation.
            Block::AgentSpawn {
                tool_use_id,
                subagent_type,
                ..
            } => {
                fanout += 1;
                facts.record_delegation(&delegator, subagent_type, &model);
                if !uuid.is_empty() {
                    agent_at
                        .entry(uuid.clone())
                        .or_default()
                        .push((tool_use_id.clone(), subagent_type.clone()));
                }
            }
            Block::Tool { name } => facts.record_tool(name),
            Block::Text(_) => {}
        }
    }
    facts.max_fanout = facts.max_fanout.max(fanout);
}

/// Walk up the parentUuid chain until an entry known to have opened an agent
/// is found. Bounded by the chain itself and by a visited set — a malformed
/// transcript (e.g. a cyclic parentUuid chain) must never hang the audit.
///
/// An entry that opened *exactly one* agent unambiguously identifies the
/// parent. An entry that opened *several* in parallel (one message, N
/// `tool_use` spawns) is ambiguous: the transcript format carries no field
/// correlating a sidechain entry back to the specific `tool_use_id` that
/// spawned it, so among parallel siblings the true parent is not recoverable
/// from the data — returning `None` here lets the caller degrade to
/// `ROOT_AGENT` (attribution rule 3) instead of guessing a sibling and
/// fabricating an edge that would be indistinguishable from a real one
/// downstream. `tool_use_id` is kept alongside each spawn regardless, so a
/// future correlation source (if one is ever found) can disambiguate without
/// another refactor of this map's shape.
#[allow(dead_code)]
fn enclosing_agent(
    uuid: &str,
    parent_of: &HashMap<String, String>,
    agent_at: &HashMap<String, Vec<(String, String)>>,
) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    let mut cursor = uuid.to_string();
    while seen.insert(cursor.clone()) {
        if let Some(spawns) = agent_at.get(&cursor) {
            return match spawns.as_slice() {
                [(_, agent)] => Some(agent.clone()),
                _ => None, // zero is unreachable (entries are only ever
                           // inserted with a spawn); more than one is ambiguous.
            };
        }
        cursor = parent_of.get(&cursor)?.clone();
    }
    None
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::scan;

    fn write(dir: &Path, name: &str, lines: &[&str]) {
        std::fs::write(dir.join(name), format!("{}\n", lines.join("\n"))).unwrap();
    }

    /// Same env-guard shape as discovery's tests.
    struct ProjectsDirGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl ProjectsDirGuard {
        fn set(path: &Path) -> Self {
            let lock = armadai_core::config::ENV_MUTEX.lock().unwrap();
            // SAFETY: modifies the global environment; serialised via ENV_MUTEX.
            unsafe { std::env::set_var("ARMADAI_CLAUDE_PROJECTS_DIR", path) }
            Self { _lock: lock }
        }
    }
    impl Drop for ProjectsDirGuard {
        fn drop(&mut self) {
            // SAFETY: restoring env state at end of test scope.
            unsafe { std::env::remove_var("ARMADAI_CLAUDE_PROJECTS_DIR") }
        }
    }

    fn fixture(lines: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let project = PathBuf::from("/Users/x/proj");
        let slug = dir
            .path()
            .join(crate::audit::usage::discovery::slug_for(&project));
        std::fs::create_dir_all(&slug).unwrap();
        write(&slug, "s1.jsonl", lines);
        (dir, project)
    }

    #[test]
    fn counts_delegations_skills_tools_and_sessions() {
        let (dir, project) = fixture(&[
            r#"{"type":"assistant","timestamp":"2026-08-01T00:00:00Z","isSidechain":false,"uuid":"u1","message":{"model":"claude-opus-5","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"qa","description":"run gate"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            r#"{"type":"assistant","timestamp":"2026-08-02T00:00:00Z","isSidechain":false,"uuid":"u2","attributionSkill":"armadai","message":{"model":"claude-opus-5","content":[{"type":"tool_use","id":"t2","name":"Bash","input":{}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let _g = ProjectsDirGuard::set(dir.path());

        let f = scan(&project);
        assert_eq!(f.sessions, 1, "one transcript file = one session");
        assert_eq!(f.agents["qa"].invocations, 1);
        assert_eq!(f.dominant_model("qa"), Some("claude-opus-5"));
        assert_eq!(f.skills["armadai"], 1);
        assert_eq!(f.tools["Bash"], 1);
        assert_eq!(
            f.window,
            Some((
                "2026-08-01T00:00:00Z".to_string(),
                "2026-08-02T00:00:00Z".to_string()
            ))
        );
        assert_eq!(f.depth(), 1);
    }

    #[test]
    fn parallel_fanout_in_one_message_is_measured() {
        let (dir, project) = fixture(&[
            r#"{"type":"assistant","timestamp":"2026-08-01T00:00:00Z","isSidechain":false,"uuid":"u1","message":{"model":"m","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"qa","description":"a"}},{"type":"tool_use","id":"t2","name":"Agent","input":{"subagent_type":"core","description":"b"}},{"type":"tool_use","id":"t3","name":"Agent","input":{"subagent_type":"ui","description":"c"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let _g = ProjectsDirGuard::set(dir.path());
        assert_eq!(scan(&project).max_fanout, 3);
    }

    #[test]
    fn sidechain_delegation_nests_under_its_parent_agent() {
        let (dir, project) = fixture(&[
            // Main thread spawns "lead" (tool_use id t1) from entry u1.
            r#"{"type":"assistant","timestamp":"2026-08-01T00:00:00Z","isSidechain":false,"uuid":"u1","message":{"model":"m","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"lead","description":"lead work"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            // Inside lead's sidechain (parent chain u2 -> u1), it spawns "qa".
            r#"{"type":"assistant","timestamp":"2026-08-01T00:01:00Z","isSidechain":true,"uuid":"u2","parentUuid":"u1","message":{"model":"m","content":[{"type":"tool_use","id":"t2","name":"Agent","input":{"subagent_type":"qa","description":"sub work"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let _g = ProjectsDirGuard::set(dir.path());

        let f = scan(&project);
        assert_eq!(f.depth(), 2, "qa must nest under lead, not under the root");
        assert!(f.edges["lead"].contains("qa"), "edges: {:?}", f.edges);
    }

    #[test]
    fn malformed_and_unknown_lines_are_skipped_without_failing() {
        let (dir, project) = fixture(&[
            "not json at all",
            r#"{"type":"ai-title","aiTitle":"x"}"#,
            r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"qa"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let _g = ProjectsDirGuard::set(dir.path());

        let f = scan(&project);
        assert_eq!(f.agents["qa"].invocations, 1, "the valid line still counts");
        assert_eq!(f.window, None, "no timestamp anywhere -> no window");
    }

    #[test]
    fn no_transcripts_yields_empty_facts() {
        let dir = tempfile::tempdir().unwrap();
        let _g = ProjectsDirGuard::set(dir.path());
        let f = scan(Path::new("/Users/x/nothing-here"));
        assert!(f.is_empty());
        assert_eq!(f.sessions, 0);
    }

    /// Attribution rule 3: when a sidechain's `parentUuid` walk cannot resolve
    /// to any entry known to have opened an agent (here it points at a uuid
    /// this transcript never records), the delegation degrades to
    /// `ROOT_AGENT` rather than erroring.
    #[test]
    fn unresolvable_sidechain_parent_chain_degrades_to_root_agent() {
        let (dir, project) = fixture(&[
            r#"{"type":"assistant","timestamp":"2026-08-01T00:00:00Z","isSidechain":true,"uuid":"u1","parentUuid":"orphan","message":{"model":"m","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"qa","description":"orphaned"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let _g = ProjectsDirGuard::set(dir.path());

        let f = scan(&project);
        assert!(
            f.edges[super::ROOT_AGENT].contains("qa"),
            "an unresolvable walk must degrade to ROOT_AGENT, not be dropped: {:?}",
            f.edges
        );
        assert_eq!(f.depth(), 1);
    }

    /// The transcript format carries no correlation from a sidechain entry
    /// back to which of several parallel `tool_use` spawns opened it, so
    /// among ambiguous siblings the true parent is not recoverable — the
    /// delegation must degrade to `ROOT_AGENT` (rule 3) rather than guess a
    /// sibling, and NONE of the three must gain a (possibly wrong) edge.
    #[test]
    fn ambiguous_parallel_spawn_degrades_to_root_agent_without_fabricating_an_edge() {
        let (dir, project) = fixture(&[
            // One entry opens three agents in parallel — agent_at["u1"] cannot
            // pick a single one of them.
            r#"{"type":"assistant","timestamp":"2026-08-01T00:00:00Z","isSidechain":false,"uuid":"u1","message":{"model":"m","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"qa","description":"a"}},{"type":"tool_use","id":"t2","name":"Agent","input":{"subagent_type":"core","description":"b"}},{"type":"tool_use","id":"t3","name":"Agent","input":{"subagent_type":"ui","description":"c"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            // A sidechain entry whose parentUuid chain reaches u1 — but WHICH
            // of qa/core/ui actually opened it is not recoverable from the data.
            r#"{"type":"assistant","timestamp":"2026-08-01T00:01:00Z","isSidechain":true,"uuid":"u2","parentUuid":"u1","message":{"model":"m","content":[{"type":"tool_use","id":"t4","name":"Agent","input":{"subagent_type":"grandchild","description":"d"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let _g = ProjectsDirGuard::set(dir.path());

        let f = scan(&project);
        assert!(
            f.edges[super::ROOT_AGENT].contains("grandchild"),
            "ambiguous parent must degrade to ROOT_AGENT: {:?}",
            f.edges
        );
        for sibling in ["qa", "core", "ui"] {
            assert!(
                !f.edges
                    .get(sibling)
                    .is_some_and(|c| c.contains("grandchild")),
                "must not fabricate an edge from sibling {sibling:?}: {:?}",
                f.edges
            );
        }
    }

    #[test]
    fn cyclic_parent_uuid_chain_terminates_instead_of_hanging() {
        let (dir, project) = fixture(&[
            // u1 <-> u2 form a parentUuid cycle; neither entry opens an agent,
            // so the walk must exhaust the visited set and degrade to
            // ROOT_AGENT rather than loop forever.
            r#"{"type":"assistant","timestamp":"2026-08-01T00:00:00Z","isSidechain":true,"uuid":"u1","parentUuid":"u2","message":{"model":"m","content":[{"type":"text","text":"x"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            r#"{"type":"assistant","timestamp":"2026-08-01T00:00:01Z","isSidechain":true,"uuid":"u2","parentUuid":"u1","message":{"model":"m","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"qa","description":"cyclic"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let _g = ProjectsDirGuard::set(dir.path());

        // Must terminate at all (the assertions below are secondary to that).
        let f = scan(&project);
        assert!(
            f.edges[super::ROOT_AGENT].contains("qa"),
            "a cyclic walk must degrade to ROOT_AGENT, not hang: {:?}",
            f.edges
        );
    }
}
