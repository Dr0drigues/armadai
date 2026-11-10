//! `armadai __claude-grant <level>` — the Claude Code `PermissionRequest` hook of a
//! permission level (`policy.grants`).
//!
//! Format read on code.claude.com/docs/en/hooks (2026-11-10): the hook runs when Claude
//! Code is about to ask for a permission, after the `deny` and `ask` rules, so it can
//! never lift a `deny`; it answers `{"hookSpecificOutput":{"hookEventName":
//! "PermissionRequest","decision":{"behavior":"allow"}}}` and prints nothing for "no
//! opinion", which leaves the prompt. Measured on 2026-11-10 (claude 2.1.319, `claude -p`):
//! `allow` passes an `ask` rule, a `deny` rule is never lifted, and the `if` filter does
//! not fire on `echo other` but fires on a compound or unreadable command.
//!
//! The rule: **`allow` only when everything is established**; any error, doubt or
//! missing piece is "no opinion". The `if` filter is best effort, so this hook checks the
//! call against the rules of its level itself, and answers only for a simple `Bash`
//! command. It never answers for another tool: the permissions page (2026-11-10) says an
//! allow rule on a file applies only when both the requested path and the file it resolves
//! to match, which a hook does not redo (measured: an `Edit` through a symlink under
//! `src/` wrote outside it under an `Edit(/src/**)` level). A rule that is not a `Bash`
//! rule is reported by the validator (`grant-unsupported-rule`) and gets no hook.
//!
//! **What this is not: a boundary** (permissions page, D16: a rule covers the call Claude
//! usually makes, it is not a security boundary). The checks below are lines of defence
//! that make an accident or a naive attempt fail; four ways around them were found in
//! review, and more remain. Under a level whose rule is broad (`Bash(cp *)`, `Bash(python
//! *)`, `Bash(sh *)`, `Bash(*)`), an agent can still do what the allowed program can do:
//! an interpreter or a shell runs any code, so it can rewrite `.armadai/state/grants.yaml`
//! or call `armadai grant` from inside a script the hook never reads; `cp`, `mv`, `ln` or
//! `git` can reach the state file through a link, a directory or a name this hook does not
//! recognize; a command a rule covers may start a program that does. Write the rules of a
//! level narrowly (a full command with fixed arguments, e.g. `Bash(git push origin
//! feature/*)`), never an interpreter, a shell, `env`, `xargs` or a bare `*`.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where the active levels are kept, under the project root.
pub const STATE_FILE: &str = ".armadai/state/grants.yaml";

/// `.armadai/state/grants.yaml`: the levels switched on by `armadai grant`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantState {
    pub version: u32,
    #[serde(default)]
    pub active: BTreeMap<String, ActiveGrant>,
}

impl Default for GrantState {
    fn default() -> Self {
        Self {
            version: 1,
            active: BTreeMap::new(),
        }
    }
}

/// A level that is on: since when, and what ends it (at least one bound).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveGrant {
    pub since: DateTime<Utc>,
    /// The level is over from this instant on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// `<flow>/<step>`: the level is over once a delegation to it is recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_step: Option<String>,
}

/// A rule of the form `Bash(<pattern>)`, the only one a level answers.
pub fn is_bash_rule(rule: &str) -> bool {
    rule.starts_with("Bash(") && rule.ends_with(')')
}

/// The tool a permission rule names: `Bash(git push *)` is `Bash`.
pub fn rule_tool(rule: &str) -> &str {
    rule.split_once('(').map_or(rule, |(t, _)| t)
}

/// A command that is one plain program call: ASCII only (a case-insensitive disk folds
/// `grantſ.yaml` onto `grants.yaml`), and no operator, substitution, redirection,
/// expansion, quoting, escape, grouping or line break. Claude Code's own filter cannot
/// tell for some commands (`eval`, a substitution), so a level answers only for these.
fn is_simple(command: &str) -> bool {
    !command.is_empty()
        && command.is_ascii()
        && !command
            .chars()
            .any(|c| "&|;<>()`$\\\"'{}*?[]~!#\n\r\0".contains(c))
}

/// A command that mentions the state file or `armadai` itself: a level never answers for
/// it, whatever its rules say. A line of defence, not a guarantee: it catches the plain
/// forms (`cp x .armadai/state/grants.yaml`, `armadai grant lvl`), not a program that does
/// it for the agent.
fn names_grant_machinery(command: &str) -> bool {
    let c = command.to_ascii_lowercase();
    c.contains("grants.yaml") || c.contains("armadai")
}

/// Whether `text` matches `pattern`, where `*` is any run of characters.
fn glob(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((head, rest)) => {
            let Some(after) = text.strip_prefix(head) else {
                return false;
            };
            (0..=after.len())
                .filter(|i| after.is_char_boundary(*i))
                .any(|i| glob(rest, &after[i..]))
        }
    }
}

/// Whether a `Bash(<pattern>)` rule covers `command`. `prefix:*` is the legacy form of
/// `prefix *`. shortcut: a bare `Bash(git push *)` rule does not cover `git push` with no
/// argument (Claude Code might); the answer is then "no opinion", never a wrong `allow`.
fn bash_rule_covers(rule: &str, command: &str) -> bool {
    let Some(pattern) = rule.strip_prefix("Bash(").and_then(|r| r.strip_suffix(')')) else {
        return false;
    };
    match pattern.strip_suffix(":*") {
        Some(prefix) => command == prefix || command.starts_with(&format!("{prefix} ")),
        None => glob(pattern, command),
    }
}

fn allow() -> String {
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": { "behavior": "allow" },
        }
    })
    .to_string()
}

/// Whether the working directory of the call is, once resolved, inside `<root>/.armadai`:
/// a `cd` there persists across calls, and `cp ../../x .` then lands in the state folder
/// without naming it. Anything that does not resolve counts as inside.
fn cwd_in_armadai_dir(v: &Value, root: &Path) -> bool {
    let cwd = v
        .get("cwd")
        .and_then(Value::as_str)
        .and_then(|c| std::fs::canonicalize(c).ok());
    let dir = std::fs::canonicalize(root.join(".armadai")).ok();
    match (cwd, dir) {
        (Some(cwd), Some(dir)) => cwd.starts_with(dir),
        _ => true,
    }
}

/// The whole decision, pure but for resolving paths under `root` (the project). `None` is
/// "no opinion". `state` is `None` when the file is
/// missing or does not read; `rules` are the `allow` rules of `level`; `marked_since(step,
/// since)` says whether a delegation to `step` was recorded since `since`.
pub fn decide(
    raw: &str,
    root: &Path,
    level: &str,
    rules: &[String],
    state: Option<&GrantState>,
    now: DateTime<Utc>,
    marked_since: &dyn Fn(&str, DateTime<Utc>) -> bool,
) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    if v.get("hook_event_name").and_then(Value::as_str)? != "PermissionRequest" {
        return None;
    }
    let tool = v.get("tool_name").and_then(Value::as_str)?;
    let active = state?.active.get(level)?;
    // a level with no bound never ends: it is not a grant
    if active.expires_at.is_none() && active.until_step.is_none() {
        return None;
    }
    if active.expires_at.is_some_and(|end| now >= end) {
        return None;
    }
    if let Some(step) = &active.until_step
        && marked_since(step, active.since)
    {
        return None;
    }
    if tool != "Bash" {
        return None;
    }
    let command = v.pointer("/tool_input/command").and_then(Value::as_str)?;
    if cwd_in_armadai_dir(&v, root) {
        return None;
    }
    (is_simple(command)
        && !names_grant_machinery(command)
        && rules.iter().any(|r| bash_rule_covers(r, command)))
    .then(allow)
}

/// Whether a `mark` line for `step` (`<flow>/<step>`, or a bare step id) was logged at or
/// after `since` in a session log under `root`. Anything unreadable counts as marked: the
/// level is then over.
pub fn marked_since_in_logs(root: &Path, step: &str, since: DateTime<Utc>) -> bool {
    let dir = root.join(crate::flow_status::record::LOGS_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    let since_secs = since.timestamp().max(0) as u64;
    // marks are logged to the second: compare a file's time at that precision too
    let since_floor = DateTime::<Utc>::from_timestamp(since.timestamp(), 0).unwrap_or(since);
    for entry in entries {
        let Ok(entry) = entry else { return true };
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        // a log not touched since `since` holds no later line
        let touched = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .map(DateTime::<Utc>::from);
        if touched.is_some_and(|t| t < since_floor) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            return true;
        };
        for line in text.lines() {
            // a line torn by a concurrent write, in a log touched since `since`, may have
            // been the mark: the level is over
            let Ok(ev) = serde_json::from_str::<crate::flow_status::record::FlowEvent>(line) else {
                if line.trim().is_empty() {
                    continue;
                }
                return true;
            };
            if ev.event != crate::flow_status::record::EventKind::Mark || ev.ts < since_secs {
                continue;
            }
            let (Some(f), Some(s)) = (ev.flow.as_deref(), ev.step.as_deref()) else {
                continue;
            };
            if step == format!("{f}/{s}") || step == s {
                return true;
            }
        }
    }
    false
}

/// The state file of `root`: `None` when missing or unreadable.
pub fn read_state(root: &Path) -> Option<GrantState> {
    let text = std::fs::read_to_string(root.join(STATE_FILE)).ok()?;
    let state: GrantState = armadai_model::yaml::from_str(&text).ok()?;
    (state.version == 1).then_some(state)
}

/// The level's `allow` rules and the project root, read as the gate reads the project.
fn project_rules(raw: &str, level: &str) -> Option<(PathBuf, Vec<String>)> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let cwd = v.get("cwd").and_then(Value::as_str)?;
    let root = super::policy_gate::resolve_project(Path::new(cwd))?;
    let view = crate::pack::effective::open(&root, crate::pack::effective::Vars::Literal).ok()?;
    let (project, _) = view.load_checked().ok()?;
    let grant = project.manifest.policy.grants.get(level)?;
    Some((root, grant.allow.clone()))
}

/// Read a hook payload from stdin and emit a decision for `level`. Never fails the hook.
pub fn grant_from_stdin(level: &str) -> anyhow::Result<()> {
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return Ok(());
    }
    let Some((root, rules)) = project_rules(&raw, level) else {
        return Ok(());
    };
    let state = read_state(&root);
    let marked = |step: &str, since: DateTime<Utc>| marked_since_in_logs(&root, step, since);
    if let Some(json) = decide(
        &raw,
        &root,
        level,
        &rules,
        state.as_ref(),
        Utc::now(),
        &marked,
    ) {
        println!("{json}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project root with a `.armadai/` folder, shared: tests only read from it.
    fn root() -> &'static Path {
        static ROOT: std::sync::LazyLock<tempfile::TempDir> = std::sync::LazyLock::new(|| {
            let d = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(d.path().join(".armadai/state")).unwrap();
            std::fs::create_dir_all(d.path().join("src")).unwrap();
            d
        });
        ROOT.path()
    }

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    const NOW: &str = "2026-11-10T10:00:00Z";
    const END: &str = "2026-11-10T11:00:00Z";
    const SINCE: &str = "2026-11-10T09:00:00Z";

    fn bash(command: &str) -> String {
        serde_json::json!({
            "session_id": "s", "cwd": root(), "hook_event_name": "PermissionRequest",
            "tool_name": "Bash", "tool_input": { "command": command },
        })
        .to_string()
    }

    fn tool(name: &str) -> String {
        serde_json::json!({
            "session_id": "s", "cwd": root(), "hook_event_name": "PermissionRequest",
            "tool_name": name, "tool_input": { "file_path": "/x" },
        })
        .to_string()
    }

    fn state(expires: Option<&str>, until: Option<&str>) -> GrantState {
        GrantState {
            version: 1,
            active: BTreeMap::from([(
                "lvl".to_string(),
                ActiveGrant {
                    since: at(SINCE),
                    expires_at: expires.map(at),
                    until_step: until.map(Into::into),
                },
            )]),
        }
    }

    fn rules(r: &[&str]) -> Vec<String> {
        r.iter().map(|s| s.to_string()).collect()
    }

    const NEVER: &dyn Fn(&str, DateTime<Utc>) -> bool = &|_, _| false;
    const ALLOW: &str = r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#;

    fn run(raw: &str, st: Option<&GrantState>, rs: &[&str]) -> Option<String> {
        decide(raw, root(), "lvl", &rules(rs), st, at(NOW), NEVER)
    }

    #[test]
    fn an_active_level_allows_a_covered_simple_command() {
        let st = state(Some(END), None);
        for command in [
            "git push origin feature/x",
            "git push origin feature/a --force",
        ] {
            assert_eq!(
                run(
                    &bash(command),
                    Some(&st),
                    &["Bash(git push origin feature/*)"]
                )
                .as_deref(),
                Some(ALLOW),
                "{command}"
            );
        }
        // the legacy `prefix:*` form
        assert_eq!(
            run(&bash("git push origin x"), Some(&st), &["Bash(git push:*)"]).as_deref(),
            Some(ALLOW)
        );
    }

    #[test]
    fn a_command_no_rule_covers_gets_no_opinion() {
        let st = state(Some(END), None);
        let r = ["Bash(git push origin feature/*)"];
        for command in [
            "git push origin main",
            "git pull origin feature/x",
            "rm -rf feature/",
        ] {
            assert_eq!(run(&bash(command), Some(&st), &r), None, "{command}");
        }
        // an exact rule covers only that command
        assert_eq!(run(&bash("ls -la"), Some(&st), &["Bash(ls)"]), None);
        assert_eq!(
            run(&bash("ls"), Some(&st), &["Bash(ls)"]).as_deref(),
            Some(ALLOW)
        );
    }

    #[test]
    fn the_end_instant_is_not_covered() {
        let st = state(Some(END), None);
        let r = ["Bash(echo *)"];
        let call = bash("echo hi");
        let at_end =
            |now: &str| decide(&call, root(), "lvl", &rules(&r), Some(&st), at(now), NEVER);
        assert_eq!(at_end("2026-11-10T10:59:59Z").as_deref(), Some(ALLOW));
        assert_eq!(at_end(END), None);
        assert_eq!(at_end("2026-11-10T11:00:01Z"), None);
    }

    #[test]
    fn a_delegation_to_the_until_step_ends_the_level() {
        let st = state(None, Some("evolution/publish"));
        let r = ["Bash(echo *)"];
        let call = bash("echo hi");
        let seen = std::cell::RefCell::new(Vec::new());
        let marked = |step: &str, since: DateTime<Utc>| {
            seen.borrow_mut().push((step.to_string(), since));
            false
        };
        assert_eq!(
            decide(
                &call,
                root(),
                "lvl",
                &rules(&r),
                Some(&st),
                at(NOW),
                &marked
            )
            .as_deref(),
            Some(ALLOW)
        );
        // asked about the step and the start of the level, nothing else
        assert_eq!(
            *seen.borrow(),
            [("evolution/publish".to_string(), at(SINCE))]
        );
        let marked = |_: &str, _: DateTime<Utc>| true;
        assert_eq!(
            decide(
                &call,
                root(),
                "lvl",
                &rules(&r),
                Some(&st),
                at(NOW),
                &marked
            ),
            None
        );
        // a mark also ends a level that has a time bound
        let both = state(Some(END), Some("evolution/publish"));
        assert_eq!(
            decide(
                &call,
                root(),
                "lvl",
                &rules(&r),
                Some(&both),
                at(NOW),
                &marked
            ),
            None
        );
    }

    #[test]
    fn anything_unestablished_is_no_opinion() {
        let r = ["Bash(echo *)"];
        let call = bash("echo hi");
        let st = state(Some(END), None);
        // the state does not read (or is missing)
        assert_eq!(run(&call, None, &r), None);
        // another level, or none active
        assert_eq!(
            decide(
                &call,
                root(),
                "other",
                &rules(&r),
                Some(&st),
                at(NOW),
                NEVER
            ),
            None
        );
        assert_eq!(run(&call, Some(&GrantState::default()), &r), None);
        // a level with no bound is no grant
        assert_eq!(run(&call, Some(&state(None, None)), &r), None);
        // not a PermissionRequest, not JSON, no command
        let pre = call.replace("PermissionRequest", "PreToolUse");
        assert_eq!(run(&pre, Some(&st), &r), None);
        assert_eq!(run("{not json", Some(&st), &r), None);
        assert_eq!(run("", Some(&st), &r), None);
        let nocmd = serde_json::json!({"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{}}).to_string();
        assert_eq!(run(&nocmd, Some(&st), &r), None);
        // no rule at all
        assert_eq!(run(&call, Some(&st), &[]), None);
    }

    /// Measured on 2026-11-10: the `if` filter runs a hook for a compound or unreadable
    /// command, so the hook itself answers only for a simple one.
    #[test]
    fn a_compound_substituted_or_unreadable_command_gets_no_opinion() {
        let st = state(Some(END), None);
        // each is covered by its rule's pattern: only the shell filter refuses it
        for (command, rule) in [
            ("echo grant-probe x && echo y", "Bash(echo grant-probe *)"),
            ("echo grant-probe x || echo y", "Bash(echo grant-probe *)"),
            ("echo grant-probe x ; echo y", "Bash(echo grant-probe *)"),
            ("echo grant-probe x | cat", "Bash(echo grant-probe *)"),
            ("echo grant-probe x & echo y", "Bash(echo grant-probe *)"),
            ("echo grant-probe x\necho y", "Bash(echo grant-probe *)"),
            ("echo grant-probe $(date)", "Bash(echo grant-probe *)"),
            ("echo grant-probe `date`", "Bash(echo grant-probe *)"),
            ("echo grant-probe x > out", "Bash(echo grant-probe *)"),
            ("echo grant-probe $HOME", "Bash(echo grant-probe *)"),
            ("echo grant-probe 'x'", "Bash(echo grant-probe *)"),
            (r#"eval "$(printf "ec""ho grant-probe z")""#, "Bash(eval *)"),
        ] {
            assert_eq!(run(&bash(command), Some(&st), &[rule]), None, "{command:?}");
        }
        // control: the same rule covers the simple form
        assert_eq!(
            run(
                &bash("echo grant-probe x"),
                Some(&st),
                &["Bash(echo grant-probe *)"]
            )
            .as_deref(),
            Some(ALLOW)
        );
    }

    /// A file tool is never answered: its rule is anchored and its path resolved by Claude
    /// Code (symlinks included), which a hook does not redo. Measured on 2026-11-10: an
    /// `Edit` through a link under `src/` wrote outside it under an `Edit(/src/**)` level.
    #[test]
    fn a_tool_other_than_bash_gets_no_opinion() {
        let st = state(Some(END), None);
        let edit = |path: &str| {
            serde_json::json!({
                "session_id": "s", "cwd": root(), "hook_event_name": "PermissionRequest",
                "tool_name": "Edit", "tool_input": { "file_path": path },
            })
            .to_string()
        };
        for path in ["/etc/passwd", "/src/a.rs", "/src/lnk/t.txt"] {
            assert_eq!(
                run(&edit(path), Some(&st), &["Edit(/src/*)"]),
                None,
                "{path}"
            );
        }
        for name in [
            "Edit",
            "Write",
            "NotebookEdit",
            "Read",
            "WebFetch",
            "mcp__s__t",
        ] {
            assert_eq!(
                run(&tool(name), Some(&st), &[&format!("{name}(*)"), name]),
                None,
                "{name}"
            );
        }
        // control: the same state answers a Bash rule
        assert_eq!(
            run(&bash("echo hi"), Some(&st), &["Bash(echo *)"]).as_deref(),
            Some(ALLOW)
        );
    }

    fn bash_in(cwd: &Path, command: &str) -> String {
        serde_json::json!({
            "session_id": "s", "cwd": cwd, "hook_event_name": "PermissionRequest",
            "tool_name": "Bash", "tool_input": { "command": command },
        })
        .to_string()
    }

    /// Measured on 2026-11-10: `cd .armadai/state` needs no permission and persists, so the
    /// next call arrives with that `cwd`, and `cp ../../x .` writes there without naming
    /// the file. A level gives no opinion from inside `.armadai/`, links included.
    #[test]
    fn a_call_from_inside_the_armadai_folder_gets_no_opinion() {
        let st = state(Some(END), None);
        let r = ["Bash(cp *)"];
        let call = |cwd: &Path| run(&bash_in(cwd, "cp -R ../../src/ ."), Some(&st), &r);
        let state_dir = root().join(".armadai/state");
        assert_eq!(call(&state_dir), None);
        assert_eq!(call(&root().join(".armadai")), None);
        // a spelling that resolves into it, and a link to it
        assert_eq!(call(&root().join("src/../.armadai/state")), None);
        #[cfg(unix)]
        {
            let link = root().join("src/to-state");
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(&state_dir, &link).unwrap();
            assert_eq!(call(&link), None);
        }
        // from the project root, or `src/`, the same command is covered
        assert_eq!(call(root()).as_deref(), Some(ALLOW));
        assert_eq!(call(&root().join("src")).as_deref(), Some(ALLOW));
        // a folder that does not resolve, or no `cwd` at all: unestablished
        assert_eq!(call(&root().join("nope")), None);
        let nocwd = serde_json::json!({"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"cp a b"}}).to_string();
        assert_eq!(run(&nocwd, Some(&st), &r), None);
    }

    /// `grantſ.yaml` (U+017F) is `grants.yaml` on a case-insensitive disk (measured on
    /// APFS: the same inode).
    #[test]
    fn a_command_with_a_character_outside_ascii_gets_no_opinion() {
        let st = state(Some(END), None);
        for command in ["cp ../../evil.yaml grantſ.yaml", "cp x é", "cp x \u{200b}y"] {
            assert_eq!(
                run(&bash(command), Some(&st), &["Bash(cp *)"]),
                None,
                "{command}"
            );
        }
        assert_eq!(
            run(&bash("cp x y"), Some(&st), &["Bash(cp *)"]).as_deref(),
            Some(ALLOW)
        );
    }

    /// A level, however broad, never answers for a command that names the state file or
    /// `armadai`: those forms are not all covered by the `deny` rules link writes.
    #[test]
    fn a_command_naming_the_grant_machinery_gets_no_opinion() {
        let st = state(Some(END), None);
        for (command, rule) in [
            ("cp x .armadai/state/grants.yaml", "Bash(cp *)"),
            ("cat .armadai/state/grants.yaml", "Bash(cat *)"),
            ("tee grants.yaml", "Bash(tee *)"),
            ("armadai grant lvl", "Bash(armadai *)"),
            ("./target/debug/armadai grant lvl", "Bash(./target/debug/*)"),
            ("env ARMADAI_CONFIG_DIR=x ls", "Bash(env *)"),
            ("sh -c ls", "Bash(sh *)"),
        ] {
            let want_none = command != "sh -c ls";
            assert_eq!(
                run(&bash(command), Some(&st), &[rule]).is_none(),
                want_none,
                "{command}"
            );
        }
    }

    #[test]
    fn the_state_reads_back_and_a_bad_one_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_state(dir.path()), None);
        let path = dir.path().join(STATE_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "version: 1\nactive:\n  lvl:\n    since: \"2026-11-10T09:00:00Z\"\n    expires_at: \"2026-11-10T11:00:00Z\"\n    until_step: evolution/publish\n",
        )
        .unwrap();
        assert_eq!(
            read_state(dir.path()),
            Some(state(Some(END), Some("evolution/publish")))
        );
        for bad in [
            "version: 2\nactive: {}\n",
            "version: [\n",
            "active: 3\n",
            "version: 1\nactive:\n  l: { since: nope }\n",
        ] {
            std::fs::write(&path, bad).unwrap();
            assert_eq!(read_state(dir.path()), None, "{bad}");
        }
    }

    fn log(root: &Path, name: &str, lines: &[&str]) {
        let dir = root.join(crate::flow_status::record::LOGS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn a_mark_for_the_step_since_the_start_is_found_in_the_logs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // well before the files below are written, whatever the clock says
        let since = at("2026-01-01T00:00:00Z");
        let after = since.timestamp() + 60;
        let before = since.timestamp() - 60;
        let mark = |ts: i64, flow: &str, step: &str| {
            format!(
                r#"{{"ts":{ts},"event":"mark","agent_type":"a","flow":"{flow}","step":"{step}"}}"#
            )
        };
        // no logs at all: nothing recorded
        assert!(!marked_since_in_logs(root, "evolution/publish", since));
        log(
            root,
            "a.jsonl",
            &[
                &mark(before, "evolution", "publish"),
                &mark(after, "evolution", "review"),
                &mark(after, "other", "publish"),
                "",
                r#"{"ts":1,"event":"start","agent_id":"x","agent_type":"a","state":null}"#,
            ],
        );
        // an older mark, another step and another flow do not count
        assert!(!marked_since_in_logs(root, "evolution/publish", since));
        log(root, "b.jsonl", &[&mark(after, "evolution", "publish")]);
        assert!(marked_since_in_logs(root, "evolution/publish", since));
        // a bare step id names the step of any flow
        assert!(marked_since_in_logs(root, "review", since));
        // at the very second counts
        log(
            root,
            "b.jsonl",
            &[&mark(since.timestamp(), "evolution", "publish")],
        );
        assert!(marked_since_in_logs(root, "evolution/publish", since));
        // a log last written before `since` is not read
        let old = std::fs::File::options()
            .write(true)
            .open(
                root.join(crate::flow_status::record::LOGS_DIR)
                    .join("b.jsonl"),
            )
            .unwrap();
        old.set_modified(std::time::UNIX_EPOCH).unwrap();
        std::fs::remove_file(
            root.join(crate::flow_status::record::LOGS_DIR)
                .join("a.jsonl"),
        )
        .unwrap();
        assert!(!marked_since_in_logs(root, "evolution/publish", since));
    }

    #[test]
    fn a_torn_line_in_a_log_touched_since_the_start_counts_as_marked() {
        let dir = tempfile::tempdir().unwrap();
        let since = at("2026-01-01T00:00:00Z");
        let torn = r#"{"ts":1794400000,"event":"mark","agent_type":"a","flow":"evolution","ste"#;
        log(dir.path(), "a.jsonl", &[torn]);
        assert!(marked_since_in_logs(dir.path(), "evolution/publish", since));
        // a log not touched since is not read, torn or not
        let path = dir
            .path()
            .join(crate::flow_status::record::LOGS_DIR)
            .join("a.jsonl");
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH)
            .unwrap();
        assert!(!marked_since_in_logs(
            dir.path(),
            "evolution/publish",
            since
        ));
    }

    #[test]
    fn a_log_touched_in_the_second_of_the_start_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let t0 = 1_700_000_000_i64;
        let mark =
            format!(r#"{{"ts":{t0},"event":"mark","agent_type":"a","flow":"f","step":"s"}}"#);
        log(dir.path(), "a.jsonl", &[&mark]);
        let path = dir
            .path()
            .join(crate::flow_status::record::LOGS_DIR)
            .join("a.jsonl");
        // written 0.3 s into the second, the level started 0.6 s into it
        let mtime = std::time::UNIX_EPOCH + std::time::Duration::new(t0 as u64, 300_000_000);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        let since = DateTime::<Utc>::from_timestamp(t0, 600_000_000).unwrap();
        assert!(marked_since_in_logs(dir.path(), "f/s", since));
    }

    #[test]
    fn an_unreadable_log_directory_counts_as_marked() {
        let dir = tempfile::tempdir().unwrap();
        // `claude` is a file where the logs directory should be
        let state = dir.path().join(".armadai/state");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("claude"), "x").unwrap();
        assert!(marked_since_in_logs(dir.path(), "a/b", at(SINCE)));
    }

    #[test]
    fn a_log_that_does_not_read_counts_as_marked() {
        let dir = tempfile::tempdir().unwrap();
        // a directory named like a log: it opens, it does not read
        let logs = dir.path().join(crate::flow_status::record::LOGS_DIR);
        std::fs::create_dir_all(logs.join("s.jsonl")).unwrap();
        assert!(marked_since_in_logs(
            dir.path(),
            "a/b",
            at("2026-01-01T00:00:00Z")
        ));
    }
}
