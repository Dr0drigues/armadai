use clap::Subcommand;

use armadai_core::config as app_config;

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Manage custom starter pack directories
    #[command(name = "starters-dir")]
    StartersDir {
        #[command(subcommand)]
        action: StartersDirAction,
    },
}

#[derive(Subcommand)]
pub enum StartersDirAction {
    /// List all starter directories and their sources
    List,
    /// Add a custom starter directory to config.yaml
    Add {
        /// Path to the directory containing starter packs
        path: String,
    },
    /// Remove a custom starter directory from config.yaml
    Remove {
        /// Path to remove
        path: String,
    },
}

pub fn execute(action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::StartersDir { action } => match action {
            StartersDirAction::List => starters_dir_list(),
            StartersDirAction::Add { path } => starters_dir_add(&path),
            StartersDirAction::Remove { path } => starters_dir_remove(&path),
        },
    }
}

/// List all starter directories with their source type.
fn starters_dir_list() -> anyhow::Result<()> {
    use armadai_core::starter::{all_starters_dirs, builtin_starters_dir};

    let builtin = builtin_starters_dir();
    let user = app_config::user_starters_dir();

    let config = app_config::load_user_config();
    let config_dirs: Vec<std::path::PathBuf> = config
        .starters_dirs
        .iter()
        .map(std::path::PathBuf::from)
        .collect();

    let env_dirs: Vec<std::path::PathBuf> = std::env::var("ARMADAI_STARTERS_DIRS")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.trim().is_empty())
        .map(|s| std::path::PathBuf::from(s.trim()))
        .collect();

    let project_starters = std::path::Path::new(".armadai/starters");

    for dir in all_starters_dirs() {
        let source = if dir == builtin {
            "built-in"
        } else if dir == project_starters.to_path_buf() {
            "project"
        } else if dir == user {
            "user"
        } else if env_dirs.contains(&dir) {
            "env"
        } else if config_dirs.contains(&dir) {
            "config"
        } else {
            "unknown"
        };
        let m = crate::cli::style::muted();
        anstream::println!("{m}  [{source}]{m:#} {}", dir.display());
    }
    for (name, path) in armadai_core::starter::user_packs_masking_embedded() {
        let w = crate::cli::style::warn();
        anstream::println!(
            "{w}  note: {} is your own folder: it masks the embedded pack {name} \
             (ArmadAI never writes to it){w:#}",
            path.display()
        );
    }

    Ok(())
}

/// Add a custom starter directory to config.yaml.
fn starters_dir_add(path: &str) -> anyhow::Result<()> {
    let mut config = app_config::try_load_user_config()?;

    if config.starters_dirs.contains(&path.to_string()) {
        let m = crate::cli::style::muted();
        anstream::println!("{m}Directory already in config: {path}{m:#}");
        return Ok(());
    }

    config.starters_dirs.push(path.to_string());
    save_starters_dirs(&config.starters_dirs)?;
    let o = crate::cli::style::ok();
    anstream::println!("{o}Added starter directory: {path}{o:#}");
    let m = crate::cli::style::muted();
    anstream::println!(
        "{m}  Saved to {}{m:#}",
        app_config::config_file_path().display()
    );

    Ok(())
}

/// Remove a custom starter directory from config.yaml.
fn starters_dir_remove(path: &str) -> anyhow::Result<()> {
    let mut config = app_config::try_load_user_config()?;

    let before = config.starters_dirs.len();
    config.starters_dirs.retain(|d| d != path);

    if config.starters_dirs.len() == before {
        let m = crate::cli::style::muted();
        anstream::println!("{m}Directory not found in config: {path}{m:#}");
        return Ok(());
    }

    save_starters_dirs(&config.starters_dirs)?;
    let o = crate::cli::style::ok();
    anstream::println!("{o}Removed starter directory: {path}{o:#}");
    let m = crate::cli::style::muted();
    anstream::println!(
        "{m}  Saved to {}{m:#}",
        app_config::config_file_path().display()
    );

    Ok(())
}

/// Write `dirs` as the `starters_dirs` of config.yaml, touching nothing else in the file
/// (other keys, comments, order, line endings): see [`edit_starters_dirs`].
fn save_starters_dirs(dirs: &[String]) -> anyhow::Result<()> {
    use anyhow::Context as _;
    let path = link_target(&app_config::config_file_path())?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => anyhow::bail!("cannot read {}: {e}", path.display()),
    };
    // the atomic rename replaces the file: refuse what writing it in place could not do
    if let Ok(m) = std::fs::metadata(&path) {
        if let Err(e) = std::fs::OpenOptions::new().write(true).open(&path) {
            anyhow::bail!(
                "{} is not writable ({e}): make it writable (`chmod u+w`) or edit \
                 `starters_dirs` by hand",
                path.display()
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // a fresh file is owned by whoever runs this
            let me = tempfile::tempfile()?.metadata()?.uid();
            refuse_foreign_owner(&path, m.uid(), me)?;
        }
        #[cfg(unix)]
        if std::os::unix::fs::MetadataExt::nlink(&m) > 1 {
            anyhow::bail!(
                "{} has {} hard links: edit `starters_dirs` by hand, or break the link \
                 (`cp` the file, then `mv` the copy over it)",
                path.display(),
                std::os::unix::fs::MetadataExt::nlink(&m)
            );
        }
        #[cfg(not(unix))]
        let _ = m;
    }
    let edited = edit_starters_dirs(&text, dirs).with_context(|| {
        format!(
            "{} left untouched: edit `starters_dirs` by hand",
            path.display()
        )
    })?;
    crate::compile::patch::write_atomic(&path, &edited)
}

/// The rename would hand the file over to whoever runs this: a file of another user is
/// refused (writing it in place would keep its owner).
fn refuse_foreign_owner(path: &std::path::Path, owner: u32, me: u32) -> anyhow::Result<()> {
    if owner != me {
        anyhow::bail!(
            "{} belongs to another user (uid {owner}): edit `starters_dirs` by hand, or run \
             this command as that user",
            path.display()
        );
    }
    Ok(())
}

/// `start` with its symbolic links followed (40 at most, as Linux), even to a target that
/// does not exist yet: a link stays one, its target is what gets written. A `..` after a
/// missing directory is resolved on the path, so no directory outside the target is created.
fn link_target(start: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    let mut path = start.to_path_buf();
    for _ in 0..=40 {
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_symlink() => {
                let to = std::fs::read_link(&path)?;
                path = match path.parent() {
                    Some(dir) => dir.join(to),
                    None => to,
                };
            }
            _ => {
                // each existing prefix resolved, so a `..` only ever pops a real directory
                let mut out = std::path::PathBuf::new();
                for c in path.components() {
                    match c {
                        std::path::Component::ParentDir => {
                            out.pop();
                        }
                        std::path::Component::CurDir => {}
                        c => {
                            out.push(c);
                            if let Ok(real) = out.canonicalize() {
                                out = real;
                            }
                        }
                    }
                }
                return Ok(out);
            }
        }
    }
    anyhow::bail!("{}: more than 40 levels of symbolic links", start.display())
}

const KEY: &str = "starters_dirs:";

/// `text` with only its top-level `starters_dirs` reading as `dirs`: matching item lines
/// are kept as they are, the others dropped, the missing ones appended after the last
/// item (the key itself appended at the end of the file when absent, removed when the
/// list ends up empty, or written `[]` when that would drop a comment). A block list of
/// one-line items, null and a one-line flow list are edited; anything else (anchor or
/// alias, multi-line item or flow list...) is refused, and so is any edit whose result
/// does not read back as `dirs`.
fn edit_starters_dirs(text: &str, dirs: &[String]) -> anyhow::Result<String> {
    use anyhow::bail;
    if text.starts_with('\u{feff}') {
        bail!("the file starts with a byte order mark (BOM): remove it");
    }
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let render = |d: &String| -> anyhow::Result<String> {
        let item = armadai_model::yaml::to_string(&[d])?;
        let item = item.trim_end_matches('\n');
        if item.contains('\n') {
            bail!("{d:?} does not fit on one line");
        }
        Ok(item.to_string())
    };
    let mut out = String::new();
    let push_line = |out: &mut String, line: &str| {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(eol);
        }
        out.push_str(line);
        out.push_str(eol);
    };

    let keys: Vec<usize> = (0..lines.len())
        .filter(|&i| lines[i].starts_with(KEY))
        .collect();
    match keys[..] {
        [] => {
            out.push_str(text);
            if !dirs.is_empty() {
                push_line(&mut out, "starters_dirs:");
                for d in dirs {
                    push_line(&mut out, &render(d)?);
                }
            }
        }
        [k] => {
            let raw = lines[k][KEY.len()..].trim_end_matches(['\r', '\n']);
            let line_eol = &lines[k][KEY.len() + raw.len()..];
            // a comment opens at a `#` that starts the value or follows a blank
            let cut = raw
                .char_indices()
                .find(|&(i, c)| c == '#' && (i == 0 || raw[..i].ends_with([' ', '\t'])))
                .map_or(raw.len(), |(i, _)| i);
            let (value, comment) = (raw[..cut].trim(), &raw[cut..]);
            if value.is_empty() {
                // block list: its items run until the next top-level line that is not one
                let (mut items, mut indent) = (Vec::new(), None);
                for (i, l) in lines.iter().enumerate().skip(k + 1) {
                    let t = l.trim();
                    if t.is_empty() || t.starts_with('#') {
                        continue;
                    }
                    let ind = &l[..l.len() - l.trim_start().len()];
                    if ind.is_empty() && !t.starts_with('-') {
                        break;
                    }
                    if *indent.get_or_insert(ind) != ind || !(t.starts_with("- ") || t == "-") {
                        bail!("line {}: an item must be `- <path>` on one line", i + 1);
                    }
                    let parsed: Option<Vec<String>> = armadai_model::yaml::from_str(t).ok();
                    let Some([v]) = parsed.as_deref() else {
                        bail!("line {}: `{t}` is not a single path", i + 1);
                    };
                    items.push((i, v.clone()));
                }
                // what the scan missed (a block scalar `|-` whose lines look like comments...)
                let before: Option<app_config::UserConfig> =
                    armadai_model::yaml::from_str(text).ok().flatten();
                if before.unwrap_or_default().starters_dirs
                    != items.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>()
                {
                    bail!("an item of `starters_dirs` is not a path on one line");
                }
                let end = items.last().map_or(k + 1, |(i, _)| i + 1);
                let missing: Vec<&String> = dirs
                    .iter()
                    .filter(|d| !items.iter().any(|(_, v)| v == *d))
                    .collect();
                let emptied = missing.is_empty() && !items.iter().any(|(_, v)| dirs.contains(v));
                // an emptied list keeps its key as `[]` rather than lose a comment
                let keep_key = !comment.is_empty()
                    || lines[k + 1..end].iter().any(|l| l.trim().starts_with('#'));
                for (i, l) in lines.iter().enumerate() {
                    if emptied && keep_key && i == k {
                        out.push_str(&format!("starters_dirs: []{raw}{line_eol}"));
                        continue;
                    }
                    if i == end {
                        for d in &missing {
                            push_line(&mut out, &format!("{}{}", indent.unwrap_or(""), render(d)?));
                        }
                    }
                    let gone = (emptied && !keep_key && (k..end).contains(&i))
                        || items.iter().any(|(j, v)| *j == i && !dirs.contains(v));
                    if !gone {
                        out.push_str(l);
                    }
                }
                if end == lines.len() {
                    for d in &missing {
                        push_line(&mut out, &format!("{}{}", indent.unwrap_or(""), render(d)?));
                    }
                }
            } else if matches!(value, "~" | "null" | "Null" | "NULL")
                || value.starts_with('[') && value.ends_with(']')
            {
                // null or one-line flow list: rewritten as a block list
                if value.starts_with('[') {
                    armadai_model::yaml::from_str::<Vec<String>>(value)
                        .map_err(|e| anyhow::anyhow!("line {}: {e}", k + 1))?;
                }
                let comment = if comment.is_empty() {
                    String::new()
                } else {
                    format!(" {comment}")
                };
                for (i, l) in lines.iter().enumerate() {
                    if i != k {
                        out.push_str(l);
                    } else if !dirs.is_empty() {
                        push_line(&mut out, &format!("starters_dirs:{comment}"));
                        for d in dirs {
                            push_line(&mut out, &render(d)?);
                        }
                    } else if !comment.is_empty() {
                        push_line(&mut out, &format!("starters_dirs: []{comment}"));
                    }
                }
            } else {
                bail!(
                    "line {}: only a block list, null or a one-line flow list is edited",
                    k + 1
                );
            }
        }
        _ => bail!("`starters_dirs` appears more than once"),
    }
    let read: Option<app_config::UserConfig> = armadai_model::yaml::from_str(&out)
        .map_err(|e| anyhow::anyhow!("the edited file would not read: {e}"))?;
    if read.unwrap_or_default().starters_dirs != dirs {
        bail!("the edited file would not read back as the expected list");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::edit_starters_dirs as edit;

    fn dirs(d: &[&str]) -> Vec<String> {
        d.iter().map(|s| s.to_string()).collect()
    }

    const FULL: &str = "\
# ArmadAI user configuration
defaults:
  provider: anthropic   # kept
starters_dirs:   # mine
  - /opt/a   # first
  # between
  - '/opt/b'

# costs below
costs:
  enabled: true
";

    #[test]
    fn only_the_items_change_and_everything_else_is_kept() {
        let added = edit(FULL, &dirs(&["/opt/a", "/opt/b", "/opt/c"])).unwrap();
        assert_eq!(
            added,
            FULL.replace("  - '/opt/b'\n", "  - '/opt/b'\n  - /opt/c\n")
        );
        let removed = edit(FULL, &dirs(&["/opt/b"])).unwrap();
        assert_eq!(removed, FULL.replace("  - /opt/a   # first\n", ""));
        // the list emptied: its comments stay, under a `[]` key
        let emptied = edit(FULL, &[]).unwrap();
        assert_eq!(
            emptied,
            FULL.replace(
                "starters_dirs:   # mine\n  - /opt/a   # first\n  # between\n  - '/opt/b'\n",
                "starters_dirs: []   # mine\n  # between\n"
            )
        );
        assert_eq!(
            edit("starters_dirs: # c\n  # mine\n  - /a\n", &[]).unwrap(),
            "starters_dirs: [] # c\n  # mine\n"
        );
        assert_eq!(
            edit("starters_dirs: # c\n- /a\n", &[]).unwrap(),
            "starters_dirs: [] # c\n"
        );
        assert_eq!(
            edit("starters_dirs:\n  # mine\n  - /a\n", &[]).unwrap(),
            "starters_dirs: []\n  # mine\n"
        );
        // without a comment to keep, the key goes
        assert_eq!(
            edit("costs: 1\nstarters_dirs:\n- /a\n\nx: 2\n", &[]).unwrap(),
            "costs: 1\n\nx: 2\n"
        );
    }

    #[test]
    fn a_comment_after_a_flow_or_null_value_is_kept() {
        let text = "starters_dirs: [/a] # note\n";
        assert_eq!(
            edit(text, &dirs(&["/a", "/b"])).unwrap(),
            "starters_dirs: # note\n- /a\n- /b\n"
        );
        assert_eq!(edit(text, &[]).unwrap(), "starters_dirs: [] # note\n");
        assert_eq!(
            edit("starters_dirs: [/a] # see [x]\n", &dirs(&["/a", "/b"])).unwrap(),
            "starters_dirs: # see [x]\n- /a\n- /b\n"
        );
        for null in ["~ # c", "null  # c", "[] # c"] {
            assert_eq!(
                edit(&format!("starters_dirs: {null}\n"), &dirs(&["/a"])).unwrap(),
                "starters_dirs: # c\n- /a\n",
                "{null}"
            );
        }
    }

    #[test]
    fn an_absent_key_is_appended_at_the_end() {
        assert_eq!(edit("", &dirs(&["/x"])).unwrap(), "starters_dirs:\n- /x\n");
        assert_eq!(
            edit("# notes\ncosts:\n  enabled: true", &dirs(&["/x"])).unwrap(),
            "# notes\ncosts:\n  enabled: true\nstarters_dirs:\n- /x\n"
        );
    }

    #[test]
    fn crlf_lines_stay_crlf() {
        let text = "# notes\r\nstarters_dirs:\r\n- /a\r\ncosts: 1\r\n";
        assert_eq!(
            edit(text, &dirs(&["/a", "/b"])).unwrap(),
            "# notes\r\nstarters_dirs:\r\n- /a\r\n- /b\r\ncosts: 1\r\n"
        );
        assert_eq!(
            edit("costs: 1\r\n", &dirs(&["/b"])).unwrap(),
            "costs: 1\r\nstarters_dirs:\r\n- /b\r\n"
        );
    }

    #[test]
    fn a_last_item_without_newline_gets_one_before_the_new_item() {
        assert_eq!(
            edit("starters_dirs:\n  - /a", &dirs(&["/a", "/b"])).unwrap(),
            "starters_dirs:\n  - /a\n  - /b\n"
        );
    }

    #[test]
    fn a_one_line_flow_list_becomes_a_block_list() {
        let text = "costs: 1\nstarters_dirs: [/a, \"/b\"]\nlogging: x\n";
        assert_eq!(
            edit(text, &dirs(&["/a", "/b", "/c"])).unwrap(),
            "costs: 1\nstarters_dirs:\n- /a\n- /b\n- /c\nlogging: x\n"
        );
        assert_eq!(edit(text, &[]).unwrap(), "costs: 1\nlogging: x\n");
        assert_eq!(
            edit("starters_dirs: []\n", &dirs(&["/a"])).unwrap(),
            "starters_dirs:\n- /a\n"
        );
    }

    #[test]
    fn styles_that_cannot_be_edited_line_by_line_are_refused() {
        for text in [
            // a `#` inside a quoted flow item cannot be told from a comment
            "starters_dirs: [\"/a #b\"]\n",
            "starters_dirs: [/a,\n  /b]\n",
            "starters_dirs: &d\n- /a\nother: *d\n",
            "base: &p /a\nstarters_dirs:\n- *p\n",
            // removing an anchored item would break its alias
            "starters_dirs:\n- &p /a\nother: *p\n",
            "starters_dirs:\n- /a\n  /continued\n",
            "starters_dirs:\n  - /a\n - /b\n",
            "starters_dirs:\n- [/a]\n",
            "starters_dirs: []\nstarters_dirs: []\n",
            // not recognised as the key: appending one would duplicate it
            "\"starters_dirs\": [/a]\n",
        ] {
            assert!(edit(text, &dirs(&["/z"])).is_err(), "{text:?}");
        }
        for (text, want, msg) in [
            // only the read-back check catches these two
            (
                "{a: 1}\n",
                &["/z"][..],
                "the edited file would not read back as the expected list",
            ),
            (
                "{starters_dirs: [/a]}\n",
                &["/a", "/z"],
                "the edited file would not read back as the expected list",
            ),
            (
                "\u{feff}starters_dirs:\n- /a\n",
                &["/a", "/z"],
                "the file starts with a byte order mark (BOM): remove it",
            ),
            // removing another item would leave `#x` behind as a comment
            (
                "starters_dirs:\n- |-\n    #x\n- /b\n",
                &["#x"],
                "an item of `starters_dirs` is not a path on one line",
            ),
        ] {
            let e = edit(text, &dirs(want)).unwrap_err().to_string();
            assert_eq!(e, msg, "{text:?}");
        }
    }

    #[test]
    fn a_null_list_becomes_a_block_list() {
        for null in ["~", "null"] {
            let text = format!("starters_dirs: {null}\ncosts: 1\n");
            assert_eq!(
                edit(&text, &dirs(&["/a"])).unwrap(),
                "starters_dirs:\n- /a\ncosts: 1\n"
            );
        }
    }

    #[test]
    fn a_file_of_another_user_is_refused() {
        let p = std::path::Path::new("/c/config.yaml");
        assert!(super::refuse_foreign_owner(p, 501, 501).is_ok());
        assert_eq!(
            super::refuse_foreign_owner(p, 0, 501)
                .unwrap_err()
                .to_string(),
            "/c/config.yaml belongs to another user (uid 0): edit `starters_dirs` by hand, \
             or run this command as that user"
        );
    }

    #[cfg(unix)]
    #[test]
    fn link_target_follows_40_links_and_no_more() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().canonicalize().unwrap();
        for i in 0..41 {
            symlink(format!("l{}", i + 1), d.join(format!("l{i}"))).unwrap();
        }
        // l1 -> ... -> l41 (missing): 40 links
        assert_eq!(super::link_target(&d.join("l1")).unwrap(), d.join("l41"));
        let e = super::link_target(&d.join("l0")).unwrap_err().to_string();
        assert_eq!(
            e,
            format!(
                "{}: more than 40 levels of symbolic links",
                d.join("l0").display()
            )
        );
        // a `..` after a linked directory pops the directory it points to, as the kernel does
        std::fs::create_dir_all(d.join("far/inner")).unwrap();
        symlink(d.join("far/inner"), d.join("hop")).unwrap();
        symlink("hop/../z.yaml", d.join("cfg.yaml")).unwrap();
        assert_eq!(
            super::link_target(&d.join("cfg.yaml")).unwrap(),
            d.join("far/z.yaml")
        );
        symlink("b", d.join("a")).unwrap();
        symlink("a", d.join("b")).unwrap();
        let e = super::link_target(&d.join("a")).unwrap_err().to_string();
        assert_eq!(
            e,
            format!(
                "{}: more than 40 levels of symbolic links",
                d.join("a").display()
            )
        );
    }
}
