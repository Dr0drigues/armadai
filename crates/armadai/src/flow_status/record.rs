//! `armadai __record-flow-event` — appends one line per sub-agent event to
//! `.armadai/state/claude/<session_id>.jsonl`.
//!
//! Wired by `link` as `async` Claude Code hooks on `SubagentStart`,
//! `SubagentStop` and `PreToolUse` (matcher `SubagentHandback`); input shapes
//! from https://code.claude.com/docs/en/hooks. Like the policy gate, it never
//! fails the hook and never prints on stdout: bad input, no project or an
//! unwritable log simply records nothing. Only ids and the `STATE:` word are
//! kept, never prompt or reply text.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Start,
    Stop,
}

/// One line of the session log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowEvent {
    pub ts: u64,
    pub event: EventKind,
    pub agent_id: String,
    pub agent_type: String,
    pub state: Option<String>,
}

/// The word of the last line that reads exactly `STATE: <word>` (`word` an id).
pub fn state_of(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .filter_map(|l| l.trim_end().strip_prefix("STATE: "))
        .find(|w| armadai_model::is_valid_id(w))
        .map(str::to_string)
}

/// `(session_id, cwd, event)` for a sub-agent start or stop, `None` otherwise.
pub fn event_from_hook(raw: &str, now: u64) -> Option<(String, PathBuf, FlowEvent)> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let text = |k: &str| v.get(k).and_then(serde_json::Value::as_str);
    let (event, state) = match text("hook_event_name")? {
        "SubagentStart" => (EventKind::Start, None),
        "SubagentStop" => (
            EventKind::Stop,
            text("last_assistant_message").and_then(state_of),
        ),
        // `agent_id` (required below) is what marks a sub-agent: `agent_type`
        // is also set on the main thread under `--agent`.
        "PreToolUse" if text("tool_name") == Some("SubagentHandback") => {
            let message = v.pointer("/tool_input/message").and_then(|m| m.as_str());
            (EventKind::Stop, message.and_then(state_of))
        }
        _ => return None,
    };
    let event = FlowEvent {
        ts: now,
        event,
        agent_id: text("agent_id")?.to_string(),
        agent_type: text("agent_type").unwrap_or_default().to_string(),
        state,
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
    let line = serde_json::to_string(&event)? + "\n";
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

    fn line(raw: &str) -> (String, PathBuf, String) {
        let (s, cwd, ev) = event_from_hook(raw, 100).expect("an event");
        (s, cwd, serde_json::to_string(&ev).unwrap())
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
    fn other_events_record_nothing() {
        let stop = STOP.replace(r#""SubagentStop""#, r#""Stop""#);
        let session = START.replace(r#""SubagentStart""#, r#""SessionStart""#);
        // A handback seen from the main thread carries no agent: nothing to attribute.
        let main_thread = HANDBACK.replace(r#""agent_id":"def456","agent_type":"Explore","#, "");
        // `--agent` sets `agent_type` on the main thread too: `agent_id` decides.
        let typed_main_thread = HANDBACK.replace(r#""agent_id":"def456","#, "");
        let other_tool = HANDBACK.replace("SubagentHandback", "Bash");
        for raw in [
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
