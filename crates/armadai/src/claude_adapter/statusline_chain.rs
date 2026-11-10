//! The user's own Claude Code status bar, run before ours (`ui.status_line`).
//!
//! Format read on https://code.claude.com/docs/en/statusline (2026-11-10, plan D17, D21,
//! D22): `statusLine` is `{"type":"command","command":...}`; the command runs in a shell
//! and gets the session JSON on stdin; each printed line is a row. The user's setting is
//! in `$CLAUDE_CONFIG_DIR/settings.json`, else `$HOME/.claude/settings.json`.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::compile::claude_settings::STATUS_LINE_COMMAND;

/// Set on the child we run: a user command that ends up calling us again must not chain.
pub const CHAINED_ENV: &str = "ARMADAI_STATUSLINE_CHAINED";

/// The user's statusLine command; `None` when absent, not a command, or ours (the same
/// command, or we run inside one chained by us).
pub fn user_command(env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    if env(CHAINED_ENV).is_some() {
        return None;
    }
    let dir = match env("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::path::PathBuf::from(env("HOME")?).join(".claude"),
    };
    let text = std::fs::read_to_string(dir.join("settings.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let bar = value.get("statusLine")?;
    if bar.get("type")?.as_str()? != "command" {
        return None;
    }
    let command = bar.get("command")?.as_str()?.trim();
    (!command.is_empty() && command != STATUS_LINE_COMMAND).then(|| command.to_string())
}

/// Run `sh -c <command>` with `stdin`, at most `limit`: its stdout when it exits 0 in
/// time, `None` otherwise (the child is killed on timeout).
#[cfg(unix)]
pub fn run_user(command: &str, stdin: &[u8], limit: Duration) -> Option<String> {
    let mut child = Command::new("sh")
        .args(["-c", command])
        .env(CHAINED_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Both ends go through threads: a child that never reads or that fills the pipe
    // must not block us past `limit`.
    let mut input = child.stdin.take()?;
    let data = stdin.to_vec();
    std::thread::spawn(move || {
        let _ = input.write_all(&data);
    });
    let mut output = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut raw = Vec::new();
        let _ = output.read_to_end(&mut raw);
        let _ = tx.send(raw);
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let left = limit.saturating_sub(start.elapsed());
                let raw = rx.recv_timeout(left).ok()?;
                return Some(String::from_utf8_lossy(&raw).into_owned());
            }
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if start.elapsed() >= limit => {
                // shortcut: only `sh` is killed, a grandchild may live on; upgrade with a process group
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// shortcut: no chaining on Windows (no `sh`): the flow line alone; upgrade with `cmd /C`.
#[cfg(not(unix))]
pub fn run_user(_command: &str, _stdin: &[u8], _limit: Duration) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn settings(dir: &std::path::Path, json: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("settings.json"), json).unwrap();
    }

    fn env_of<'a>(pairs: &'a [(&'a str, String)]) -> impl Fn(&str) -> Option<String> + 'a {
        let map: HashMap<&str, String> = pairs.iter().cloned().collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn the_user_command_comes_from_claude_config_dir_then_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let cfg = tmp.path().join("cfg");
        settings(
            &home.join(".claude"),
            r#"{"statusLine":{"type":"command","command":"from-home"}}"#,
        );
        let h = ("HOME", home.display().to_string());
        let c = ("CLAUDE_CONFIG_DIR", cfg.display().to_string());
        // CLAUDE_CONFIG_DIR set but without a bar: HOME is not consulted
        settings(&cfg, r#"{}"#);
        assert_eq!(user_command(&env_of(&[h.clone(), c.clone()])), None);
        settings(
            &cfg,
            r#"{"statusLine":{"type":"command","command":" from-cfg ","refreshInterval":10}}"#,
        );
        assert_eq!(
            user_command(&env_of(&[h.clone(), c])).as_deref(),
            Some("from-cfg")
        );
        assert_eq!(user_command(&env_of(&[h])).as_deref(), Some("from-home"));
        assert_eq!(user_command(&env_of(&[])), None);
    }

    #[test]
    fn a_bar_that_is_absent_unreadable_not_a_command_or_ours_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let h = ("HOME", tmp.path().display().to_string());
        let cases = [
            ("no file", None),
            ("not json", Some("nope")),
            ("no statusLine", Some(r#"{"a":1}"#)),
            (
                "other type",
                Some(r#"{"statusLine":{"type":"text","command":"x"}}"#),
            ),
            (
                "empty command",
                Some(r#"{"statusLine":{"type":"command","command":"  "}}"#),
            ),
            (
                "ours",
                Some(
                    r#"{"statusLine":{"type":"command","command":"armadai __claude-statusline"}}"#,
                ),
            ),
        ];
        for (name, json) in cases {
            let _ = std::fs::remove_dir_all(tmp.path().join(".claude"));
            if let Some(json) = json {
                settings(&tmp.path().join(".claude"), json);
            }
            assert_eq!(
                user_command(&env_of(std::slice::from_ref(&h))),
                None,
                "{name}"
            );
        }
        // a valid bar, but we run inside one we chained
        settings(
            &tmp.path().join(".claude"),
            r#"{"statusLine":{"type":"command","command":"x"}}"#,
        );
        let chained = (CHAINED_ENV, "1".to_string());
        assert_eq!(user_command(&env_of(&[h.clone(), chained])), None);
        assert_eq!(user_command(&env_of(&[h])).as_deref(), Some("x"));
    }

    #[cfg(unix)]
    #[test]
    fn run_user_gives_stdout_only_for_a_command_that_succeeds_in_time() {
        let limit = Duration::from_secs(2);
        assert_eq!(
            run_user("printf user-bar", b"", limit).as_deref(),
            Some("user-bar")
        );
        // stdin reaches the command
        assert_eq!(run_user("cat", b"in", limit).as_deref(), Some("in"));
        assert_eq!(run_user("printf lost; exit 3", b"", limit), None);
        assert_eq!(
            run_user("printf $ARMADAI_STATUSLINE_CHAINED", b"", limit).as_deref(),
            Some("1")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_slow_command_is_killed_at_the_limit() {
        // the defect keeps the call for the 5 s of the command; the bound is 3 s
        for pass in 0..5 {
            let start = Instant::now();
            assert_eq!(run_user("sleep 5", b"", Duration::from_secs(1)), None);
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "pass {pass}: {:?}",
                start.elapsed()
            );
        }
    }
}
