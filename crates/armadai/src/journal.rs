//! The lot journal: JSONL files under the main checkout's `.armadai/state/`, one per lot
//! (`journal-<lot in lowercase>.jsonl`) plus `journal.jsonl` for entries that hold for every
//! lot. Written by `armadai journal add` in one append, read by `armadai journal show`.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::flow_status::marker::is_valid_lot;

pub const SCHEMA: &str = "armadai.journal/1";
/// Characters of `text`.
pub const MAX_TEXT: usize = 2000;
/// Bytes of the written line, `\n` included.
pub const MAX_LINE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub schema: String,
    pub ts: u64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    pub text: String,
}

/// Closed on write (clap lists it); any string is read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Kind {
    Decision,
    State,
    Note,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Decision => "decision",
            Kind::State => "state",
            Kind::Note => "note",
        }
    }
}

/// `.armadai/state/` of the main checkout when `root` is a linked worktree whose main
/// checkout holds `.armadai/project.yaml`, else of `root`.
pub fn state_dir(root: &Path) -> PathBuf {
    let own = root.join(".armadai/state");
    match crate::worktree::main_checkout_of(root) {
        Some(main) if main.join(".armadai/project.yaml").is_file() => main.join(".armadai/state"),
        _ => own,
    }
}

/// A lot names a file: the gate's alphabet and length, and not a dot-file or `.`/`..`.
fn check_lot(lot: &str) -> anyhow::Result<()> {
    if is_valid_lot(lot) && lot.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return Ok(());
    }
    bail!(
        "lot {lot} cannot name a journal: 1 to 40 letters, digits, . or -, starting with a letter or a digit"
    )
}

/// `journal.jsonl` for None, `journal-<lot in lowercase>.jsonl` otherwise.
pub fn file_name(lot: Option<&str>) -> anyhow::Result<String> {
    Ok(match lot {
        None => "journal.jsonl".to_string(),
        Some(l) => {
            check_lot(l)?;
            format!("journal-{}.jsonl", l.to_ascii_lowercase())
        }
    })
}

fn is_link(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())
}

pub fn add(
    root: &Path,
    lot: Option<&str>,
    kind: Kind,
    by: Option<&str>,
    text: &str,
    now: u64,
) -> anyhow::Result<()> {
    use std::io::Write;
    let name = file_name(lot)?;
    if let Some(b) = by
        && !((1..=64).contains(&b.len())
            && b.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)))
    {
        bail!("--by {b}: 1 to 64 letters, digits, . _ or -");
    }
    if text.is_empty() {
        bail!("a journal entry needs a text");
    }
    if text.contains(['\n', '\r']) {
        bail!("a journal entry is one line: remove the line breaks");
    }
    let n = text.chars().count();
    if n > MAX_TEXT {
        bail!("the text is {n} characters; a journal entry holds at most {MAX_TEXT}");
    }
    let entry = Entry {
        schema: SCHEMA.into(),
        ts: now,
        kind: kind.as_str().into(),
        lot: lot.map(Into::into),
        by: by.map(Into::into),
        text: text.into(),
    };
    let line = format!("{}\n", serde_json::to_string(&entry)?);
    if line.len() > MAX_LINE {
        bail!(
            "this entry is {} bytes once written; a journal line holds at most {MAX_LINE} (shorten the text)",
            line.len()
        );
    }
    let dir = state_dir(root);
    let path = dir.join(name);
    for at in [&dir, &path] {
        if is_link(at) {
            bail!(
                "{} is a symbolic link; the journal is not written through it",
                at.display()
            );
        }
    }
    std::fs::create_dir_all(&dir)?;
    // One write of the whole line in append mode: two writers never mix their lines.
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .write_all(line.as_bytes())?;
    Ok(())
}

/// The file names to read, in no particular order.
fn files_to_read(dir: &Path, lot: Option<&str>) -> anyhow::Result<Vec<String>> {
    let mut v = vec![file_name(None)?];
    match lot {
        Some(l) => v.push(file_name(Some(l))?),
        None => {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().into_owned();
                    let valid = n
                        .strip_prefix("journal-")
                        .and_then(|s| s.strip_suffix(".jsonl"))
                        .is_some_and(|s| check_lot(s).is_ok() && s == s.to_ascii_lowercase());
                    if valid {
                        v.push(n);
                    }
                }
            }
        }
    }
    Ok(v)
}

pub fn show(
    root: &Path,
    lot: Option<&str>,
    kind: Option<&str>,
    last: usize,
) -> anyhow::Result<Vec<Entry>> {
    let dir = state_dir(root);
    let mut all: Vec<Entry> = Vec::new();
    for name in files_to_read(&dir, lot)? {
        let Ok(bytes) = std::fs::read(dir.join(&name)) else {
            continue;
        };
        let mut bad = 0;
        for raw in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            match serde_json::from_slice::<Entry>(raw) {
                // a lot's file holds that lot's entries to the exact case; `journal.jsonl` all
                Ok(e) if lot.is_none() || e.lot.is_none() || e.lot.as_deref() == lot => all.push(e),
                Ok(_) => {}
                Err(_) => bad += 1,
            }
        }
        if bad > 0 {
            eprintln!("{name}: {bad} unreadable lines skipped");
        }
    }
    // stable: equal `(ts, lot)` keep the order of their file
    all.sort_by(|a, b| (a.ts, &a.lot).cmp(&(b.ts, &b.lot)));
    all.retain(|e| kind.is_none_or(|k| e.kind == k));
    let skip = all.len().saturating_sub(last);
    Ok(all.split_off(skip))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join(".armadai")).unwrap();
        fs::write(t.path().join(".armadai/project.yaml"), "").unwrap();
        t
    }

    fn put(root: &Path, lot: Option<&str>, text: &str, now: u64) {
        add(root, lot, Kind::Note, None, text, now).unwrap();
    }

    fn entries(root: &Path) -> Vec<fs::DirEntry> {
        fs::read_dir(root.join(".armadai"))
            .unwrap()
            .flatten()
            .collect()
    }

    #[test]
    fn a_refused_lot_names_the_rule_and_writes_nothing() {
        let t = tempfile::tempdir().unwrap();
        let long = "a".repeat(41);
        for bad in [".", "..", ".x", "a_b", "a+b", long.as_str()] {
            let e = add(t.path(), Some(bad), Kind::Note, None, "x", 1).unwrap_err();
            assert_eq!(
                e.to_string(),
                format!(
                    "lot {bad} cannot name a journal: 1 to 40 letters, digits, . or -, starting with a letter or a digit"
                )
            );
        }
        assert!(!t.path().join(".armadai").exists(), "nothing was created");
    }

    #[test]
    fn a_lot_file_is_lowercase_and_show_keeps_the_exact_case() {
        let t = project();
        put(t.path(), Some("Menu-2.1"), "upper", 1);
        put(t.path(), Some("menu-2.1"), "lower", 2);
        // names are listed: `is_file` would pass on a case-insensitive file system
        let names: Vec<_> = fs::read_dir(t.path().join(".armadai/state"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["journal-menu-2.1.jsonl"]);
        let got = show(t.path(), Some("Menu-2.1"), None, 20).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, "upper");
    }

    #[test]
    fn lots_merge_by_ts_then_lot_and_the_lot_filter_keeps_the_other_out() {
        let t = project();
        put(t.path(), Some("b"), "b5", 5);
        put(t.path(), Some("a"), "a5", 5);
        put(t.path(), Some("a"), "a1", 1);
        put(t.path(), None, "all3", 3);
        let all: Vec<_> = show(t.path(), None, None, 20)
            .unwrap()
            .into_iter()
            .map(|e| e.text)
            .collect();
        assert_eq!(all, ["a1", "all3", "a5", "b5"]);
        let a: Vec<_> = show(t.path(), Some("a"), None, 20)
            .unwrap()
            .into_iter()
            .map(|e| e.text)
            .collect();
        assert_eq!(a, ["a1", "all3", "a5"]);
    }

    #[test]
    fn last_applies_after_the_merge_and_the_kind_filter() {
        let t = project();
        put(t.path(), Some("a"), "a1", 1);
        put(t.path(), Some("b"), "b2", 2);
        put(t.path(), Some("a"), "a3", 3);
        add(t.path(), Some("b"), Kind::Decision, None, "b4", 4).unwrap();
        let t2: Vec<_> = show(t.path(), None, None, 2)
            .unwrap()
            .into_iter()
            .map(|e| e.text)
            .collect();
        assert_eq!(t2, ["a3", "b4"]);
        let d = show(t.path(), None, Some("decision"), 20).unwrap();
        assert_eq!(d.len(), 1);
        // the filter runs before the cut: the last note is not the last entry
        let n: Vec<_> = show(t.path(), None, Some("note"), 1)
            .unwrap()
            .into_iter()
            .map(|e| e.text)
            .collect();
        assert_eq!(n, ["a3"]);
    }

    #[test]
    fn two_thousand_characters_pass_and_two_thousand_one_do_not() {
        let t = project();
        add(t.path(), None, Kind::Note, None, &"a".repeat(2000), 1).unwrap();
        let e = add(t.path(), None, Kind::Note, None, &"a".repeat(2001), 1).unwrap_err();
        assert_eq!(
            e.to_string(),
            "the text is 2001 characters; a journal entry holds at most 2000"
        );
    }

    #[test]
    fn a_line_over_4096_bytes_is_refused_with_its_size() {
        let t = project();
        let text = "€".repeat(2000);
        let e = add(t.path(), None, Kind::Note, None, &text, 1).unwrap_err();
        let msg = e.to_string();
        assert!(
            msg.starts_with("this entry is 6")
                && msg.ends_with(
                    " bytes once written; a journal line holds at most 4096 (shorten the text)"
                ),
            "{msg}"
        );
        assert!(!t.path().join(".armadai/state").exists());
    }

    #[test]
    fn line_breaks_empty_text_and_bad_by_are_refused() {
        let t = project();
        for (text, by, want) in [
            (
                "a\nb",
                None,
                "a journal entry is one line: remove the line breaks",
            ),
            (
                "a\rb",
                None,
                "a journal entry is one line: remove the line breaks",
            ),
            ("", None, "a journal entry needs a text"),
            (
                "x",
                Some("a b"),
                "--by a b: 1 to 64 letters, digits, . _ or -",
            ),
        ] {
            let e = add(t.path(), None, Kind::Note, by, text, 1).unwrap_err();
            assert_eq!(e.to_string(), want);
        }
        let long = "a".repeat(65);
        assert!(add(t.path(), None, Kind::Note, Some(&long), "x", 1).is_err());
        add(t.path(), None, Kind::Note, Some(&"a".repeat(64)), "x", 1).unwrap();
        assert_eq!(entries(t.path()).len(), 2); // project.yaml and state/
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_state_dir_or_file_is_not_written_through() {
        use std::os::unix::fs::symlink;
        let t = project();
        let out = tempfile::tempdir().unwrap();
        symlink(out.path(), t.path().join(".armadai/state")).unwrap();
        let e = add(t.path(), None, Kind::Note, None, "x", 1).unwrap_err();
        assert!(
            e.to_string()
                .ends_with(" is a symbolic link; the journal is not written through it")
        );
        assert_eq!(fs::read_dir(out.path()).unwrap().count(), 0);

        let t = project();
        fs::create_dir_all(t.path().join(".armadai/state")).unwrap();
        let target = out.path().join("t.jsonl");
        fs::write(&target, "").unwrap();
        symlink(&target, t.path().join(".armadai/state/journal.jsonl")).unwrap();
        assert!(add(t.path(), None, Kind::Note, None, "x", 1).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "");
    }

    #[test]
    fn unreadable_lines_are_skipped_and_unknown_kinds_shown() {
        let t = project();
        let d = t.path().join(".armadai/state");
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("journal.jsonl"),
            "not json\n{\"schema\":\"armadai.journal/1\",\"ts\":1,\"kind\":\"future\",\"text\":\"t\"}\n{}\n",
        )
        .unwrap();
        let got = show(t.path(), None, None, 20).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "future");
    }

    #[test]
    fn other_files_of_state_are_not_journals() {
        let t = project();
        let d = t.path().join(".armadai/state");
        fs::create_dir_all(&d).unwrap();
        let line = "{\"schema\":\"armadai.journal/1\",\"ts\":1,\"kind\":\"note\",\"text\":\"t\"}\n";
        for n in [
            "journal-Menu.jsonl",
            "journal-a_b.jsonl",
            "journal-.x.jsonl",
            "ledger-a.md",
            "journal-a.jsonl.bak",
        ] {
            fs::write(d.join(n), line).unwrap();
        }
        assert!(show(t.path(), None, None, 20).unwrap().is_empty());
    }

    #[test]
    fn state_dir_follows_the_main_checkout_only_when_it_has_a_project() {
        let t = tempfile::tempdir().unwrap();
        let main = t.path().join("main");
        let wt = t.path().join("wt");
        fs::create_dir_all(main.join(".git/worktrees/wt")).unwrap();
        fs::create_dir_all(&wt).unwrap();
        fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", main.join(".git/worktrees/wt").display()),
        )
        .unwrap();
        // no project in the main checkout: the worktree's own
        assert_eq!(state_dir(&wt), wt.join(".armadai/state"));
        fs::create_dir_all(main.join(".armadai")).unwrap();
        fs::write(main.join(".armadai/project.yaml"), "").unwrap();
        assert_eq!(state_dir(&wt), main.join(".armadai/state"));
        // the main checkout itself, and outside git
        assert_eq!(state_dir(&main), main.join(".armadai/state"));
        let out = tempfile::tempdir().unwrap();
        assert_eq!(state_dir(out.path()), out.path().join(".armadai/state"));
    }
}
