//! `armadai journal add|show` end to end: two checkouts of one repository, the lines
//! printed, the refusals, and two processes appending to one file.

use std::path::{Path, PathBuf};

use assert_cmd::Command;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    /// `main/` is the main checkout, `wt/` a linked worktree of it; both hold a project.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        let wt = dir.path().join("wt");
        std::fs::create_dir_all(main.join(".git/worktrees/wt")).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", main.join(".git/worktrees/wt").display()),
        )
        .unwrap();
        for d in [&main, &wt] {
            std::fs::create_dir_all(d.join(".armadai")).unwrap();
            std::fs::write(d.join(".armadai/project.yaml"), "schema_version: 1\n").unwrap();
        }
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        Env { dir }
    }

    fn main(&self) -> PathBuf {
        self.dir.path().join("main")
    }

    fn wt(&self) -> PathBuf {
        self.dir.path().join("wt")
    }

    fn cmd(&self, at: &Path) -> Command {
        let mut c = Command::cargo_bin("armadai").unwrap();
        c.current_dir(at)
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path())
            .env("NO_COLOR", "1")
            .timeout(std::time::Duration::from_secs(60));
        c
    }

    fn run(&self, at: &Path, args: &[&str]) -> (Option<i32>, String, String) {
        let out = self.cmd(at).args(args).output().unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

fn local(ts: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts as i64, 0)
        .unwrap()
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

#[test]
fn an_entry_written_in_a_worktree_is_read_from_the_main_checkout() {
    let e = Env::new();
    let r = e.run(
        &e.wt(),
        &[
            "journal",
            "add",
            "--lot",
            "menu",
            "--kind",
            "decision",
            "--by",
            "lead",
            "Keep 1.3.0",
        ],
    );
    assert_eq!(
        r,
        (
            Some(0),
            "added to the journal of lot menu\n".into(),
            String::new()
        )
    );
    let file = e.main().join(".armadai/state/journal-menu.jsonl");
    assert!(
        !e.wt().join(".armadai/state").exists(),
        "the worktree gets no state/"
    );
    let written = std::fs::read_to_string(&file).unwrap();
    let v: serde_json::Value = serde_json::from_str(written.trim_end()).unwrap();
    let ts = v["ts"].as_u64().unwrap();
    assert_eq!(
        written,
        format!(
            "{{\"schema\":\"armadai.journal/1\",\"ts\":{ts},\"kind\":\"decision\",\"lot\":\"menu\",\"by\":\"lead\",\"text\":\"Keep 1.3.0\"}}\n"
        )
    );

    let r = e.run(&e.main(), &["journal", "show", "--lot", "menu"]);
    assert_eq!(
        r,
        (
            Some(0),
            format!("{}  menu  decision  lead  Keep 1.3.0\n", local(ts)),
            String::new()
        )
    );
    let r = e.run(&e.main(), &["journal", "show", "--lot", "menu", "--json"]);
    assert_eq!(
        r,
        (
            Some(0),
            format!(
                "{{\"schema\":\"armadai.journal/1\",\"entries\":[{}]}}\n",
                written.trim_end()
            ),
            String::new()
        )
    );
}

#[test]
fn an_entry_without_a_lot_shows_as_all_and_with_every_lot() {
    let e = Env::new();
    let r = e.run(
        &e.main(),
        &["journal", "add", "--kind", "state", "hold for all"],
    );
    assert_eq!(r.1, "added to the journal of every lot\n");
    let written = std::fs::read_to_string(e.main().join(".armadai/state/journal.jsonl")).unwrap();
    let ts = serde_json::from_str::<serde_json::Value>(written.trim_end()).unwrap()["ts"]
        .as_u64()
        .unwrap();
    let r = e.run(&e.main(), &["journal", "show", "--lot", "menu"]);
    assert_eq!(
        r,
        (
            Some(0),
            format!("{}  (all)  state     -  hold for all\n", local(ts)),
            String::new()
        )
    );
}

#[test]
fn an_unknown_kind_is_refused_by_clap_with_the_list_and_nothing_is_written() {
    let e = Env::new();
    let (code, _, err) = e.run(&e.main(), &["journal", "add", "--kind", "other", "x"]);
    assert_ne!(code, Some(0));
    assert!(
        err.contains("[possible values: decision, state, note]"),
        "{err}"
    );
    assert!(!e.main().join(".armadai/state").exists());
}

#[test]
fn an_empty_journal_prints_nothing_and_succeeds() {
    let e = Env::new();
    assert_eq!(
        e.run(&e.main(), &["journal", "show"]),
        (Some(0), String::new(), String::new())
    );
}

#[test]
fn a_refused_lot_exits_non_zero_with_the_rule() {
    let e = Env::new();
    let (code, out, err) = e.run(&e.main(), &["journal", "add", "--lot", "a_b", "x"]);
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert!(
        err.contains("lot a_b cannot name a journal: 1 to 40 letters, digits, . or -, starting with a letter or a digit"),
        "{err}"
    );
}

#[test]
fn a_line_written_by_the_first_version_is_read_back_unchanged() {
    let e = Env::new();
    let fixture = include_str!("fixtures/journal/v1.jsonl");
    let state = e.main().join(".armadai/state");
    std::fs::create_dir_all(&state).unwrap();
    let mut lines = fixture.lines();
    std::fs::write(
        state.join("journal-menu.jsonl"),
        format!("{}\n", lines.next().unwrap()),
    )
    .unwrap();
    std::fs::write(
        state.join("journal.jsonl"),
        format!("{}\n", lines.next().unwrap()),
    )
    .unwrap();
    let (code, out, _) = e.run(&e.main(), &["journal", "show", "--json"]);
    assert_eq!(code, Some(0));
    let want: Vec<String> = fixture.lines().map(String::from).collect();
    assert_eq!(
        out,
        format!(
            "{{\"schema\":\"armadai.journal/1\",\"entries\":[{}]}}\n",
            want.join(",")
        )
    );
}

#[test]
fn the_fixtures_hold_nothing_private() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/journal");
    let mut seen = 0;
    for f in std::fs::read_dir(dir).unwrap().flatten() {
        let s = std::fs::read_to_string(f.path()).unwrap();
        for bad in ["/Users/", "/home/"] {
            assert!(!s.contains(bad), "{} holds {bad}", f.path().display());
        }
        seen += 1;
    }
    assert!(seen > 0);
}

/// Two processes appending 50 lines each to one lot, five times: no line is cut or mixed.
#[test]
fn two_processes_appending_leave_every_line_whole() {
    let e = Env::new();
    // long enough that a write split in two can be interleaved with the other process
    let text = "x".repeat(1500);
    let file = e.main().join(".armadai/state/journal-menu.jsonl");
    for pass in 0..5 {
        let _ = std::fs::remove_file(&file);
        std::thread::scope(|s| {
            for who in ["p", "q"] {
                let (e, text) = (&e, &text);
                s.spawn(move || {
                    for i in 0..50 {
                        let t = format!("{who}{i} {text}");
                        let r = e.run(&e.main(), &["journal", "add", "--lot", "menu", &t]);
                        assert_eq!(r.0, Some(0), "{r:?}");
                    }
                });
            }
        });
        let body = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<_> = body.lines().collect();
        assert_eq!(lines.len(), 100, "pass {pass}");
        for l in &lines {
            let v: serde_json::Value = serde_json::from_str(l)
                .unwrap_or_else(|err| panic!("pass {pass}: not JSON ({err}): {l:.80}"));
            assert!(v["text"].as_str().unwrap().ends_with(&text), "pass {pass}");
        }
    }
}
