//! Files link shares with the user (`.claude/settings.json`, the root `CLAUDE.md`):
//! patched in place, never written whole by `linker::manifest::write_files` (which
//! skips an existing file without `--force` and deletes it whole on `unlink`).
//!
//! Every write is atomic (temporary file + rename) and refuses a symbolic link or an
//! unreadable file before touching anything. What was added is returned as a
//! [`PatchedFile`], recorded in the link manifest, and is all `unlink` removes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::claude_settings::{HookEntry, Owned, merge_settings, remove_entries};
use super::{BLOCK_BEGIN, BLOCK_END};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchedFile {
    /// Relative to the project root.
    pub path: PathBuf,
    pub kind: PatchKind,
    /// The file did not exist before link: removed once nothing else is left in it.
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PatchKind {
    SettingsHooks(Owned),
    MarkdownBlock,
}

/// Current text of `path`, `None` when absent; a symbolic link is refused.
fn read(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        Ok(m) if m.file_type().is_symlink() => {
            bail!(
                "{} is a symbolic link; refusing to patch it",
                path.display()
            )
        }
        Ok(_) => std::fs::read_to_string(path)
            .map(Some)
            .with_context(|| format!("cannot read {}", path.display())),
    }
}

/// Temporary file next to `path`, then rename: a reader never sees half a file.
fn write_atomic(path: &Path, text: &str) -> anyhow::Result<()> {
    let dir = path.parent().context("patched file has no parent")?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().context("patched file has no name")?;
    let tmp = dir.join(format!(".{}.armadai-tmp", name.to_string_lossy()));
    let result = (|| {
        std::fs::write(&tmp, text)?;
        if let Ok(m) = std::fs::metadata(path) {
            std::fs::set_permissions(&tmp, m.permissions())?;
        }
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.with_context(|| format!("cannot write {}", path.display()))
}

/// Everything [`apply_settings`] would refuse (symbolic link, unreadable or invalid
/// JSON, a native key clash), checked without writing: run before any write.
pub fn preflight_settings(
    root: &Path,
    rel: &Path,
    entries: &[HookEntry],
    keys: &BTreeMap<String, Value>,
    previous: Option<&PatchedFile>,
) -> anyhow::Result<()> {
    let path = root.join(rel);
    let prev = match previous.map(|p| &p.kind) {
        Some(PatchKind::SettingsHooks(o)) => o.clone(),
        _ => Owned::default(),
    };
    merge_settings(read(&path)?.as_deref(), entries, keys, &prev)
        .map(|_| ())
        .with_context(|| path.display().to_string())
}

/// Everything [`apply_markdown_block`] would refuse, checked without writing.
pub fn preflight_markdown(root: &Path, rel: &Path) -> anyhow::Result<()> {
    let path = root.join(rel);
    if let Some(t) = read(&path)? {
        block_span(&t, &path)?;
    }
    Ok(())
}

/// Merge hook `entries` and native `keys` into `root/rel` (a settings.json).
pub fn apply_settings(
    root: &Path,
    rel: &Path,
    entries: &[HookEntry],
    keys: &BTreeMap<String, Value>,
    previous: Option<&PatchedFile>,
) -> anyhow::Result<PatchedFile> {
    let path = root.join(rel);
    let existing = read(&path)?;
    let prev_owned = match previous.map(|p| &p.kind) {
        Some(PatchKind::SettingsHooks(o)) => o.clone(),
        _ => Owned::default(),
    };
    let created = previous.is_some_and(|p| p.created) || existing.is_none();
    let (text, owned) = merge_settings(existing.as_deref(), entries, keys, &prev_owned)
        .with_context(|| path.display().to_string())?;
    let nothing_to_create = existing.is_none() && owned == Owned::default();
    if !nothing_to_create && existing.as_deref() != Some(text.as_str()) {
        write_atomic(&path, &text)?;
    }
    Ok(PatchedFile {
        path: rel.to_path_buf(),
        kind: PatchKind::SettingsHooks(owned),
        created,
    })
}

fn marked(block: &str) -> String {
    let nl = if block.ends_with('\n') { "" } else { "\n" };
    format!("{BLOCK_BEGIN}\n{block}{nl}{BLOCK_END}\n")
}

/// Byte span of the ArmadAI block (end includes the newline after the end marker).
fn block_span(text: &str, path: &Path) -> anyhow::Result<Option<(usize, usize)>> {
    let Some(b) = text.find(BLOCK_BEGIN) else {
        return Ok(None);
    };
    let Some(e) = text[b..].find(BLOCK_END).map(|i| b + i + BLOCK_END.len()) else {
        bail!("{}: `{BLOCK_BEGIN}` without `{BLOCK_END}`", path.display());
    };
    let e = if text[e..].starts_with('\n') {
        e + 1
    } else {
        e
    };
    Ok(Some((b, e)))
}

/// Put `block` between the ArmadAI markers of `root/rel`: replaced in place when the
/// markers exist, appended after the user's content otherwise.
pub fn apply_markdown_block(
    root: &Path,
    rel: &Path,
    block: &str,
    previous: Option<&PatchedFile>,
) -> anyhow::Result<PatchedFile> {
    let path = root.join(rel);
    let existing = read(&path)?;
    let created = previous.is_some_and(|p| p.created) || existing.is_none();
    let text = match &existing {
        None => marked(block),
        Some(t) => match block_span(t, &path)? {
            Some((b, e)) => format!("{}{}{}", &t[..b], marked(block), &t[e..]),
            None if t.is_empty() => marked(block),
            None => {
                let nl = if t.ends_with('\n') { "" } else { "\n" };
                format!("{t}{nl}\n{}", marked(block))
            }
        },
    };
    if existing.as_deref() != Some(text.as_str()) {
        write_atomic(&path, &text)?;
    }
    Ok(PatchedFile {
        path: rel.to_path_buf(),
        kind: PatchKind::MarkdownBlock,
        created,
    })
}

/// Undo `patched` under `root`: remove what link added, and the file itself when link
/// created it and nothing else is left. A missing file is nothing to undo.
pub fn remove_patch(root: &Path, patched: &PatchedFile) -> anyhow::Result<()> {
    let path = root.join(&patched.path);
    let Some(text) = read(&path)? else {
        return Ok(());
    };
    let (rest, empty) = match &patched.kind {
        PatchKind::SettingsHooks(owned) => {
            let rest = remove_entries(&text, owned).with_context(|| path.display().to_string())?;
            let empty = serde_json::from_str::<Value>(&rest)
                .is_ok_and(|v| v.as_object().is_some_and(|m| m.is_empty()));
            (rest, empty)
        }
        PatchKind::MarkdownBlock => {
            let Some((b, e)) = block_span(&text, &path)? else {
                return Ok(());
            };
            let mut rest = format!("{}{}", &text[..b], &text[e..]);
            // the blank line `apply_markdown_block` put before an appended block
            if e == text.len() && rest.ends_with("\n\n") {
                rest.pop();
            }
            let empty = rest.trim().is_empty();
            (rest, empty)
        }
    };
    if patched.created && empty {
        std::fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))
    } else if rest != text {
        write_atomic(&path, &rest)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SETTINGS: &str = ".claude/settings.json";
    const USER: &str = "{\n  \"permissions\": {\n    \"allow\": [\"Bash(ls)\"]\n  }\n}\n";

    fn entries() -> Vec<HookEntry> {
        vec![HookEntry {
            event: "SessionStart".into(),
            matcher: None,
            command: "echo hi".into(),
            native: BTreeMap::new(),
        }]
    }

    fn value(p: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("armadai-tmp"))
            .collect()
    }

    #[test]
    fn a_created_settings_file_is_removed_again() {
        let dir = tempfile::tempdir().unwrap();
        let p = apply_settings(
            dir.path(),
            Path::new(SETTINGS),
            &entries(),
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        assert!(p.created);
        assert_eq!(p.path, Path::new(SETTINGS));
        let file = dir.path().join(SETTINGS);
        assert!(value(&file)["hooks"]["SessionStart"].is_array());
        assert!(leftovers(&dir.path().join(".claude")).is_empty());
        remove_patch(dir.path(), &p).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn a_user_file_left_empty_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(SETTINGS);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{}\n").unwrap();
        let p = apply_settings(
            dir.path(),
            Path::new(SETTINGS),
            &entries(),
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        remove_patch(dir.path(), &p).unwrap();
        assert_eq!(value(&file), json!({}));
    }

    #[test]
    fn nothing_to_add_creates_no_file() {
        let dir = tempfile::tempdir().unwrap();
        apply_settings(dir.path(), Path::new(SETTINGS), &[], &BTreeMap::new(), None).unwrap();
        assert!(!dir.path().join(SETTINGS).exists());
    }

    #[test]
    fn a_user_settings_file_is_kept_and_restored() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(SETTINGS);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, USER).unwrap();
        let p = apply_settings(
            dir.path(),
            Path::new(SETTINGS),
            &entries(),
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        assert!(!p.created);
        assert_eq!(value(&file)["permissions"], json!({"allow": ["Bash(ls)"]}));
        // relinking with the recorded patch changes nothing
        let again = apply_settings(
            dir.path(),
            Path::new(SETTINGS),
            &entries(),
            &BTreeMap::new(),
            Some(&p),
        )
        .unwrap();
        assert_eq!(again, p);
        remove_patch(dir.path(), &p).unwrap();
        assert_eq!(value(&file), serde_json::from_str::<Value>(USER).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_settings_file_is_refused_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("elsewhere.json");
        std::fs::write(&real, USER).unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::os::unix::fs::symlink(&real, dir.path().join(SETTINGS)).unwrap();
        let err = apply_settings(
            dir.path(),
            Path::new(SETTINGS),
            &entries(),
            &BTreeMap::new(),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("symbolic link"), "{err}");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), USER);
    }

    #[test]
    fn invalid_json_is_refused_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(SETTINGS);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{ broken").unwrap();
        assert!(
            apply_settings(
                dir.path(),
                Path::new(SETTINGS),
                &entries(),
                &BTreeMap::new(),
                None
            )
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ broken");
    }

    #[test]
    fn a_markdown_block_is_appended_replaced_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("CLAUDE.md");
        std::fs::write(&file, "# Mine\n").unwrap();
        let p = apply_markdown_block(dir.path(), Path::new("CLAUDE.md"), "Lead.\n", None).unwrap();
        assert!(!p.created);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            format!("# Mine\n\n{BLOCK_BEGIN}\nLead.\n{BLOCK_END}\n")
        );
        apply_markdown_block(dir.path(), Path::new("CLAUDE.md"), "Lead v2.", Some(&p)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            format!("# Mine\n\n{BLOCK_BEGIN}\nLead v2.\n{BLOCK_END}\n"),
            "replaced in place, never duplicated"
        );
        remove_patch(dir.path(), &p).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "# Mine\n");
    }

    #[test]
    fn a_block_in_the_middle_is_replaced_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("CLAUDE.md");
        std::fs::write(
            &file,
            format!("top\n{BLOCK_BEGIN}\nold\n{BLOCK_END}\nbottom\n"),
        )
        .unwrap();
        let p = apply_markdown_block(dir.path(), Path::new("CLAUDE.md"), "new\n", None).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            format!("top\n{BLOCK_BEGIN}\nnew\n{BLOCK_END}\nbottom\n")
        );
        remove_patch(dir.path(), &p).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "top\nbottom\n");
    }

    #[test]
    fn a_created_markdown_file_is_removed_again() {
        let dir = tempfile::tempdir().unwrap();
        let p = apply_markdown_block(dir.path(), Path::new("CLAUDE.md"), "Lead.\n", None).unwrap();
        assert!(p.created);
        remove_patch(dir.path(), &p).unwrap();
        assert!(!dir.path().join("CLAUDE.md").exists());
    }

    #[test]
    fn an_unterminated_block_is_refused_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("CLAUDE.md");
        let text = format!("x\n{BLOCK_BEGIN}\ny\n");
        std::fs::write(&file, &text).unwrap();
        assert!(apply_markdown_block(dir.path(), Path::new("CLAUDE.md"), "b", None).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    }
}
