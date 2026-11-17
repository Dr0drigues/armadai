//! `armadai grant` and `armadai revoke` end to end: the lines printed, the state file,
//! the refusals, and `unlink` taking the state away.

use std::path::{Path, PathBuf};

use assert_cmd::Command;

const MANIFEST: &str = "schema_version: 1\ncoordinator: lead\npolicy:\n  grants:\n    auto-until-mr:\n      description: Push feature branches until the MR\n      allow: [\"Bash(git push origin feature/*)\", \"Bash(git commit *)\"]\n      until: publish\n    read-logs:\n      description: Lire les journaux de recette\n      allow: [\"Bash(tail *)\"]\n";

struct Env {
    dir: tempfile::TempDir,
    extra: Vec<(String, String)>,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("project/.armadai");
        std::fs::create_dir_all(a.join("agents")).unwrap();
        std::fs::create_dir_all(a.join("flows")).unwrap();
        std::fs::write(a.join("project.yaml"), MANIFEST).unwrap();
        for agent in ["lead", "dev"] {
            std::fs::write(
                a.join(format!("agents/{agent}.md")),
                "---\ndescription: d\n---\nB\n",
            )
            .unwrap();
        }
        std::fs::write(
            a.join("flows/evolution.yaml"),
            "steps:\n  build: { agent: dev }\n  publish: { agent: lead }\nlinks:\n  - { from: build, to: publish }\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("project/.git")).unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        Env { dir, extra: vec![] }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    fn run(&self, args: &[&str]) -> (Option<i32>, String, String) {
        self.run_in(&self.root(), args)
    }

    fn run_in(&self, cwd: &Path, args: &[&str]) -> (Option<i32>, String, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(cwd)
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path())
            .env("CLAUDE_CONFIG_DIR", self.dir.path().join("claude"))
            .env("NO_COLOR", "1")
            // the tests run in a Claude Code shell too: grant refuses there
            .env_remove("CLAUDECODE")
            .envs(self.extra.iter().cloned())
            .args(args)
            .timeout(std::time::Duration::from_secs(30))
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// A linked worktree `name` of the project, with its own (fake) git dir and no
    /// `.armadai`: the project root the command resolves is the main checkout's, the
    /// same for every worktree, so only the cwd tells the checkouts apart.
    fn worktree(&self, name: &str) -> PathBuf {
        let base = self.dir.path().canonicalize().unwrap();
        let wt = base.join(name);
        std::fs::create_dir_all(&wt).unwrap();
        let gd = base.join(format!("project/.git/worktrees/{name}"));
        std::fs::create_dir_all(&gd).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gd.display())).unwrap();
        wt
    }

    fn link(&self) {
        let (code, _, err) = self.run(&["link", "--target", "claude"]);
        assert_eq!(code, Some(0), "{err}");
    }

    fn state(&self) -> Option<String> {
        std::fs::read_to_string(self.root().join(".git/armadai/grants.yaml")).ok()
    }
}

fn ok(r: (Option<i32>, String, String)) -> String {
    assert_eq!((r.0, r.2.as_str()), (Some(0), ""), "{r:?}");
    r.1
}

fn refused(r: (Option<i32>, String, String)) -> String {
    assert_eq!((r.0, r.1.as_str()), (Some(1), ""), "{r:?}");
    r.2
}

#[test]
fn the_list_then_a_grant_then_a_revoke() {
    let env = Env::new();
    env.link();
    assert_eq!(
        ok(env.run(&["grant"])),
        "grant levels (policy.grants)\n  auto-until-mr  inactive  Push feature branches until the MR\n  read-logs      inactive  Lire les journaux de recette\n"
    );
    assert_eq!(env.state(), None);
    assert_eq!(
        ok(env.run(&["grant", "auto-until-mr"])),
        "granted auto-until-mr until a delegation to evolution/publish is recorded (2 rules)\n"
    );
    let state = env.state().unwrap();
    assert!(
        state.starts_with("version: 1\nactive:\n  auto-until-mr:\n    since: \""),
        "{state}"
    );
    assert!(
        state.ends_with("\"\n    until_step: evolution/publish\n"),
        "{state}"
    );
    let list = ok(env.run(&["grant"]));
    assert!(
        list.contains("  auto-until-mr  active until evolution/publish starts, since "),
        "{list}"
    );
    assert!(list.contains("  read-logs      inactive  Lire"), "{list}");
    assert_eq!(
        ok(env.run(&["revoke", "auto-until-mr"])),
        "revoked auto-until-mr\n"
    );
    assert_eq!(ok(env.run(&["revoke", "--all"])), "no grant is active\n");
    assert!(!env.state().unwrap().contains("auto-until-mr"));
}

#[test]
fn a_time_bound_names_its_hour_and_two_levels_revoke_together() {
    let env = Env::new();
    env.link();
    let line = ok(env.run(&["grant", "read-logs", "--for", "1h30m"]));
    let hour = line
        .strip_prefix("granted read-logs until ")
        .and_then(|r| r.strip_suffix(" (1 rule); armadai revoke read-logs ends it\n"))
        .unwrap_or_else(|| panic!("{line}"));
    assert_eq!(hour.len(), 5);
    assert_eq!(hour.as_bytes()[2], b':');
    ok(env.run(&["grant", "auto-until-mr", "--until-time", "23:59"]));
    assert_eq!(
        ok(env.run(&["revoke", "--all"])),
        "revoked 2 levels: auto-until-mr, read-logs\n"
    );
}

#[test]
fn every_refusal_says_what_to_do_and_writes_nothing() {
    let env = Env::new();
    assert_eq!(
        refused(env.run(&["grant", "read-logs"])),
        "Error: grant read-logs needs a bound: --until <step>, --for <duration> or --until-time <HH:MM>, or until: in policy.grants\n"
    );
    assert_eq!(
        refused(env.run(&["grant", "x", "--for", "1h"])),
        "Error: unknown grant level x; levels: auto-until-mr, read-logs\n"
    );
    // no `link` yet: no marker hook
    assert_eq!(
        refused(env.run(&["grant", "auto-until-mr"])),
        "Error: --until publish needs the delegation markers that the flow recording hook writes; run armadai link --target claude first\n"
    );
    assert_eq!(
        refused(env.run(&["grant", "read-logs", "--for", "soon"])),
        "Error: invalid duration soon; write 30m, 2h or 1h30m\n"
    );
    env.link();
    for local in [".claude/settings.local.json", ".claude/settings.json"] {
        let path = env.root().join(local);
        let before = std::fs::read_to_string(&path).ok();
        let mut doc: serde_json::Value = match &before {
            Some(t) => serde_json::from_str(t).unwrap(),
            None => serde_json::json!({}),
        };
        doc["permissions"]["deny"] = serde_json::json!(["Bash(tail *)"]);
        std::fs::write(&path, doc.to_string()).unwrap();
        assert_eq!(
            refused(env.run(&["grant", "read-logs", "--for", "1h"])),
            format!(
                "Error: grant read-logs refused: its rule Bash(tail *) is a deny rule of {local}, and a grant never lifts a deny\n"
            )
        );
        match before {
            Some(t) => std::fs::write(&path, t).unwrap(),
            None => std::fs::remove_file(&path).unwrap(),
        }
    }
    assert_eq!(env.state(), None);
    assert_eq!(
        refused(env.run(&["revoke", "read-logs"])),
        "Error: grant read-logs is not active\n"
    );
}

#[test]
fn a_level_without_a_bash_rule_is_refused_and_writes_nothing() {
    let env = Env::new();
    let manifest = MANIFEST.replace(
        "    read-logs:",
        "    docs-only:\n      allow: [\"Read(./docs/**)\", \"WebFetch\"]\n    read-logs:",
    );
    std::fs::write(env.root().join(".armadai/project.yaml"), manifest).unwrap();
    assert_eq!(
        refused(env.run(&["grant", "docs-only", "--for", "1h"])),
        "Error: grant docs-only has no Bash rule to answer; list its rules as Bash(...) in policy.grants, other rules are never granted\n"
    );
    assert_eq!(env.state(), None);
}

#[test]
fn clap_refuses_the_impossible_forms() {
    let env = Env::new();
    for args in [
        &["revoke"][..],
        &["revoke", "a", "--all"],
        &["grant", "--for", "1h"],
        &["grant", "read-logs", "--for", "1h", "--until-time", "18:00"],
    ] {
        let (code, out, _) = env.run(args);
        assert_eq!((code, out.as_str()), (Some(2), ""), "{args:?}");
    }
}

#[test]
fn unlink_takes_the_state_away() {
    let env = Env::new();
    env.link();
    ok(env.run(&["grant", "read-logs", "--for", "1h"]));
    assert!(env.state().is_some());
    let (code, _, err) = env.run(&["unlink", "--target", "claude", "--dry-run"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(env.state().is_some(), "a dry run removes nothing");
    let (code, _, err) = env.run(&["unlink", "--target", "claude"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(env.state(), None);
}

#[test]
fn a_grant_from_a_claude_code_shell_is_refused_and_a_list_is_not() {
    let mut env = Env::new();
    env.extra.push(("CLAUDECODE".into(), "1".into()));
    assert_eq!(
        refused(env.run(&["grant", "read-logs", "--for", "1h"])),
        "Error: grant is for the person at a terminal, and this shell belongs to a Claude Code session (CLAUDECODE is set); run armadai grant in your own terminal\n"
    );
    assert_eq!(env.state(), None);
    assert!(ok(env.run(&["grant"])).starts_with("grant levels"));
}

#[test]
fn a_level_whose_step_was_delegated_is_listed_ended() {
    let env = Env::new();
    env.link();
    ok(env.run(&["grant", "auto-until-mr"]));
    let logs = env.root().join(".armadai/state/claude");
    std::fs::create_dir_all(&logs).unwrap();
    let ts = chrono::Utc::now().timestamp() + 5;
    let mark = format!(
        "{{\"ts\":{ts},\"event\":\"mark\",\"agent_type\":\"lead\",\"flow\":\"evolution\",\"step\":\"publish\"}}\n"
    );
    std::fs::write(logs.join("s1.jsonl"), mark).unwrap();
    let list = ok(env.run(&["grant"]));
    assert!(
        list.contains("  auto-until-mr  ended (evolution/publish started)\n"),
        "{list}"
    );
}

const OLD_STATE: &str = ".armadai/state/grants.yaml";

#[test]
fn a_level_opened_in_one_worktree_is_not_open_in_another() {
    let env = Env::new();
    let (a, b) = (env.worktree("a"), env.worktree("b"));
    env.link();
    ok(env.run_in(&a, &["grant", "read-logs", "--for", "1h"]));
    let list = |wt: &Path| ok(env.run_in(wt, &["grant"]));
    assert!(
        list(&a).contains("  read-logs      active until "),
        "{}",
        list(&a)
    );
    assert_eq!(
        list(&b),
        "grant levels (policy.grants)\n  auto-until-mr  inactive  Push feature branches until the MR\n  read-logs      inactive  Lire les journaux de recette\n"
    );
    // a revoke typed in B does not end the level of A
    assert_eq!(
        refused(env.run_in(&b, &["revoke", "read-logs"])),
        "Error: grant read-logs is not active\n"
    );
    assert!(list(&a).contains("  read-logs      active until "));
    assert!(!env.root().join(OLD_STATE).exists());
    // `revoke` in A ends it there
    ok(env.run_in(&a, &["revoke", "read-logs"]));
    assert!(list(&a).contains("  read-logs      inactive"));
}

#[test]
fn a_dot_git_that_leads_nowhere_refuses_grant_and_revoke_and_writes_nothing() {
    let env = Env::new();
    env.link();
    let dot_git = env.root().canonicalize().unwrap().join(".git");
    std::fs::remove_dir(&dot_git).unwrap();
    std::fs::write(&dot_git, "").unwrap();
    for cmd in ["grant", "revoke"] {
        let args: &[&str] = if cmd == "grant" {
            &["grant", "read-logs", "--for", "1h"]
        } else {
            &["revoke", "--all"]
        };
        assert_eq!(
            refused(env.run(args)),
            format!(
                "Error: {cmd} refused: {} does not lead to a git directory, where this checkout keeps its grant levels; repair it (git worktree repair) and run the command again\n",
                dot_git.display()
            )
        );
    }
    assert!(!env.root().join(OLD_STATE).exists());
    assert!(!env.root().join(".git/armadai").exists());
}

#[test]
fn an_old_state_file_is_announced_at_each_grant_and_never_touched() {
    let env = Env::new();
    env.link();
    let old = env.root().join(OLD_STATE);
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, "version: 1\nactive: {}\n").unwrap();
    let before = (
        std::fs::read(&old).unwrap(),
        std::fs::metadata(&old).unwrap().modified().unwrap(),
    );
    let new = env
        .root()
        .canonicalize()
        .unwrap()
        .join(".git/armadai/grants.yaml");
    let note = format!(
        "note: .armadai/state/grants.yaml is no longer read: grant levels are now kept per checkout in {}, and a level opened before this version is off; delete the old file to drop this note\n",
        new.display()
    );
    // the list, a grant, and a second grant: the note every time, on stderr only
    for args in [
        &["grant"][..],
        &["grant", "read-logs", "--for", "1h"],
        &["grant", "read-logs", "--for", "1h"],
    ] {
        let (code, out, err) = env.run(args);
        assert_eq!((code, err.as_str()), (Some(0), note.as_str()), "{args:?}");
        assert!(!out.contains("note:"), "{out}");
    }
    // revoke does not repeat it
    let (code, _, err) = env.run(&["revoke", "--all"]);
    assert_eq!((code, err.as_str()), (Some(0), ""));
    let after = (
        std::fs::read(&old).unwrap(),
        std::fs::metadata(&old).unwrap().modified().unwrap(),
    );
    assert_eq!(before, after);
    // without the old file: no note
    std::fs::remove_file(&old).unwrap();
    assert_eq!(
        ok(env.run(&["grant"])).lines().next(),
        Some("grant levels (policy.grants)")
    );
}
