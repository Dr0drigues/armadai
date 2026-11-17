//! The lot journal: JSONL files under the main checkout's `.armadai/state/` or in the folder
//! `sessions.journal_dir` names, one per lot (`journal-<lot in lowercase>.jsonl`) plus
//! `journal.jsonl` for entries that hold for every lot. Written by `armadai journal add` in one
//! append, read by `armadai journal show`.

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

/// `.armadai/state/` of the project of `root` in the main checkout, when `root` is in a
/// linked worktree and that project (the same path relative to the worktree) holds
/// `.armadai/project.yaml`, else of `root`.
pub fn state_dir(root: &Path) -> PathBuf {
    let own = root.join(".armadai/state");
    let Some(main) = crate::worktree::main_checkout_of(root) else {
        return own;
    };
    let Some(top) = root.ancestors().find(|d| d.join(".git").exists()) else {
        return own;
    };
    let project = main.join(root.strip_prefix(top).unwrap_or(Path::new("")));
    if project.join(".armadai/project.yaml").is_file() {
        project.join(".armadai/state")
    } else {
        own
    }
}

/// The folder holding the journal files: named by `sessions.journal_dir` (it must already
/// exist) or the default one (created on the first write).
enum JournalDir {
    Default(PathBuf),
    Named(PathBuf),
    /// The key holds a value that `invalid-journal-dir` refuses.
    Invalid,
}

/// `sessions.journal_dir`, read from the `.armadai/project.yaml` of `root` alone (not the
/// effective view: `journal` must keep working on a project that does not validate, and a key
/// set by a pack is not read). A relative value starts from the parent of the resolved
/// `.armadai` (links followed), so worktrees sharing one `.armadai` agree on the folder.
/// A file that does not read, or a `sessions` of another shape, means the default folder; a
/// `journal_dir` that `validate` refuses (bad text, a list, a map) is `Invalid`.
fn journal_dir(root: &Path) -> JournalDir {
    /// Only the key: read as the model reads it (a number is its text), whatever else is wrong.
    #[derive(Deserialize)]
    struct Key {
        sessions: Option<Sessions>,
    }
    #[derive(Deserialize)]
    struct Sessions {
        journal_dir: Option<String>,
    }
    let named = (|| {
        let text = std::fs::read_to_string(root.join(".armadai/project.yaml")).ok()?;
        let dir = match armadai_model::yaml::from_str::<Key>(&text) {
            Ok(k) => k.sessions?.journal_dir?,
            // a value `validate` refuses (a list, a map): refused here too, not ignored
            Err(_) => {
                let v: serde_json::Value = armadai_model::yaml::from_str(&text).ok()?;
                v.pointer("/sessions/journal_dir")?;
                return Some(JournalDir::Invalid);
            }
        };
        if !armadai_model::is_valid_journal_dir(&dir) {
            return Some(JournalDir::Invalid);
        }
        // a `canonicalize` that fails means no readable project: the default folder
        let real = std::fs::canonicalize(root.join(".armadai")).ok()?;
        Some(JournalDir::Named(real.parent()?.join(dir)))
    })();
    named.unwrap_or_else(|| JournalDir::Default(state_dir(root)))
}

/// The folder to use; a named one that is not a folder is refused with what to do next.
fn resolve_dir(root: &Path) -> anyhow::Result<PathBuf> {
    match journal_dir(root) {
        JournalDir::Default(d) => Ok(d),
        JournalDir::Invalid => bail!(
            "{}; fix it in .armadai/project.yaml",
            armadai_model::JOURNAL_DIR_RULE
        ),
        JournalDir::Named(d) => {
            // a link is refused by the callers' own check, with its own message
            if !is_link(&d) {
                let (what, fix) = match std::fs::metadata(&d) {
                    Err(_) => ("does not exist", "create it, or fix the path"),
                    Ok(m) if !m.is_dir() => ("is not a folder", "fix the path"),
                    Ok(_) => return Ok(d),
                };
                bail!(
                    "sessions.journal_dir leads to {}, which {what}; {fix} in .armadai/project.yaml",
                    d.display()
                );
            }
            Ok(d)
        }
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
    add_with(root, lot, kind, by, text, now, |_| {})
}

/// `add`, calling `between(path)` twice under the lock (a test looks at the lock there): right
/// after it is taken, then after the last-byte check and before the write.
fn add_with(
    root: &Path,
    lot: Option<&str>,
    kind: Kind,
    by: Option<&str>,
    text: &str,
    now: u64,
    mut between: impl FnMut(&Path),
) -> anyhow::Result<()> {
    use std::io::{Read, Seek, SeekFrom, Write};
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
    let dir = resolve_dir(root)?;
    let path = dir.join(name);
    for at in [&dir, &path] {
        if is_link(at) {
            bail!(
                "{} is a symbolic link; the journal is not written through it",
                at.display()
            );
        }
    }
    // a named folder exists by now (`resolve_dir`), so this only creates the default one
    std::fs::create_dir_all(&dir)?;
    // One write of the whole line in append mode: two writers never mix their lines.
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    // The check below and the write must not interleave with another writer's: a writer's
    // line can be seen half written (Linux updates the size page by page), which would
    // read as a torn line and add a blank one.
    crate::file_lock::lock_exclusive(&f)?;
    between(&path);
    let mut out = Vec::with_capacity(line.len() + 1);
    // a torn last line (no `\n`) must not swallow this entry
    if f.metadata()?.len() > 0 {
        let mut last = [0u8];
        f.seek(SeekFrom::End(-1))?;
        f.read_exact(&mut last)?;
        if last[0] != b'\n' {
            out.push(b'\n');
        }
    }
    out.extend_from_slice(line.as_bytes());
    between(&path);
    f.write_all(&out)?;
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

/// The file's bytes, `None` when absent; a symbolic link (the folder or the file) is refused.
fn read_unlinked(dir: &Path, name: &str) -> anyhow::Result<Option<Vec<u8>>> {
    read_unlinked_with(dir, name, |_| {})
}

/// `read_unlinked`, calling `during(path)` while the shared lock is held, before the read.
fn read_unlinked_with(
    dir: &Path,
    name: &str,
    during: impl FnOnce(&Path),
) -> anyhow::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let path = dir.join(name);
    for at in [dir, path.as_path()] {
        if is_link(at) {
            bail!(
                "{} is a symbolic link; the journal is not read through it",
                at.display()
            );
        }
    }
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // closes the gap between the check above and the open
        o.custom_flags(libc::O_NOFOLLOW);
    }
    match o.open(&path) {
        Ok(mut f) => {
            // wait for a writer in the middle of a line
            crate::file_lock::lock_shared(&f)?;
            during(&path);
            let mut v = Vec::new();
            f.read_to_end(&mut v)?;
            Ok(Some(v))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

pub fn show(
    root: &Path,
    lot: Option<&str>,
    kind: Option<&str>,
    last: usize,
) -> anyhow::Result<Vec<Entry>> {
    let dir = resolve_dir(root)?;
    let mut all: Vec<Entry> = Vec::new();
    for name in files_to_read(&dir, lot)? {
        let Some(bytes) = read_unlinked(&dir, &name)? else {
            continue;
        };
        let mut bad = 0;
        for raw in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            match serde_json::from_slice::<Entry>(raw) {
                // a lot's file holds that lot's entries to the exact case; `journal.jsonl` all
                // a line of another version is skipped and counted, not read as this one
                Ok(e) if e.schema != SCHEMA => bad += 1,
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

    /// A project whose `project.yaml` names `journal_dir`; returns it and its real path.
    fn named(dir: &str) -> (tempfile::TempDir, PathBuf) {
        let t = project();
        fs::write(
            t.path().join(".armadai/project.yaml"),
            format!("sessions:\n  journal_dir: {dir}\n"),
        )
        .unwrap();
        let real = fs::canonicalize(t.path()).unwrap();
        (t, real)
    }

    #[test]
    fn a_named_folder_receives_the_journal_and_state_is_not_created() {
        let (_t, real) = named("j");
        let j = real.join("j");
        fs::create_dir_all(&j).unwrap();
        put(&real, Some("a"), "x", 1);
        assert!(j.join("journal-a.jsonl").is_file());
        assert!(!real.join(".armadai/state").exists());
        assert_eq!(show(&real, Some("a"), None, 20).unwrap().len(), 1);
    }

    #[test]
    fn a_missing_named_folder_is_refused_and_not_created() {
        let (_t, real) = named("lot-journal");
        let want = format!(
            "sessions.journal_dir leads to {}, which does not exist; create it, or fix the path in .armadai/project.yaml",
            real.join("lot-journal").display()
        );
        let e = add(&real, None, Kind::Note, None, "x", 1).unwrap_err();
        assert_eq!(e.to_string(), want);
        let e = show(&real, None, None, 20).unwrap_err();
        assert_eq!(e.to_string(), want);
        assert!(!real.join("lot-journal").exists());
    }

    #[test]
    fn a_named_path_that_is_a_file_is_refused() {
        let (_t, real) = named("f");
        fs::write(real.join("f"), "").unwrap();
        let want = format!(
            "sessions.journal_dir leads to {}, which is not a folder; fix the path in .armadai/project.yaml",
            real.join("f").display()
        );
        let e = add(&real, None, Kind::Note, None, "x", 1).unwrap_err();
        assert_eq!(e.to_string(), want);
        assert_eq!(show(&real, None, None, 20).unwrap_err().to_string(), want);
    }

    #[test]
    fn a_named_folder_that_is_a_link_is_not_used_but_a_linked_armadai_is() {
        use std::os::unix::fs::symlink;
        let (_t, real) = named("j");
        let out = tempfile::tempdir().unwrap();
        symlink(out.path(), real.join("j")).unwrap();
        let e = add(&real, None, Kind::Note, None, "x", 1).unwrap_err();
        assert_eq!(
            e.to_string(),
            format!(
                "{} is a symbolic link; the journal is not written through it",
                real.join("j").display()
            )
        );
        let e = show(&real, None, None, 20).unwrap_err();
        assert_eq!(
            e.to_string(),
            format!(
                "{} is a symbolic link; the journal is not read through it",
                real.join("j").display()
            )
        );
        assert_eq!(fs::read_dir(out.path()).unwrap().count(), 0);

        // `.armadai` itself a link: allowed, the folder is beside the real one
        let w = tempfile::tempdir().unwrap();
        let w = fs::canonicalize(w.path()).unwrap();
        fs::create_dir_all(w.join("wt")).unwrap();
        fs::create_dir_all(w.join("c/j")).unwrap();
        fs::create_dir_all(w.join("c/.armadai")).unwrap();
        fs::write(
            w.join("c/.armadai/project.yaml"),
            "sessions:\n  journal_dir: j\n",
        )
        .unwrap();
        symlink(w.join("c/.armadai"), w.join("wt/.armadai")).unwrap();
        put(&w.join("wt"), None, "x", 1);
        assert!(w.join("c/j/journal.jsonl").is_file());
    }

    #[test]
    fn a_dotdot_after_a_linked_component_is_left_to_the_file_system() {
        use std::os::unix::fs::symlink;
        let (_t, real) = named("lnk/../j");
        let else_ = tempfile::tempdir().unwrap();
        let else_ = fs::canonicalize(else_.path()).unwrap();
        fs::create_dir_all(else_.join("sub")).unwrap();
        fs::create_dir_all(else_.join("j")).unwrap();
        symlink(else_.join("sub"), real.join("lnk")).unwrap();
        put(&real, None, "x", 1);
        // `lnk/..` is `else_`, not the folder holding `lnk`
        assert!(else_.join("j/journal.jsonl").is_file());
        assert!(!real.join("j").exists());
    }

    #[test]
    fn a_value_the_validator_refuses_names_no_folder_and_writes_nothing() {
        for bad in ["\"\"", "\" j\"", "\"j \"", "\"a\\nb\""] {
            let (_t, real) = named(bad);
            let want = format!(
                "{}; fix it in .armadai/project.yaml",
                armadai_model::JOURNAL_DIR_RULE
            );
            let e = add(&real, None, Kind::Note, None, "x", 1).unwrap_err();
            assert_eq!(e.to_string(), want, "{bad}");
            assert_eq!(show(&real, None, None, 20).unwrap_err().to_string(), want);
            let names: Vec<_> = fs::read_dir(&real).unwrap().flatten().collect();
            assert_eq!(names.len(), 1, "{bad}: only .armadai");
            assert!(!real.join(".armadai/state").exists());
        }
    }

    #[test]
    fn journal_dir_is_read_as_validate_reads_it() {
        // a number is its text: the folder `5`
        let (_t, real) = named("5");
        fs::create_dir_all(real.join("5")).unwrap();
        put(&real, None, "x", 1);
        assert!(real.join("5/journal.jsonl").is_file());
        assert!(!real.join(".armadai/state").exists());
        // a list or a map is refused, not ignored
        for bad in ["[a]", "{a: b}"] {
            let (_t, real) = named(bad);
            let e = add(&real, None, Kind::Note, None, "x", 1).unwrap_err();
            assert_eq!(
                e.to_string(),
                format!(
                    "{}; fix it in .armadai/project.yaml",
                    armadai_model::JOURNAL_DIR_RULE
                ),
                "{bad}"
            );
            assert!(show(&real, None, None, 20).is_err());
            assert!(!real.join(".armadai/state").exists(), "{bad}");
        }
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

    #[test]
    fn a_project_in_a_subfolder_keeps_its_own_journal_in_the_main_checkout() {
        let t = tempfile::tempdir().unwrap();
        let main = t.path().join("main");
        let wt = t.path().join("wt");
        fs::create_dir_all(main.join(".git/worktrees/wt")).unwrap();
        fs::create_dir_all(wt.join("sub")).unwrap();
        fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", main.join(".git/worktrees/wt").display()),
        )
        .unwrap();
        // the main checkout has a project at its root, none in sub/: sub keeps its own
        fs::create_dir_all(main.join(".armadai")).unwrap();
        fs::write(main.join(".armadai/project.yaml"), "").unwrap();
        assert_eq!(state_dir(&wt.join("sub")), wt.join("sub/.armadai/state"));
        fs::create_dir_all(main.join("sub/.armadai")).unwrap();
        fs::write(main.join("sub/.armadai/project.yaml"), "").unwrap();
        assert_eq!(state_dir(&wt.join("sub")), main.join("sub/.armadai/state"));
        assert_eq!(state_dir(&wt), main.join(".armadai/state"));
        // a main checkout without a root project but with one in sub/
        fs::remove_file(main.join(".armadai/project.yaml")).unwrap();
        assert_eq!(state_dir(&wt), wt.join(".armadai/state"));
    }

    #[test]
    fn an_entry_after_a_torn_line_is_not_lost_and_a_clean_file_gets_no_blank_line() {
        let t = project();
        let d = t.path().join(".armadai/state");
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("journal.jsonl"),
            "{\"schema\":\"armadai.journal/1\",\"ts\":1,\"kind\":\"note\",\"text\":\"half",
        )
        .unwrap();
        put(t.path(), None, "whole", 2);
        let got = show(t.path(), None, None, 20).unwrap();
        assert_eq!(
            got.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
            ["whole"]
        );
        put(t.path(), None, "next", 3);
        let body = fs::read_to_string(d.join("journal.jsonl")).unwrap();
        assert_eq!(body.matches("\n\n").count(), 0);
        assert_eq!(body.lines().count(), 3);
    }

    #[test]
    fn a_line_of_another_schema_version_is_skipped() {
        let t = project();
        let d = t.path().join(".armadai/state");
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("journal.jsonl"),
            "{\"schema\":\"armadai.journal/2\",\"ts\":1,\"kind\":\"note\",\"text\":\"future\"}\n",
        )
        .unwrap();
        put(t.path(), None, "now", 2);
        let got = show(t.path(), None, None, 20).unwrap();
        assert_eq!(
            got.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
            ["now"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn show_refuses_to_read_through_a_link() {
        use std::os::unix::fs::symlink;
        let t = project();
        let out = tempfile::tempdir().unwrap();
        let d = t.path().join(".armadai/state");
        fs::create_dir_all(&d).unwrap();
        fs::write(out.path().join("x.jsonl"), "").unwrap();
        symlink(out.path().join("x.jsonl"), d.join("journal.jsonl")).unwrap();
        let e = show(t.path(), None, None, 20).unwrap_err();
        assert!(
            e.to_string()
                .ends_with(" is a symbolic link; the journal is not read through it"),
            "{e}"
        );
        let t = project();
        symlink(out.path(), t.path().join(".armadai/state")).unwrap();
        assert!(show(t.path(), None, None, 20).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_locked_from_its_opening_to_the_write() {
        let t = project();
        let mut seen = vec![];
        add_with(t.path(), None, Kind::Note, None, "x", 1, |path| {
            let other = fs::OpenOptions::new().append(true).open(path).unwrap();
            seen.push(crate::file_lock::is_locked_elsewhere(&other));
        })
        .unwrap();
        // right after the lock, and again just before the write
        assert_eq!(seen, [true, true]);
    }

    #[cfg(unix)]
    #[test]
    fn a_read_holds_a_shared_lock_not_an_exclusive_one() {
        let t = project();
        put(t.path(), None, "x", 1);
        let dir = t.path().join(".armadai/state");
        let mut seen = None;
        let got = read_unlinked_with(&dir, "journal.jsonl", |path| {
            let open = || fs::OpenOptions::new().read(true).open(path).unwrap();
            // no exclusive lock can be taken (checked first: a lock taken here would
            // hide the next answer), but a shared one can: the lock is shared
            let (a, b) = (open(), open());
            seen = Some((
                crate::file_lock::is_locked_elsewhere(&a),
                crate::file_lock::can_share(&b),
            ));
        })
        .unwrap();
        assert!(got.is_some());
        assert_eq!(seen, Some((true, true)));
    }
}
