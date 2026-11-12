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

/// The most of the user's output we keep: a bar is a few lines, a runaway one is cut.
const MAX_OUTPUT: u64 = 64 * 1024;

/// Run `sh -c <command>` with `stdin`, at most `limit`: its first 64 KiB of stdout when it
/// exits 0 in time, `None` otherwise. The command runs in its own process group, which is
/// killed as a whole on timeout; after a normal end the group is left alone.
#[cfg(unix)]
pub fn run_user(command: &str, stdin: &[u8], limit: Duration) -> Option<String> {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new("sh")
        .args(["-c", command])
        .process_group(0)
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
        let _ = (&mut output).take(MAX_OUTPUT).read_to_end(&mut raw);
        // keep draining, so a chatty child is not blocked on a full pipe until the limit
        let _ = std::io::copy(&mut output, &mut std::io::sink());
        let _ = tx.send(raw);
    });
    let start = Instant::now();
    // after a clean end the group is left alone (a refresher the command detached is none of
    // ours to kill); the output counts once the pipe closes in time
    let answer = match finish(&mut child, limit, false) {
        Finish::Exited(status) if status.success() => {
            rx.recv_timeout(limit.saturating_sub(start.elapsed())).ok()
        }
        _ => None,
    };
    answer.map(|raw| String::from_utf8_lossy(&raw).into_owned())
}

/// What the leader of a process group is doing, seen without reaping it.
#[derive(Debug, PartialEq)]
pub(crate) enum Leader {
    Running,
    /// A zombie: it keeps its pid, hence the id of its group, until it is waited for.
    Exited,
    /// `waitid` failed (ECHILD: it was reaped, or auto-reaped with SIGCHLD ignored): its pid
    /// may name anything now.
    Gone,
}

/// The state of `child`, which was spawned with `process_group(0)`, without reaping it.
#[cfg(unix)]
pub(crate) fn leader(child: &mut std::process::Child) -> Leader {
    // SAFETY: `info` is a zeroed `siginfo_t`, a valid value for the C struct, and the only
    // thing `waitid` writes through the pointer; `child.id()` is a pid we spawned, and
    // WNOHANG | WNOWAIT make the call neither block nor reap. With nothing to report `info`
    // stays zeroed; a leader that has exited sets `si_signo` to SIGCHLD.
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let rc = libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        );
        match (rc, info.si_signo == libc::SIGCHLD) {
            (0, true) => Leader::Exited,
            (0, false) => Leader::Running,
            _ => Leader::Gone,
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn leader(child: &mut std::process::Child) -> Leader {
    match child.try_wait() {
        Ok(None) => Leader::Running,
        Ok(Some(_)) => Leader::Exited,
        Err(_) => Leader::Gone,
    }
}

/// SIGKILL to every process of the group `child` leads. Only for a leader seen `Running` or
/// `Exited` by [`leader`] and not waited for since.
#[cfg(unix)]
fn kill_group(child: &mut std::process::Child) {
    // SAFETY: `kill` takes two integers and touches no memory. The leader was seen alive or
    // as a zombie and has not been waited for, so its pid is still the id of the group we
    // created and no process of anyone else can carry it; a group already empty makes
    // `kill` return -1 (ESRCH), which we ignore.
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// How [`finish`] ended.
#[derive(Debug)]
pub(crate) enum Finish {
    Exited(std::process::ExitStatus),
    TimedOut,
    /// The leader could not be seen or waited for: no group was signalled.
    Lost,
}

/// Wait for `child` (spawned with `process_group(0)`) for at most `limit`. At the limit its
/// whole group is killed; after a normal end only when `kill_after_exit`. A group is never
/// signalled unless the leader was seen alive or exited and not reaped.
pub(crate) fn finish(
    child: &mut std::process::Child,
    limit: Duration,
    kill_after_exit: bool,
) -> Finish {
    let start = Instant::now();
    loop {
        match leader(child) {
            Leader::Exited => {
                if kill_after_exit {
                    kill_group(child);
                }
                return child.wait().map_or(Finish::Lost, Finish::Exited);
            }
            Leader::Gone => return Finish::Lost,
            Leader::Running if start.elapsed() >= limit => {
                kill_group(child);
                let _ = child.wait();
                return Finish::TimedOut;
            }
            Leader::Running => std::thread::sleep(Duration::from_millis(10)),
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
    fn alive(pid: &str) -> bool {
        Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    #[cfg(unix)]
    #[test]
    fn a_grandchild_of_a_slow_command_dies_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("pid");
        let command = format!("sleep 30 & echo $! > {}; wait", file.display());
        // the defect leaves the `sleep` for its 30 s; it is dead well before 3 s
        let start = Instant::now();
        assert_eq!(run_user(&command, b"", Duration::from_secs(1)), None);
        let pid = std::fs::read_to_string(&file).unwrap().trim().to_string();
        let mut gone = false;
        while start.elapsed() < Duration::from_secs(3) {
            if !alive(&pid) {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !gone {
            // never leave the `sleep` behind, whatever the verdict
            let _ = Command::new("kill").args(["-9", &pid]).status();
        }
        assert!(gone, "the grandchild {pid} was still alive after 3 s");
    }

    #[cfg(unix)]
    fn pid_alive_for_2s(pid: &str) -> bool {
        // `kill -0` also succeeds on a zombie until init reaps it: allow 2 s
        (0..200).all(|_| {
            let a = alive(pid);
            if a {
                std::thread::sleep(Duration::from_millis(10));
            }
            a
        })
    }

    #[cfg(unix)]
    #[test]
    fn leader_does_not_reap_the_leader_so_its_group_id_stays_ours() {
        use std::os::unix::process::CommandExt;
        let mut child = Command::new("sh")
            .args(["-c", "exit 0"])
            .process_group(0)
            .spawn()
            .unwrap();
        let start = Instant::now();
        while leader(&mut child) != Leader::Exited {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        // a reaped leader would be ESRCH here: a zombie still holds the pid and the group id
        assert!(alive(&child.id().to_string()), "the leader was reaped");
        kill_group(&mut child);
        assert!(child.wait().unwrap().success());
    }

    /// The leader is reaped while a member of its group lives on: `waitid` then fails, as
    /// it does with SIGCHLD ignored on Linux (not reachable on macOS), and nothing may be
    /// signalled, since the pid no longer names our group.
    #[cfg(unix)]
    #[test]
    fn a_leader_that_cannot_be_seen_gets_no_group_kill() {
        use std::os::unix::process::CommandExt;
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("pid");
        let command = format!("sleep 30 >/dev/null 2>&1 & echo $! > {}", file.display());
        let mut child = Command::new("sh")
            .args(["-c", &command])
            .process_group(0)
            .spawn()
            .unwrap();
        child.wait().unwrap();
        assert_eq!(leader(&mut child), Leader::Gone);
        let outcome = finish(&mut child, Duration::from_secs(1), true);
        let pid = std::fs::read_to_string(&file).unwrap().trim().to_string();
        let still = pid_alive_for_2s(&pid);
        let _ = Command::new("kill").args(["-9", &pid]).status();
        assert!(matches!(outcome, Finish::Lost), "{outcome:?}");
        assert!(still, "the group of a leader not seen was killed");
    }

    #[cfg(unix)]
    #[test]
    fn a_grandchild_detached_by_a_bar_that_succeeded_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("pid");
        // the refresher has its output redirected: the pipe closes with the shell
        let command = format!(
            "sleep 30 >/dev/null 2>&1 & echo $! > {}; printf bar",
            file.display()
        );
        let out = run_user(&command, b"", Duration::from_secs(2));
        let pid = std::fs::read_to_string(&file).unwrap().trim().to_string();
        let still = pid_alive_for_2s(&pid);
        let _ = Command::new("kill").args(["-9", &pid]).status();
        assert_eq!(out.as_deref(), Some("bar"));
        assert!(still, "the group was killed after a clean end");
    }

    #[cfg(unix)]
    #[test]
    fn a_runaway_output_is_cut_at_64_kib_and_does_not_block_the_command() {
        // 200 000 bytes on a pipe holds `head` until someone reads them: kept 64 KiB, the rest drained
        let out = run_user(
            "head -c 200000 /dev/zero | tr '\\0' a",
            b"",
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(out.len(), 64 * 1024);
        assert!(out.bytes().all(|b| b == b'a'));
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
