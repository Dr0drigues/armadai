//! Native starter packs: a `pack.yaml` with `format: 1` next to a canonical tree.
//!
//! A pack is copied by a white list (`project.yaml`, `agents/*.md`, `templates/*.md`,
//! `commands/*.md`, `skills/**`, `flows/*.yaml`, `automations/*.yaml`); anything else (`pack.yaml`,
//! `.armadai-version`, `link-manifest.yaml`, `.migrated/`, `.gitignore`, ...) stays
//! behind. A pack holding any symbolic link is refused. The pack is validated in a
//! scratch project before anything is written, and the install is undone if it
//! introduces an error into the project it lands in.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

const IGNORED: &str = "ignored-pack-file";

/// `project.yaml` of a pack that ships none.
pub const DEFAULT_PROJECT: &str = "schema_version: 1\ntargets: [claude]\n";

fn listed(rel: &Path) -> bool {
    let c: Vec<&str> = rel.iter().filter_map(|s| s.to_str()).collect();
    match c.as_slice() {
        ["project.yaml"] => true,
        ["agents", f] | ["templates", f] | ["commands", f] => f.ends_with(".md"),
        ["flows", f] | ["automations", f] => f.ends_with(".yaml"),
        ["skills", _, _, ..] => true,
        _ => false,
    }
}

/// A pack's files: those it installs, as (path relative to `.armadai/`, bytes,
/// executable on Unix in the pack), and those inside its folders the white list
/// leaves out (top-level files such as `pack.yaml` or a README are expected and not
/// listed).
pub struct PackFiles {
    pub files: Vec<(PathBuf, Vec<u8>, bool)>,
    pub ignored: Vec<PathBuf>,
}

pub fn files(pack_dir: &Path) -> anyhow::Result<PackFiles> {
    let mut out = Vec::new();
    let mut ignored = Vec::new();
    let mut todo = vec![pack_dir.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for e in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = e?.path();
            let meta = std::fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() {
                bail!(
                    "the pack holds a symbolic link, refused: {}",
                    path.display()
                );
            }
            let rel = path.strip_prefix(pack_dir)?.to_path_buf();
            if meta.is_dir() {
                todo.push(path);
            } else if meta.is_file() && listed(&rel) {
                #[cfg(unix)]
                let exec =
                    std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o111 != 0;
                #[cfg(not(unix))]
                let exec = false;
                out.push((rel, std::fs::read(&path)?, exec));
            } else if rel.components().count() > 1 {
                ignored.push(rel);
            }
        }
    }
    out.sort();
    ignored.sort();
    Ok(PackFiles {
        files: out,
        ignored,
    })
}

/// The pack as a scratch project (`<tmp>/.armadai/`, the default `project.yaml`
/// when it ships none) and its diagnostics, positioned in the pack itself: one
/// `ignored-pack-file` warning per ignored file first, then the validator's.
pub fn stage(
    pack_dir: &Path,
    pack: &PackFiles,
) -> anyhow::Result<(tempfile::TempDir, Vec<armadai_model::Diagnostic>)> {
    let files = &pack.files;
    let tmp = tempfile::tempdir()?;
    let base = tmp.path().join(".armadai");
    for (rel, bytes, _) in files {
        let dest = base.join(rel);
        std::fs::create_dir_all(dest.parent().context("a file has a parent")?)?;
        std::fs::write(dest, bytes)?;
    }
    if !files.iter().any(|(r, ..)| r == Path::new("project.yaml")) {
        std::fs::create_dir_all(&base)?;
        std::fs::write(base.join("project.yaml"), DEFAULT_PROJECT)?;
    }
    let ignored = pack.ignored.iter().map(|rel| armadai_model::Diagnostic {
        severity: armadai_model::Severity::Warning,
        code: IGNORED,
        file: pack_dir.join(rel),
        path: String::new(),
        line: None,
        column: None,
        message: "not part of a pack: init --pack does not install it".into(),
        hint: Some(
            "a pack installs project.yaml, agents/*.md, templates/*.md, commands/*.md, skills/**, \
             flows/*.yaml and automations/*.yaml"
                .into(),
        ),
    });
    let ds = ignored
        .chain(
            armadai_model::validate(tmp.path())
                .into_iter()
                .map(|mut d| {
                    if let Ok(rel) = d.file.strip_prefix(&base) {
                        d.file = pack_dir.join(rel);
                    }
                    d
                }),
        )
        .collect();
    Ok((tmp, ds))
}

/// What an install created, to undo it.
#[derive(Default)]
struct Created {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl Created {
    /// Write `bytes` at `base/rel`, never through a symbolic link, never over a file.
    fn write(
        &mut self,
        base: &Path,
        rel: &Path,
        bytes: &[u8],
        executable: bool,
    ) -> anyhow::Result<()> {
        let mut at = base.to_path_buf();
        for c in std::iter::once(None).chain(rel.components().map(Some)) {
            if let Some(c) = c {
                at.push(c);
            }
            match std::fs::symlink_metadata(&at) {
                Ok(m) if m.file_type().is_symlink() => {
                    bail!(
                        "refusing to write through the symbolic link {}",
                        at.display()
                    )
                }
                Ok(_) => {}
                Err(_) if at != base.join(rel) => {
                    std::fs::create_dir(&at)
                        .with_context(|| format!("creating {}", at.display()))?;
                    self.dirs.push(at.clone());
                }
                Err(_) => {}
            }
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&at)
            .with_context(|| format!("creating {}", at.display()))?;
        self.files.push(at);
        f.write_all(bytes)?;
        #[cfg(unix)]
        if executable {
            // only the executable bit, like `copy_executable_bits` in `link`
            use std::os::unix::fs::PermissionsExt;
            let mode = f.metadata()?.permissions().mode();
            f.set_permissions(std::fs::Permissions::from_mode(
                mode | ((mode & 0o444) >> 2),
            ))?;
        }
        #[cfg(not(unix))]
        let _ = executable;
        Ok(())
    }

    fn undo(&self) {
        for f in &self.files {
            let _ = std::fs::remove_file(f);
        }
        for d in self.dirs.iter().rev() {
            let _ = std::fs::remove_dir(d);
        }
    }
}

/// Install the native pack at `pack_dir` into `root/.armadai/`. Existing files are
/// kept (one warning each); an existing `project.yaml` wins over the pack's.
pub fn install(root: &Path, pack_dir: &Path, name: &str) -> anyhow::Result<()> {
    let pack = files(pack_dir)?;
    let files = &pack.files;
    let w = crate::cli::style::warn();
    let (tmp, ds) = stage(pack_dir, &pack)?;
    // the other warnings of the validator stay silent here
    for d in ds.iter().filter(|d| d.code == IGNORED) {
        eprintln!("{}", d.render());
    }
    if armadai_model::has_errors(&ds) {
        for d in ds.iter().filter(|d| d.code != IGNORED) {
            eprintln!("{}", d.render());
        }
        bail!("pack '{name}' is invalid; nothing was written");
    }
    let base = root.join(".armadai");
    let had_project = base.join("project.yaml").is_file();
    let key = |d: &armadai_model::Diagnostic| (d.code, d.file.clone());
    let before: BTreeSet<_> = if had_project {
        armadai_model::validate(root).iter().map(key).collect()
    } else {
        BTreeSet::new()
    };

    let mut created = Created::default();
    let mut installed: Vec<&Path> = Vec::new();
    let mut kept = 0;
    let mut skills_kept = BTreeSet::new();
    // the skill folders the project had before this install
    let had_skill = |skill: &std::ffi::OsStr| {
        std::fs::symlink_metadata(base.join("skills").join(skill)).is_ok()
    };
    let existing: BTreeSet<std::ffi::OsString> = (files.iter())
        .filter_map(|(r, ..)| r.starts_with("skills").then(|| r.iter().nth(1)).flatten())
        .filter(|s| had_skill(s))
        .map(|s| s.to_owned())
        .collect();
    let result = (|| -> anyhow::Result<()> {
        for (rel, bytes, exec) in files {
            if rel == Path::new("project.yaml") && had_project {
                anstream::eprintln!(
                    "{w}  warn: .armadai/project.yaml exists: the project.yaml of the pack is \
                     ignored (its targets, coordinator and policy are not applied){w:#}"
                );
                kept += 1;
                continue;
            }
            // a skill is one folder: one the project has is never mixed with the pack's
            if let Some(skill) = rel
                .starts_with("skills")
                .then(|| rel.iter().nth(1))
                .flatten()
                && existing.contains(skill)
            {
                if skills_kept.insert(skill.to_owned()) {
                    anstream::eprintln!(
                        "{w}  warn: skills/{} exists, kept (the pack's skill is not mixed in){w:#}",
                        skill.to_string_lossy()
                    );
                }
                kept += 1;
                continue;
            }
            if std::fs::symlink_metadata(base.join(rel)).is_ok() {
                anstream::eprintln!("{w}  warn: {} exists, kept{w:#}", rel.display());
                kept += 1;
                continue;
            }
            created.write(&base, rel, bytes, *exec)?;
            installed.push(rel);
        }
        if !base.join("project.yaml").exists() {
            created.write(
                &base,
                Path::new("project.yaml"),
                DEFAULT_PROJECT.as_bytes(),
                false,
            )?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        created.undo();
        return Err(e);
    }
    let introduced: Vec<_> = armadai_model::validate(root)
        .into_iter()
        .filter(|d| !before.contains(&key(d)))
        .collect();
    if armadai_model::has_errors(&introduced) {
        created.undo();
        for d in &introduced {
            eprintln!("(rolled back) {}", d.render());
        }
        bail!("pack '{name}' would make the project invalid; nothing was written");
    }
    crate::linker::manifest::ensure_gitignore(root).context("cannot write .armadai/.gitignore")?;

    // automations become hooks the target runs: never installed in silence
    if let Some(p) = armadai_model::load_project(tmp.path()).project {
        let new = |id: &str| {
            installed.contains(
                &Path::new("automations")
                    .join(format!("{id}.yaml"))
                    .as_path(),
            )
        };
        for (id, a) in p.automations.iter().filter(|(id, _)| new(id)) {
            let event = armadai_model::yaml::to_string(&a.event).unwrap_or_default();
            anstream::eprintln!(
                "{w}  warn: automation {id} runs `{}` on {} (it becomes a hook of the target){w:#}",
                a.run,
                event.trim()
            );
        }
    }
    let count = |dir: &str| {
        (installed.iter())
            .filter(|r| r.starts_with(dir))
            .filter(|r| dir != "skills" || r.ends_with("SKILL.md"))
            .count()
    };
    let o = crate::cli::style::ok();
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    let templates = match count("templates") {
        0 => String::new(),
        n => format!(", {n} template(s)"),
    };
    let commands = match count("commands") {
        0 => String::new(),
        n => format!(", {n} command(s)"),
    };
    anstream::println!(
        "{o}Created .armadai/ from pack{o:#} {a}'{name}'{a:#}: {m}{} agent(s), {} skill(s), \
         {} flow(s), {} automation(s){templates}{commands}{m:#}",
        count("agents"),
        count("skills"),
        count("flows"),
        count("automations"),
    );
    if kept > 0 {
        anstream::println!("{m}  {kept} existing file(s) kept{m:#}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_white_list() {
        for ok in [
            "project.yaml",
            "agents/a.md",
            "templates/t.md",
            "commands/c.md",
            "flows/f.yaml",
            "automations/x.yaml",
            "skills/s/SKILL.md",
            "skills/s/scripts/run.sh",
        ] {
            assert!(listed(Path::new(ok)), "{ok}");
        }
        for no in [
            "pack.yaml",
            ".armadai-version",
            "link-manifest.yaml",
            ".gitignore",
            ".migrated/x/a.md",
            "agents/a.txt",
            "agents/sub/a.md",
            "templates/t.txt",
            "templates/sub/t.md",
            "commands/c.txt",
            "commands/sub/c.md",
            "flows/f.yml",
            "skills/SKILL.md",
            "prompts/p.md",
            "native/claude/x",
        ] {
            assert!(!listed(Path::new(no)), "{no}");
        }
    }
}
