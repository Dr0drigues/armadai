//! `armadai __record-flow-event` — appends one line per sub-agent event to
//! `.armadai/state/claude/<session_id>.jsonl`.
//!
//! Wired by `link` as `async` Claude Code hooks on `SubagentStart`,
//! `SubagentStop`, `PreToolUse` (matchers `SubagentHandback` and `Agent|Task`) and
//! `UserPromptExpansion`; input shapes from https://code.claude.com/docs/en/hooks.
//! Like the policy gate, it never fails the hook and never prints (the stdout
//! of a `UserPromptExpansion` hook is added to the model's context): bad
//! input, no project or an unwritable log simply records nothing. Only ids,
//! the `STATE:` word and the name of a typed command are kept, never prompt,
//! reply text or command arguments.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Start,
    Stop,
    Command,
    /// A delegation the coordinator marked `[flow/step#lot]`, before the
    /// sub-agent starts.
    Mark,
}

/// One line of the session log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowEvent {
    pub ts: u64,
    pub event: EventKind,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub agent_type: String,
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// `mark` lines: the marker's parts. Never the description's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lot: Option<String>,
    /// `stop` lines: the word of the reply's `REF:` line.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// The log line of an event, final `\n` included. A `command` line carries
/// the name and nothing else; a `mark` line the target and the marker's parts.
pub fn log_line(e: &FlowEvent) -> String {
    #[derive(Serialize)]
    struct CommandLine<'a> {
        ts: u64,
        event: &'a EventKind,
        command: &'a str,
    }
    #[derive(Serialize)]
    struct MarkLine<'a> {
        ts: u64,
        event: &'a EventKind,
        agent_type: &'a str,
        flow: &'a str,
        step: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        lot: Option<&'a str>,
    }
    let json = match (&e.event, &e.command) {
        (EventKind::Mark, _) => serde_json::to_string(&MarkLine {
            ts: e.ts,
            event: &e.event,
            agent_type: &e.agent_type,
            flow: e.flow.as_deref().unwrap_or_default(),
            step: e.step.as_deref().unwrap_or_default(),
            lot: e.lot.as_deref(),
        }),
        (EventKind::Command, Some(command)) => serde_json::to_string(&CommandLine {
            ts: e.ts,
            event: &e.event,
            command,
        }),
        _ => serde_json::to_string(e),
    };
    json.expect("an event serializes") + "\n"
}

/// The word of the last line that reads exactly `STATE: <word>` (`word` an id).
pub fn state_of(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .filter_map(|l| l.trim_end().strip_prefix("STATE: "))
        .find(|w| armadai_model::is_valid_id(w))
        .map(str::to_string)
}

/// The word of the last line that reads exactly `REF: <word>`, 1 to 64 chars of
/// `[A-Za-z0-9._/#-]`.
pub fn ref_of(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .filter_map(|l| l.trim_end().strip_prefix("REF: "))
        .find(|w| {
            (1..=64).contains(&w.len())
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._/#-".contains(c))
        })
        .map(str::to_string)
}

/// `(session_id, cwd, event)` for a sub-agent start or stop or a typed slash
/// command, `None` otherwise.
pub fn event_from_hook(raw: &str, now: u64) -> Option<(String, PathBuf, FlowEvent)> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let text = |k: &str| v.get(k).and_then(serde_json::Value::as_str);
    if text("hook_event_name")? == "UserPromptExpansion" {
        // https://code.claude.com/docs/en/hooks: `expansion_type` is
        // `slash_command` for skills and custom commands, `mcp_prompt` otherwise.
        let name = text("command_name").filter(|n| !n.is_empty())?;
        if text("expansion_type")? != "slash_command" {
            return None;
        }
        let event = FlowEvent {
            ts: now,
            event: EventKind::Command,
            agent_id: String::new(),
            agent_type: String::new(),
            state: None,
            command: Some(name.to_string()),
            flow: None,
            step: None,
            lot: None,
            reference: None,
        };
        return Some((
            text("session_id")?.to_string(),
            PathBuf::from(text("cwd")?),
            event,
        ));
    }
    if text("hook_event_name")? == "PreToolUse"
        && matches!(text("tool_name"), Some("Agent" | "Task"))
    {
        return mark_from_hook(&v, now);
    }
    let (event, state, reference) = match text("hook_event_name")? {
        "SubagentStart" => (EventKind::Start, None, None),
        "SubagentStop" => {
            let message = text("last_assistant_message");
            (
                EventKind::Stop,
                message.and_then(state_of),
                message.and_then(ref_of),
            )
        }
        // `agent_id` (required below) is what marks a sub-agent: `agent_type`
        // is also set on the main thread under `--agent`.
        "PreToolUse" if text("tool_name") == Some("SubagentHandback") => {
            let message = v.pointer("/tool_input/message").and_then(|m| m.as_str());
            (
                EventKind::Stop,
                message.and_then(state_of),
                message.and_then(ref_of),
            )
        }
        _ => return None,
    };
    let event = FlowEvent {
        ts: now,
        event,
        agent_id: text("agent_id")?.to_string(),
        agent_type: text("agent_type").unwrap_or_default().to_string(),
        state,
        command: None,
        flow: None,
        step: None,
        lot: None,
        reference,
    };
    Some((
        text("session_id")?.to_string(),
        PathBuf::from(text("cwd")?),
        event,
    ))
}

/// A `mark` for an `Agent`/`Task` call whose description opens with a marker,
/// and only when the delegation gate lets the call through: a refused call must
/// not leave a mark that would stick to the next start. Its `agent_type` is the
/// target, resolved like the gate does.
fn mark_from_hook(v: &serde_json::Value, now: u64) -> Option<(String, PathBuf, FlowEvent)> {
    let text = |k: &str| v.get(k).and_then(serde_json::Value::as_str);
    let input = v.get("tool_input");
    let marker = super::marker::parse(
        input
            .and_then(|i| i.get("description"))
            .and_then(serde_json::Value::as_str)?,
    )?;
    if crate::claude_adapter::policy_gate::decide(&v.to_string()).is_some() {
        return None;
    }
    let target = input
        .and_then(|i| i.get("subagent_type"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(crate::claude_adapter::policy_gate::IMPLICIT_SUBAGENT);
    let event = FlowEvent {
        ts: now,
        event: EventKind::Mark,
        agent_id: String::new(),
        agent_type: target.to_string(),
        state: None,
        command: None,
        flow: Some(marker.flow),
        step: Some(marker.step),
        lot: marker.lot,
        reference: None,
    };
    Some((
        text("session_id")?.to_string(),
        PathBuf::from(text("cwd")?),
        event,
    ))
}

/// Where the session logs live, under the project root.
pub const LOGS_DIR: &str = ".armadai/state/claude";

/// Separates the flows of a group sharing an entry step (`a | b`).
pub const GROUP_SEP: &str = " | ";

/// `<root>/.armadai/state/claude/<session_id>.jsonl`, or `None` when the id
/// could leave that directory.
pub fn log_path(root: &Path, session_id: &str) -> Option<PathBuf> {
    let safe = !session_id.is_empty()
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    safe.then(|| root.join(LOGS_DIR).join(format!("{session_id}.jsonl")))
}

/// Read a hook payload from stdin and append its event. Never fails the hook.
pub fn record_from_stdin() -> anyhow::Result<()> {
    let mut raw = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut raw).is_ok()
        && let Err(e) = record(&raw)
    {
        tracing::debug!("flow event not recorded: {e}");
    }
    Ok(())
}

fn record(raw: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let (session, cwd, event) =
        event_from_hook(raw, now).ok_or_else(|| anyhow::anyhow!("not a sub-agent event"))?;
    let root = crate::claude_adapter::policy_gate::resolve_project(&cwd)
        .ok_or_else(|| anyhow::anyhow!("no project above {}", cwd.display()))?;
    let path = log_path(&root, &session).ok_or_else(|| anyhow::anyhow!("unsafe session id"))?;
    let claude = path.parent().expect("a log has a directory");
    // never write through a linked `state/`, `state/claude/` or log
    for at in [
        claude.parent().expect("state/ holds claude/"),
        claude,
        &path,
    ] {
        if std::fs::symlink_metadata(at).is_ok_and(|m| m.file_type().is_symlink()) {
            anyhow::bail!("{} is a symbolic link", at.display());
        }
    }
    std::fs::create_dir_all(claude)?;
    // One write per line, in append mode: parallel sub-agents do not interleave.
    let line = log_line(&event);
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(line.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Examples from https://code.claude.com/docs/en/hooks (SubagentStart,
    /// SubagentStop), with the reply carrying a `STATE:` line.
    const START: &str = r#"{"session_id":"abc123","transcript_path":"/x.jsonl","cwd":"/p","hook_event_name":"SubagentStart","agent_id":"agent-abc123","agent_type":"Explore"}"#;
    const STOP: &str = r#"{"session_id":"abc123","transcript_path":"/x.jsonl","cwd":"/p","permission_mode":"default","hook_event_name":"SubagentStop","stop_hook_active":false,"agent_id":"def456","agent_type":"Explore","agent_transcript_path":"/y.jsonl","last_assistant_message":"Analysis complete.\nSTATE: approved","background_tasks":[],"session_crons":[]}"#;
    const HANDBACK: &str = r#"{"session_id":"abc123","transcript_path":"/x.jsonl","cwd":"/p","permission_mode":"default","hook_event_name":"PreToolUse","agent_id":"def456","agent_type":"Explore","tool_name":"SubagentHandback","tool_input":{"message":"Report.\nSTATE: rejected"},"tool_use_id":"toolu_01"}"#;

    /// The doc's `UserPromptExpansion` example for a project command.
    const EXPANSION: &str = r#"{"session_id":"abc123","transcript_path":"/x.jsonl","cwd":"/p","permission_mode":"default","hook_event_name":"UserPromptExpansion","expansion_type":"slash_command","command_name":"bugfix","command_args":"secret arg","command_source":"project","prompt":"/bugfix secret arg"}"#;

    /// The log line without its final `\n`.
    fn line(raw: &str) -> (String, PathBuf, String) {
        let (s, cwd, ev) = event_from_hook(raw, 100).expect("an event");
        (
            s,
            cwd,
            log_line(&ev).strip_suffix('\n').unwrap().to_string(),
        )
    }

    #[test]
    fn the_last_state_line_wins() {
        assert_eq!(
            state_of("STATE: approved\nwork\nSTATE: rejected\ntrailing prose").as_deref(),
            Some("rejected")
        );
    }

    #[test]
    fn a_malformed_state_line_is_no_state() {
        assert_eq!(state_of("STATE:rejected"), None);
        assert_eq!(state_of("STATE: two words"), None);
        assert_eq!(state_of("no verdict here"), None);
    }

    #[test]
    fn a_malformed_last_line_leaves_the_last_valid_one() {
        assert_eq!(
            state_of("STATE: approved\nSTATE: Two words").as_deref(),
            Some("approved")
        );
    }

    #[test]
    fn subagent_start_records_a_start() {
        assert_eq!(
            line(START),
            (
                "abc123".into(),
                PathBuf::from("/p"),
                r#"{"ts":100,"event":"start","agent_id":"agent-abc123","agent_type":"Explore","state":null}"#.into()
            )
        );
    }

    #[test]
    fn subagent_stop_records_the_state_of_the_last_message() {
        assert_eq!(
            line(STOP),
            (
                "abc123".into(),
                PathBuf::from("/p"),
                r#"{"ts":100,"event":"stop","agent_id":"def456","agent_type":"Explore","state":"approved"}"#.into()
            )
        );
    }

    #[test]
    fn a_handback_records_the_state_of_its_message() {
        assert_eq!(
            line(HANDBACK),
            (
                "abc123".into(),
                PathBuf::from("/p"),
                r#"{"ts":100,"event":"stop","agent_id":"def456","agent_type":"Explore","state":"rejected"}"#.into()
            )
        );
    }

    #[test]
    fn a_typed_command_records_its_name_only() {
        assert_eq!(
            line(EXPANSION),
            (
                "abc123".into(),
                PathBuf::from("/p"),
                r#"{"ts":100,"event":"command","command":"bugfix"}"#.into()
            )
        );
    }

    #[test]
    fn old_and_command_lines_read_back() {
        let old = r#"{"ts":1,"event":"start","agent_id":"i1","agent_type":"an","state":null}"#;
        assert_eq!(
            serde_json::from_str::<FlowEvent>(old).unwrap(),
            FlowEvent {
                ts: 1,
                event: EventKind::Start,
                agent_id: "i1".into(),
                agent_type: "an".into(),
                state: None,
                command: None,
                flow: None,
                step: None,
                lot: None,
                reference: None,
            }
        );
        let command = r#"{"ts":2,"event":"command","command":"bugfix"}"#;
        assert_eq!(
            serde_json::from_str::<FlowEvent>(command).unwrap(),
            FlowEvent {
                ts: 2,
                event: EventKind::Command,
                agent_id: String::new(),
                agent_type: String::new(),
                state: None,
                command: Some("bugfix".into()),
                flow: None,
                step: None,
                lot: None,
                reference: None,
            }
        );
    }

    #[test]
    fn other_events_record_nothing() {
        let stop = STOP.replace(r#""SubagentStop""#, r#""Stop""#);
        let session = START.replace(r#""SubagentStart""#, r#""SessionStart""#);
        // A handback seen from the main thread carries no agent: nothing to attribute.
        let main_thread = HANDBACK.replace(r#""agent_id":"def456","agent_type":"Explore","#, "");
        // `--agent` sets `agent_type` on the main thread too: `agent_id` decides.
        let typed_main_thread = HANDBACK.replace(r#""agent_id":"def456","#, "");
        let other_tool = HANDBACK.replace("SubagentHandback", "Bash");
        let mcp = EXPANSION.replace("slash_command", "mcp_prompt");
        let no_type = EXPANSION.replace(r#""expansion_type":"slash_command","#, "");
        let empty_name = EXPANSION.replace(r#""command_name":"bugfix""#, r#""command_name":"""#);
        let no_name = EXPANSION.replace(r#""command_name":"bugfix","#, "");
        for raw in [
            &mcp,
            &no_type,
            &empty_name,
            &no_name,
            &stop,
            &session,
            &main_thread,
            &typed_main_thread,
            &other_tool,
            "{not json",
            "",
        ] {
            assert_eq!(event_from_hook(raw, 100), None, "{raw}");
        }
    }

    /// An `Agent` call as `policy_gate` tests shape it, with a description.
    fn agent_call(cwd: &Path, tool: &str, subagent: Option<&str>, description: &str) -> String {
        let mut input = serde_json::json!({"description": description, "prompt": "p"});
        if let Some(s) = subagent {
            input["subagent_type"] = s.into();
        }
        serde_json::json!({
            "session_id": "abc123", "cwd": cwd.to_string_lossy(),
            "hook_event_name": "PreToolUse", "tool_name": tool,
            "tool_input": input, "tool_use_id": "toolu_01",
        })
        .to_string()
    }

    fn strict_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path().join(".armadai");
        std::fs::create_dir_all(r.join("agents")).unwrap();
        std::fs::create_dir_all(r.join("flows")).unwrap();
        std::fs::write(
            r.join("project.yaml"),
            "schema_version: 1\ncoordinator: lead\npolicy:\n  delegation: strict\n",
        )
        .unwrap();
        for a in ["lead", "dev", "doc"] {
            std::fs::write(
                r.join(format!("agents/{a}.md")),
                "---\ndescription: d\n---\nB\n",
            )
            .unwrap();
        }
        std::fs::write(
            r.join("flows/ship.yaml"),
            "steps:\n  build: { agent: dev }\nlinks: []\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn a_marked_delegation_records_only_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let raw = agent_call(
            dir.path(),
            "Agent",
            Some("reviewer"),
            "[evolution/review#menu] relire SECRET free text",
        );
        let (session, _, ev) = event_from_hook(&raw, 100).expect("a mark");
        assert_eq!(session, "abc123");
        assert_eq!(
            log_line(&ev),
            "{\"ts\":100,\"event\":\"mark\",\"agent_type\":\"reviewer\",\"flow\":\"evolution\",\"step\":\"review\",\"lot\":\"menu\"}\n"
        );
        // no lot, no `subagent_type`, the former tool name: the gate's own defaults
        let raw = agent_call(dir.path(), "Task", None, "[evolution/review] x");
        let (_, _, ev) = event_from_hook(&raw, 7).expect("a mark");
        assert_eq!(
            log_line(&ev),
            "{\"ts\":7,\"event\":\"mark\",\"agent_type\":\"general-purpose\",\"flow\":\"evolution\",\"step\":\"review\"}\n"
        );
    }

    #[test]
    fn an_unmarked_delegation_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        for d in ["relire", "x [evolution/review]", "[Evolution/review] x"] {
            let raw = agent_call(dir.path(), "Agent", Some("reviewer"), d);
            assert_eq!(event_from_hook(&raw, 1), None, "{d}");
        }
    }

    #[test]
    fn a_refused_delegation_records_no_mark() {
        let dir = strict_project();
        // `doc` holds no step of a flow the main thread can reach: the gate refuses
        let refused = agent_call(dir.path(), "Agent", Some("doc"), "[ship/build#l1] x");
        assert!(crate::claude_adapter::policy_gate::decide(&refused).is_some());
        assert_eq!(event_from_hook(&refused, 1), None);
        // the same marker on an allowed target does leave one
        let allowed = agent_call(dir.path(), "Agent", Some("dev"), "[ship/build#l1] x");
        assert!(crate::claude_adapter::policy_gate::decide(&allowed).is_none());
        let (_, _, ev) = event_from_hook(&allowed, 1).expect("a mark");
        assert_eq!(ev.event, EventKind::Mark);
        assert_eq!(ev.agent_type, "dev");
    }

    #[test]
    fn the_last_ref_line_wins_and_is_bounded() {
        let w64 = "a".repeat(64);
        let w65 = "a".repeat(65);
        assert_eq!(
            ref_of("REF: one\nwork\nREF: feature/menu#2.x_y-z\ntrailing").as_deref(),
            Some("feature/menu#2.x_y-z")
        );
        assert_eq!(
            ref_of(&format!("REF: {w64}")).as_deref(),
            Some(w64.as_str())
        );
        assert_eq!(ref_of(&format!("REF: {w65}")), None);
        // an invalid last line leaves the last valid one
        assert_eq!(ref_of("REF: ok\nREF: two words").as_deref(), Some("ok"));
        assert_eq!(ref_of("REF: bad$char"), None);
        assert_eq!(ref_of("REF:none"), None);
        assert_eq!(ref_of("REF: "), None);
        assert_eq!(ref_of("see REF: x"), None);
    }

    #[test]
    fn a_stop_records_the_ref_of_its_message() {
        let stop = STOP.replace("STATE: approved", "STATE: approved\\nREF: feature/menu");
        let (_, _, l) = line(&stop);
        assert_eq!(
            l,
            r#"{"ts":100,"event":"stop","agent_id":"def456","agent_type":"Explore","state":"approved","ref":"feature/menu"}"#
        );
        let hb = HANDBACK.replace("STATE: rejected", "STATE: rejected\\nREF: pr-12");
        let (_, _, l) = line(&hb);
        assert_eq!(
            l,
            r#"{"ts":100,"event":"stop","agent_id":"def456","agent_type":"Explore","state":"rejected","ref":"pr-12"}"#
        );
    }

    #[test]
    fn old_and_mark_lines_read_back() {
        let mark =
            r#"{"ts":3,"event":"mark","agent_type":"reviewer","flow":"f","step":"s","lot":"l"}"#;
        let ev: FlowEvent = serde_json::from_str(mark).unwrap();
        assert_eq!(
            (ev.event, ev.agent_type.as_str(), ev.flow.as_deref()),
            (EventKind::Mark, "reviewer", Some("f"))
        );
        assert_eq!(
            (ev.step.as_deref(), ev.lot.as_deref()),
            (Some("s"), Some("l"))
        );
        let old = r#"{"ts":1,"event":"stop","agent_id":"i1","agent_type":"an","state":"approved"}"#;
        let ev: FlowEvent = serde_json::from_str(old).unwrap();
        assert_eq!(
            (ev.flow, ev.step, ev.lot, ev.reference),
            (None, None, None, None)
        );
    }

    #[test]
    fn a_safe_session_id_maps_to_its_log() {
        assert_eq!(
            log_path(Path::new("/p"), "abc-123_X"),
            Some(PathBuf::from("/p/.armadai/state/claude/abc-123_X.jsonl"))
        );
    }

    #[test]
    fn an_unsafe_session_id_has_no_log() {
        for id in ["../x", "a/b", "", "..", "a\\b"] {
            assert_eq!(log_path(Path::new("/p"), id), None, "{id}");
        }
    }
}
