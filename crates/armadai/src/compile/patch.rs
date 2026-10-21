//! Files link shares with the user (`.claude/settings.json`, the root `CLAUDE.md`,
//! `opencode.json`):
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
use super::json_keys::{JsonKeys, OwnedJson, merge_json_keys, remove_json_keys};
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
    /// Owned keys of a shared JSON config file (`opencode.json`).
    JsonKeys(OwnedJson),
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

/// `root/rel`, refused unless it stays under `root/trust` (lexically, and through
/// any symbolic link on the way): a manifest is data on disk. `trust` is the project
/// root (`.`) for the root CLAUDE.md, the target's root for its settings.json, which
/// may legitimately sit outside the project (`--output ../sibling`).
fn checked(root: &Path, trust: &Path, rel: &Path) -> anyhow::Result<PathBuf> {
    use crate::linker::manifest::lexically_normalize;
    let path = root.join(rel);
    let base = lexically_normalize(&root.join(trust));
    let real_base = real(&base)?;
    // a trust root inside the project must not be a link leading out of it
    if base.starts_with(lexically_normalize(root)) && !real_base.starts_with(real(root)?) {
        bail!("{} resolves outside the project", trust.display());
    }
    // the file itself may be a symlink: `read` refuses it with a clearer message
    let parent = path.parent().context("patched file has no parent")?;
    if !real(parent)?.starts_with(&real_base) {
        bail!(
            "{} resolves outside where link patches files",
            rel.display()
        );
    }
    Ok(path)
}

/// `p` with its deepest existing ancestor resolved (symbolic links followed) and
/// the not-yet-existing rest appended.
fn real(p: &Path) -> anyhow::Result<PathBuf> {
    let mut rest = Vec::new();
    let mut cur = p;
    loop {
        if cur.exists() {
            let mut out = cur.canonicalize()?;
            out.extend(rest.iter().rev());
            return Ok(out);
        }
        rest.push(cur.file_name().context("path without a name")?);
        cur = cur.parent().context("path without an existing ancestor")?;
    }
}

/// A fresh, exclusively created temporary file next to `path` (random name, never
/// a planted one), then rename: a reader never sees half a file. The original's
/// permissions are kept.
pub(crate) fn write_atomic(path: &Path, text: &str) -> anyhow::Result<()> {
    use std::io::Write as _;
    let dir = path.parent().context("patched file has no parent")?;
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("cannot write next to {}", path.display()))?;
    tmp.write_all(text.as_bytes())?;
    match std::fs::metadata(path) {
        Ok(m) => {
            // the rename replaces the file: keep its owner and group when allowed, and
            // write anyway when not (a group the user is not in): never refuse midway
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let _ = std::os::unix::fs::chown(tmp.path(), Some(m.uid()), Some(m.gid()));
            }
            std::fs::set_permissions(tmp.path(), m.permissions())?
        }
        // a new file: the usual mode of a versioned text file, not the temp file's 0600
        #[cfg(unix)]
        Err(_) => {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o644))?
        }
        #[cfg(not(unix))]
        Err(_) => {}
    }
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

/// Everything [`apply_settings`] would refuse (symbolic link, unreadable or invalid
/// JSON, a native key clash), checked without writing: run before any write.
pub fn preflight_settings(
    root: &Path,
    rel: &Path,
    trust: &Path,
    entries: &[HookEntry],
    keys: &BTreeMap<String, Value>,
    previous: Option<&PatchedFile>,
) -> anyhow::Result<()> {
    let path = checked(root, trust, rel)?;
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
    let path = checked(root, Path::new("."), rel)?;
    if let Some(t) = read(&path)? {
        block_span(&t, &path)?;
    }
    Ok(())
}

/// Merge hook `entries` and native `keys` into `root/rel` (a settings.json). Also
/// says whether an existing file was reformatted: values and key order are kept,
/// the JSON layout is normalised. A file whose value does not change is not written.
pub fn apply_settings(
    root: &Path,
    rel: &Path,
    trust: &Path,
    entries: &[HookEntry],
    keys: &BTreeMap<String, Value>,
    previous: Option<&PatchedFile>,
) -> anyhow::Result<(PatchedFile, bool)> {
    let path = checked(root, trust, rel)?;
    let existing = read(&path)?;
    let prev_owned = match previous.map(|p| &p.kind) {
        Some(PatchKind::SettingsHooks(o)) => o.clone(),
        _ => Owned::default(),
    };
    let created = previous.is_some_and(|p| p.created) || existing.is_none();
    let (text, owned) = merge_settings(existing.as_deref(), entries, keys, &prev_owned)
        .with_context(|| path.display().to_string())?;
    let reformatted = write_json(&path, existing.as_deref(), &text, owned == Owned::default())?;
    let patched = PatchedFile {
        path: rel.to_path_buf(),
        kind: PatchKind::SettingsHooks(owned),
        created,
    };
    Ok((patched, reformatted))
}

/// Write merged JSON `text` over `existing` unless its value is unchanged (or, for a
/// new file, unless link owns nothing in it); says whether an existing file's layout
/// was normalised.
fn write_json(
    path: &Path,
    existing: Option<&str>,
    text: &str,
    owns_nothing: bool,
) -> anyhow::Result<bool> {
    match existing {
        None if owns_nothing => Ok(false),
        None => write_atomic(path, text).map(|_| false),
        Some(old) => {
            let old_value: Value = serde_json::from_str(old)?;
            if old_value == serde_json::from_str::<Value>(text)? {
                return Ok(false);
            }
            let canonical = serde_json::to_string_pretty(&old_value)? + "\n";
            write_atomic(path, text)?;
            Ok(old != canonical)
        }
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "called by link and unlink from step 4 task 4")
)]
/// The owned keys `previous` recorded, if it is a JSON-keys patch.
fn previous_json(previous: Option<&PatchedFile>) -> OwnedJson {
    match previous.map(|p| &p.kind) {
        Some(PatchKind::JsonKeys(o)) => o.clone(),
        _ => OwnedJson::default(),
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "called by link and unlink from step 4 task 4")
)]
/// Everything [`apply_json`] would refuse (symbolic link, unreadable or invalid JSON,
/// a key clash), checked without writing: run before any write. A file named in
/// `also_refuse` next to `rel` (`opencode.jsonc`) is refused too: the tool may read
/// it instead of, or merged with, the file link patches.
pub fn preflight_json(
    root: &Path,
    rel: &Path,
    trust: &Path,
    wanted: &JsonKeys,
    previous: Option<&PatchedFile>,
    also_refuse: &[&str],
) -> anyhow::Result<()> {
    let path = checked(root, trust, rel)?;
    let dir = path.parent().context("patched file has no parent")?;
    if let Some(name) = also_refuse
        .iter()
        .find(|n| std::fs::symlink_metadata(dir.join(n)).is_ok())
    {
        let ours = rel.file_name().unwrap_or_default().to_string_lossy();
        bail!(
            "{} exists: rename it to {ours} or remove it, link only patches {ours}",
            dir.join(name).display()
        );
    }
    merge_json_keys(read(&path)?.as_deref(), wanted, &previous_json(previous))
        .map(|_| ())
        .with_context(|| path.display().to_string())
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "called by link and unlink from step 4 task 4")
)]
/// Merge the `wanted` keys into `root/rel` (a shared JSON config file), with the same
/// contract as [`apply_settings`].
pub fn apply_json(
    root: &Path,
    rel: &Path,
    trust: &Path,
    wanted: &JsonKeys,
    previous: Option<&PatchedFile>,
) -> anyhow::Result<(PatchedFile, bool)> {
    let path = checked(root, trust, rel)?;
    let existing = read(&path)?;
    let created = previous.is_some_and(|p| p.created) || existing.is_none();
    let (text, owned) = merge_json_keys(existing.as_deref(), wanted, &previous_json(previous))
        .with_context(|| path.display().to_string())?;
    let reformatted = write_json(
        &path,
        existing.as_deref(),
        &text,
        owned == OwnedJson::default(),
    )?;
    let patched = PatchedFile {
        path: rel.to_path_buf(),
        kind: PatchKind::JsonKeys(owned),
        created,
    };
    Ok((patched, reformatted))
}

/// `block` between the markers, with the file's line ending (`\r\n` or `\n`).
fn marked(block: &str, crlf: bool) -> String {
    let nl = if block.ends_with('\n') { "" } else { "\n" };
    let text = format!("{BLOCK_BEGIN}\n{block}{nl}{BLOCK_END}\n");
    if crlf {
        text.replace('\n', "\r\n")
    } else {
        text
    }
}

/// Byte span of the ArmadAI block (end includes the line ending after the end marker).
fn block_span(text: &str, path: &Path) -> anyhow::Result<Option<(usize, usize)>> {
    let Some(b) = text.find(BLOCK_BEGIN) else {
        return Ok(None);
    };
    let Some(e) = text[b..].find(BLOCK_END).map(|i| b + i + BLOCK_END.len()) else {
        bail!("{}: `{BLOCK_BEGIN}` without `{BLOCK_END}`", path.display());
    };
    let eol = ["\r\n", "\n"]
        .into_iter()
        .find(|eol| text[e..].starts_with(eol))
        .map_or(0, str::len);
    Ok(Some((b, e + eol)))
}

/// Put `block` between the ArmadAI markers of `root/rel`: replaced in place when the
/// markers exist, appended after the user's content otherwise.
pub fn apply_markdown_block(
    root: &Path,
    rel: &Path,
    block: &str,
    previous: Option<&PatchedFile>,
) -> anyhow::Result<PatchedFile> {
    if block.contains(BLOCK_BEGIN) || block.contains(BLOCK_END) {
        bail!("the ArmadAI block may not contain `{BLOCK_BEGIN}` or `{BLOCK_END}`");
    }
    let path = checked(root, Path::new("."), rel)?;
    let existing = read(&path)?;
    let created = previous.is_some_and(|p| p.created) || existing.is_none();
    let text = match &existing {
        None => marked(block, false),
        Some(t) => {
            let crlf = t.contains("\r\n");
            match block_span(t, &path)? {
                Some((b, e)) => format!("{}{}{}", &t[..b], marked(block, crlf), &t[e..]),
                None if t.is_empty() => marked(block, false),
                None => {
                    let eol = if crlf { "\r\n" } else { "\n" };
                    let sep = if t.ends_with('\n') { "" } else { eol };
                    format!("{t}{sep}{eol}{}", marked(block, crlf))
                }
            }
        }
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

fn is_empty_object(text: &str) -> bool {
    serde_json::from_str::<Value>(text).is_ok_and(|v| v.as_object().is_some_and(|m| m.is_empty()))
}

/// Undo `patched` under `root`: remove what link added, and the file itself when link
/// created it and nothing else is left. A missing file is nothing to undo. `trust`
/// is the target's root: where a settings.json may be (a CLAUDE.md is always at the
/// project root).
pub fn remove_patch(root: &Path, trust: &Path, patched: &PatchedFile) -> anyhow::Result<()> {
    let trust = match patched.kind {
        PatchKind::MarkdownBlock => Path::new("."),
        PatchKind::SettingsHooks(_) | PatchKind::JsonKeys(_) => trust,
    };
    let path = checked(root, trust, &patched.path)?;
    let Some(text) = read(&path)? else {
        return Ok(());
    };
    let (rest, empty) = match &patched.kind {
        PatchKind::SettingsHooks(owned) => {
            let rest = remove_entries(&text, owned).with_context(|| path.display().to_string())?;
            let empty = is_empty_object(&rest);
            (rest, empty)
        }
        PatchKind::JsonKeys(owned) => {
            let rest =
                remove_json_keys(&text, owned).with_context(|| path.display().to_string())?;
            let empty = is_empty_object(&rest);
            (rest, empty)
        }
        PatchKind::MarkdownBlock => {
            let Some((b, e)) = block_span(&text, &path)? else {
                return Ok(());
            };
            let mut rest = format!("{}{}", &text[..b], &text[e..]);
            // the blank line `apply_markdown_block` put before an appended block
            if e == text.len() {
                for eol in ["\r\n", "\n"] {
                    if rest.ends_with(&eol.repeat(2)) {
                        rest.truncate(rest.len() - eol.len());
                        break;
                    }
                }
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

    fn apply(root: &Path, prev: Option<&PatchedFile>) -> PatchedFile {
        apply_settings(
            root,
            Path::new(SETTINGS),
            Path::new(".claude"),
            &entries(),
            &BTreeMap::new(),
            prev,
        )
        .unwrap()
        .0
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_temporary_symlink_never_redirects_the_write() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, "precious").unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        for planted in [
            ".claude/.settings.json.armadai-tmp",
            ".CLAUDE.md.armadai-tmp",
        ] {
            std::os::unix::fs::symlink(&victim, dir.path().join(planted)).unwrap();
        }
        apply(dir.path(), None);
        apply_markdown_block(dir.path(), Path::new("CLAUDE.md"), "Lead.\n", None).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
        assert!(value(&dir.path().join(SETTINGS))["hooks"].is_object());
    }

    #[cfg(unix)]
    #[test]
    fn the_user_file_permissions_are_kept() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join(SETTINGS);
        let md = dir.path().join("CLAUDE.md");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, USER).unwrap();
        std::fs::write(&md, "# Mine\n").unwrap();
        for f in [&settings, &md] {
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        apply(dir.path(), None);
        apply_markdown_block(dir.path(), Path::new("CLAUDE.md"), "Lead.\n", None).unwrap();
        for f in [&settings, &md] {
            let mode = std::fs::metadata(f).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o640, "{}", f.display());
        }
    }

    /// A file in a group the user is not in (here: created in /tmp, which hands out its
    /// root group on macOS, then moved) is still patched and restored: link and unlink
    /// never stop midway over ownership. Skipped where no such file can be made.
    #[cfg(unix)]
    #[test]
    fn a_file_of_a_foreign_group_is_patched_and_restored() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join(SETTINGS);
        let md = dir.path().join("CLAUDE.md");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let ids = std::process::Command::new("id").arg("-G").output().unwrap();
        let mine: Vec<u32> = String::from_utf8_lossy(&ids.stdout)
            .split_whitespace()
            .filter_map(|g| g.parse().ok())
            .collect();
        for (f, text) in [(&settings, USER), (&md, "# Mine\n")] {
            let staged = tempfile::Builder::new()
                .prefix("armadai-foreign-group")
                .tempfile_in("/tmp")
                .unwrap()
                .into_temp_path();
            std::fs::write(&staged, text).unwrap();
            let gid = std::fs::metadata(&staged).unwrap().gid();
            if mine.contains(&gid) || std::fs::rename(&staged, f).is_err() {
                eprintln!("skipped: no file of a foreign group can be made here");
                return;
            }
            let _ = staged.keep();
        }
        let p = apply(dir.path(), None);
        let b = apply_markdown_block(dir.path(), Path::new("CLAUDE.md"), "Lead.\n", None).unwrap();
        assert!(value(&settings)["hooks"].is_object());
        remove_patch(dir.path(), Path::new(".claude"), &p).unwrap();
        remove_patch(dir.path(), Path::new("."), &b).unwrap();
        assert_eq!(
            value(&settings),
            serde_json::from_str::<Value>(USER).unwrap()
        );
        assert_eq!(std::fs::read_to_string(&md).unwrap(), "# Mine\n");
    }

    #[test]
    fn a_settings_file_already_up_to_date_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = apply(dir.path(), None);
        let file = dir.path().join(SETTINGS);
        // the user reformats it by hand: same value, other bytes
        let compact = serde_json::to_string(&value(&file)).unwrap();
        std::fs::write(&file, &compact).unwrap();
        let (_, reformatted) = apply_settings(
            dir.path(),
            Path::new(SETTINGS),
            Path::new(".claude"),
            &entries(),
            &BTreeMap::new(),
            Some(&p),
        )
        .unwrap();
        assert!(!reformatted);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), compact);
    }

    #[test]
    fn rewriting_a_user_file_says_it_was_reformatted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(SETTINGS);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{\"permissions\": {}}").unwrap();
        let (_, reformatted) = apply_settings(
            dir.path(),
            Path::new(SETTINGS),
            Path::new(".claude"),
            &entries(),
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        assert!(reformatted);
        let fresh = tempfile::tempdir().unwrap();
        let (_, reformatted) = apply_settings(
            fresh.path(),
            Path::new(SETTINGS),
            Path::new(".claude"),
            &entries(),
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        assert!(!reformatted, "a file link creates is not a reformat");
    }

    #[test]
    fn numbers_are_kept_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(SETTINGS);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            "{\"big\": 12345678901234567890123, \"sci\": 1e2, \"f\": 0.1}",
        )
        .unwrap();
        let p = apply(dir.path(), None);
        let text = std::fs::read_to_string(&file).unwrap();
        // `1e2` may be spelled `1e+2`: the same number, still not turned into `100.0`
        for n in ["12345678901234567890123", "1e", "0.1"] {
            assert!(text.contains(n), "{n} lost in {text}");
        }
        remove_patch(dir.path(), Path::new(".claude"), &p).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("12345678901234567890123") && text.contains("1e"),
            "{text}"
        );
    }

    #[test]
    fn a_path_outside_the_project_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("p");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(dir.path().join("victim.json"), "{}").unwrap();
        for bad in ["../victim.json", "/tmp/x.json"] {
            assert!(
                apply_settings(
                    &root,
                    Path::new(bad),
                    Path::new(".claude"),
                    &entries(),
                    &BTreeMap::new(),
                    None
                )
                .is_err(),
                "{bad}"
            );
            let forged = PatchedFile {
                path: PathBuf::from(bad),
                kind: PatchKind::MarkdownBlock,
                created: true,
            };
            assert!(
                remove_patch(&root, Path::new(".claude"), &forged).is_err(),
                "{bad}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("victim.json")).unwrap(),
            "{}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_parent_leaving_the_project_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("p");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(".claude")).unwrap();
        assert!(
            apply_settings(
                &root,
                Path::new(SETTINGS),
                Path::new(".claude"),
                &entries(),
                &BTreeMap::new(),
                None
            )
            .is_err()
        );
        assert!(!outside.join("settings.json").exists());
    }

    #[test]
    fn a_created_file_stays_created_across_relinks() {
        let dir = tempfile::tempdir().unwrap();
        let first = apply(dir.path(), None);
        let second = apply(dir.path(), Some(&first));
        assert!(second.created);
        remove_patch(dir.path(), Path::new(".claude"), &second).unwrap();
        assert!(!dir.path().join(SETTINGS).exists(), "no `{{}}` left behind");
    }

    #[test]
    fn a_crlf_markdown_file_gets_a_crlf_block() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("CLAUDE.md");
        std::fs::write(&file, "# Mine\r\nline\r\n").unwrap();
        let p = apply_markdown_block(dir.path(), Path::new("CLAUDE.md"), "Lead.\nMore.\n", None)
            .unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            text.matches('\n').count(),
            text.matches("\r\n").count(),
            "{text:?}"
        );
        apply_markdown_block(dir.path(), Path::new("CLAUDE.md"), "Lead v2.\n", Some(&p)).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            text.matches('\n').count(),
            text.matches("\r\n").count(),
            "{text:?}"
        );
        remove_patch(dir.path(), Path::new(".claude"), &p).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "# Mine\r\nline\r\n"
        );
    }

    #[test]
    fn a_created_settings_file_is_removed_again() {
        let dir = tempfile::tempdir().unwrap();
        let p = apply_settings(
            dir.path(),
            Path::new(SETTINGS),
            Path::new(".claude"),
            &entries(),
            &BTreeMap::new(),
            None,
        )
        .unwrap()
        .0;
        assert!(p.created);
        assert_eq!(p.path, Path::new(SETTINGS));
        let file = dir.path().join(SETTINGS);
        assert!(value(&file)["hooks"]["SessionStart"].is_array());
        assert!(leftovers(&dir.path().join(".claude")).is_empty());
        remove_patch(dir.path(), Path::new(".claude"), &p).unwrap();
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
            Path::new(".claude"),
            &entries(),
            &BTreeMap::new(),
            None,
        )
        .unwrap()
        .0;
        remove_patch(dir.path(), Path::new(".claude"), &p).unwrap();
        assert_eq!(value(&file), json!({}));
    }

    #[test]
    fn nothing_to_add_creates_no_file() {
        let dir = tempfile::tempdir().unwrap();
        apply_settings(
            dir.path(),
            Path::new(SETTINGS),
            Path::new(".claude"),
            &[],
            &BTreeMap::new(),
            None,
        )
        .unwrap();
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
            Path::new(".claude"),
            &entries(),
            &BTreeMap::new(),
            None,
        )
        .unwrap()
        .0;
        assert!(!p.created);
        assert_eq!(value(&file)["permissions"], json!({"allow": ["Bash(ls)"]}));
        // relinking with the recorded patch changes nothing
        let again = apply_settings(
            dir.path(),
            Path::new(SETTINGS),
            Path::new(".claude"),
            &entries(),
            &BTreeMap::new(),
            Some(&p),
        )
        .unwrap()
        .0;
        assert_eq!(again, p);
        remove_patch(dir.path(), Path::new(".claude"), &p).unwrap();
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
            Path::new(".claude"),
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
                Path::new(".claude"),
                &entries(),
                &BTreeMap::new(),
                None
            )
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ broken");
    }

    const OPENCODE: &str = "opencode.json";

    fn keys() -> JsonKeys {
        JsonKeys {
            values: [("default_agent".to_string(), json!("lead"))].into(),
            array_entries: [(
                "instructions".to_string(),
                vec![json!(".opencode/armadai-team.md")],
            )]
            .into(),
        }
    }

    fn preflight_oc(root: &Path) -> anyhow::Result<()> {
        preflight_json(
            root,
            Path::new(OPENCODE),
            Path::new("."),
            &keys(),
            None,
            &["opencode.jsonc"],
        )
    }

    fn apply_oc(root: &Path, prev: Option<&PatchedFile>) -> anyhow::Result<PatchedFile> {
        apply_json(root, Path::new(OPENCODE), Path::new("."), &keys(), prev).map(|p| p.0)
    }

    #[test]
    fn json_keys_are_added_kept_on_relink_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(OPENCODE);
        let user = "{\"theme\": \"x\", \"instructions\": [\"mine.md\"]}";
        std::fs::write(&file, user).unwrap();
        preflight_oc(dir.path()).unwrap();
        let p = apply_oc(dir.path(), None).unwrap();
        assert!(!p.created);
        assert_eq!(
            value(&file),
            json!({"theme": "x", "instructions": ["mine.md", ".opencode/armadai-team.md"], "default_agent": "lead"})
        );
        assert_eq!(apply_oc(dir.path(), Some(&p)).unwrap(), p);
        remove_patch(dir.path(), Path::new("."), &p).unwrap();
        assert_eq!(value(&file), serde_json::from_str::<Value>(user).unwrap());

        let fresh = tempfile::tempdir().unwrap();
        let p = apply_oc(fresh.path(), None).unwrap();
        assert!(p.created);
        remove_patch(fresh.path(), Path::new("."), &p).unwrap();
        assert!(!fresh.path().join(OPENCODE).exists());
    }

    #[test]
    fn an_opencode_jsonc_beside_or_instead_is_refused() {
        for with_json in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("opencode.jsonc"), "{}").unwrap();
            if with_json {
                std::fs::write(dir.path().join(OPENCODE), "{}").unwrap();
            }
            let err = preflight_oc(dir.path()).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!(
                    "{} exists: rename it to opencode.json or remove it, link only patches opencode.json",
                    dir.path().join("opencode.jsonc").display()
                )
            );
        }
        let dir = tempfile::tempdir().unwrap();
        preflight_oc(dir.path()).unwrap();
    }

    #[test]
    fn invalid_json_keys_file_is_refused_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(OPENCODE);
        std::fs::write(&file, "{ broken").unwrap();
        let err = preflight_oc(dir.path()).unwrap_err();
        assert_eq!(
            format!("{err:#}"),
            format!(
                "{}: not valid JSON: key must be a string at line 1 column 3",
                file.display()
            )
        );
        assert!(apply_oc(dir.path(), None).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ broken");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_json_keys_file_is_refused_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("elsewhere.json");
        std::fs::write(&real, "{}").unwrap();
        std::os::unix::fs::symlink(&real, dir.path().join(OPENCODE)).unwrap();
        for err in [
            preflight_oc(dir.path()).unwrap_err(),
            apply_oc(dir.path(), None).unwrap_err(),
        ] {
            assert_eq!(
                err.to_string(),
                format!(
                    "{} is a symbolic link; refusing to patch it",
                    dir.path().join(OPENCODE).display()
                )
            );
        }
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "{}");
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
        remove_patch(dir.path(), Path::new(".claude"), &p).unwrap();
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
        remove_patch(dir.path(), Path::new(".claude"), &p).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "top\nbottom\n");
    }

    #[test]
    fn a_created_markdown_file_is_removed_again() {
        let dir = tempfile::tempdir().unwrap();
        let p = apply_markdown_block(dir.path(), Path::new("CLAUDE.md"), "Lead.\n", None).unwrap();
        assert!(p.created);
        remove_patch(dir.path(), Path::new(".claude"), &p).unwrap();
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
