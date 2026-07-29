use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One registered Claude Code session, appended by the plugin's SessionStart
/// hook (via `armadai __claude-register-session`) and read by `armadai watch`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionRef {
    pub session_id: String,
    pub transcript_path: PathBuf,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub started_at: String,
}

/// Resolved path of the session index (override with `ARMADAI_SESSION_INDEX`).
pub fn index_path() -> PathBuf {
    if let Ok(p) = std::env::var("ARMADAI_SESSION_INDEX") {
        return PathBuf::from(p);
    }
    armadai_core::config::config_dir().join("claude-sessions.jsonl")
}

/// Append one session entry to the index (creating parent dirs as needed).
pub fn append(entry: &SessionRef) -> anyhow::Result<()> {
    append_to(&index_path(), entry)
}

fn append_to(path: &std::path::Path, entry: &SessionRef) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(entry)?;
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

/// Load + dedup the index (last occurrence of a `session_id` wins).
pub fn load() -> anyhow::Result<Vec<SessionRef>> {
    load_from(&index_path())
}

fn load_from(path: &std::path::Path) -> anyhow::Result<Vec<SessionRef>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    // Dedup last-wins while preserving the order of last occurrences.
    let mut order: Vec<String> = Vec::new();
    let mut by_id: std::collections::HashMap<String, SessionRef> = std::collections::HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<SessionRef>(line) else {
            continue; // defensive: skip malformed lines
        };
        if !by_id.contains_key(&entry.session_id) {
            order.push(entry.session_id.clone());
        } else {
            order.retain(|id| id != &entry.session_id);
            order.push(entry.session_id.clone());
        }
        by_id.insert(entry.session_id.clone(), entry);
    }
    Ok(order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect())
}

/// Pick a session: `--last` → last entry; else by `session_id`.
pub fn resolve(
    sessions: &[SessionRef],
    last: bool,
    session_id: Option<&str>,
) -> Option<SessionRef> {
    if last {
        return sessions.last().cloned();
    }
    if let Some(id) = session_id {
        return sessions.iter().find(|s| s.session_id == id).cloned();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_index(lines: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("idx.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        (dir, p)
    }

    #[test]
    fn load_parses_and_dedups_last_wins() {
        let (_d, p) = tmp_index(&[
            r#"{"session_id":"a","transcript_path":"/t/a.jsonl","cwd":"/c","started_at":"t1"}"#,
            r#"{"session_id":"b","transcript_path":"/t/b.jsonl","cwd":"/c","started_at":"t2"}"#,
            r#"{"session_id":"a","transcript_path":"/t/a2.jsonl","cwd":"/c","started_at":"t3"}"#,
            "not json — skipped",
        ]);
        let v = load_from(&p).unwrap();
        assert_eq!(v.len(), 2, "dedup by session_id");
        let a = v.iter().find(|s| s.session_id == "a").unwrap();
        assert_eq!(a.transcript_path, PathBuf::from("/t/a2.jsonl"), "last wins");
    }

    #[test]
    fn append_then_load_roundtrips() {
        let (_d, p) = tmp_index(&[]);
        append_to(
            &p,
            &SessionRef {
                session_id: "x".into(),
                transcript_path: "/t/x.jsonl".into(),
                cwd: "/c".into(),
                started_at: "t".into(),
            },
        )
        .unwrap();
        let v = load_from(&p).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].session_id, "x");
    }

    #[test]
    fn resolve_last_and_by_id() {
        let (_d, p) = tmp_index(&[
            r#"{"session_id":"a","transcript_path":"/t/a.jsonl","cwd":"/c","started_at":"t1"}"#,
            r#"{"session_id":"b","transcript_path":"/t/b.jsonl","cwd":"/c","started_at":"t2"}"#,
        ]);
        let v = load_from(&p).unwrap();
        assert_eq!(
            resolve(&v, true, None).unwrap().session_id,
            "b",
            "--last = dernière"
        );
        assert_eq!(resolve(&v, false, Some("a")).unwrap().session_id, "a");
        assert!(resolve(&v, false, Some("zzz")).is_none());
    }
}
