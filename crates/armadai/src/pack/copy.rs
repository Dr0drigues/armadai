//! `pack update` of a pack installed by copy (`init --pack`): what the install wrote is
//! recorded (path -> digest) in the `packs:` section of the link manifest, so an update
//! replaces what is unchanged, keeps and says what the user edited, and removes what the
//! pack dropped.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail, ensure};
use armadai_core::registries::{RegistrySource, cache_key};
use armadai_core::starter::{StarterPack, find_pack};

use super::{Done, Env, Undo, cache, errors_now, refuse_invalid, resolve_in, source, status};
use crate::cli::native_pack;
use crate::linker::manifest::{self, InstalledPack, digest_of};

const PROJECT: &str = "project.yaml";

/// What an update does with one path of the pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyAction {
    /// Unchanged on disk (or already the pack's bytes): the pack's new version goes in.
    Replace,
    /// New in the pack, never installed: written.
    Add,
    /// Installed, unchanged, gone from the pack: removed.
    Remove,
    /// Edited since the install: kept, the pack's version is not applied.
    KeepEdited,
    /// Present, never installed by the pack (the project's own file) and different: kept.
    KeepNotInstalled,
    /// Installed, then removed by hand: not added back.
    KeepRemovedByHand,
    /// Gone from the pack but edited: kept.
    KeepDroppedEdited,
    /// An edited `project.yaml`: the pack's keys are added to it, the user's win.
    MergeManifest,
}

/// What happens to each path: `recorded` is what the install wrote, `disk` the digest now
/// (`None` when absent), `incoming` the pack's new files. A path the pack dropped and the
/// user removed has nothing left to do and is not listed. Pure.
pub fn plan_copy_update(
    recorded: &BTreeMap<PathBuf, String>,
    disk: &BTreeMap<PathBuf, Option<String>>,
    incoming: &BTreeMap<PathBuf, Vec<u8>>,
) -> BTreeMap<PathBuf, CopyAction> {
    let paths: BTreeSet<&PathBuf> = recorded.keys().chain(incoming.keys()).collect();
    let mut out = BTreeMap::new();
    for p in paths {
        let now = disk.get(p).and_then(|d| d.as_ref());
        let was = recorded.get(p);
        let action = match (incoming.get(p), now) {
            (Some(_), None) if was.is_some() => CopyAction::KeepRemovedByHand,
            (Some(_), None) => CopyAction::Add,
            (Some(bytes), Some(d)) if *d == digest_of(bytes) || was == Some(d) => {
                CopyAction::Replace
            }
            (Some(_), Some(_)) if p == Path::new(PROJECT) => CopyAction::MergeManifest,
            (Some(_), Some(_)) if was.is_none() => CopyAction::KeepNotInstalled,
            (Some(_), Some(_)) => CopyAction::KeepEdited,
            (None, None) => continue,
            (None, Some(d)) if was == Some(d) => CopyAction::Remove,
            (None, Some(_)) => CopyAction::KeepDroppedEdited,
        };
        out.insert(p.clone(), action);
    }
    out
}

/// Where the new version of the pack is, and which one it is.
struct Newest {
    dir: PathBuf,
    version: Option<String>,
    id: Option<String>,
}

fn locate(
    name: &str,
    recorded: &InstalledPack,
    registry: &Path,
    known: &[RegistrySource],
    version: Option<&str>,
) -> anyhow::Result<Newest> {
    let src = recorded.source.as_str();
    let folder = if src == "embedded" {
        ensure!(
            version.is_none(),
            "pack {name} is embedded in armadai and has no versions: run `armadai pack update` \
             without one"
        );
        let found = find_pack(name)?.filter(|f| f.project_source.is_none());
        Some(
            found
                .map(|f| f.dir)
                .with_context(|| format!("pack {name} is no longer embedded in this armadai"))?,
        )
    } else if Path::new(src).is_dir() {
        ensure!(
            version.is_none(),
            "pack {name} was installed from a folder and has no versions: run `armadai pack \
             update` without one"
        );
        Some(PathBuf::from(src))
    } else if Path::new(src).is_absolute() {
        bail!(
            "pack {name} was installed from the folder {src}, which is no longer there: nothing \
             was changed"
        );
    } else {
        None
    };
    if let Some(dir) = folder {
        let id = Some(cache::tree_digest(&dir)?);
        return Ok(Newest {
            dir,
            version: None,
            id,
        });
    }
    let real = source::real_url_in(src, known)?;
    let source = super::source_of(known, &real);
    super::sync_or_stop(registry, &source)?;
    let held = cache::versions_in(registry, &real).and_then(|v| v.packs.get(name).cloned());
    let held = held.with_context(|| {
        format!(
            "pack {name} is no longer in {}: nothing was changed",
            source::recorded(&real)
        )
    })?;
    if held.versions.is_empty() && version.is_none() {
        // a source without versions: its cache holds the one the sync just brought
        let dir = registry
            .join("starters")
            .join(cache_key(&real))
            .join(&held.path);
        let id = Some(cache::tree_digest(&dir)?);
        return Ok(Newest {
            dir,
            version: None,
            id,
        });
    }
    let (best, path) = resolve_in(registry, &source, name, version.unwrap_or("latest"), false)?;
    let dir = cache::ensure_snapshot_in(registry, &source, name, &path, &best)?;
    Ok(Newest {
        dir,
        id: best.commit.clone().or(best.digest.clone()),
        version: Some(best.version),
    })
}

/// A project with no record of its pack: M8 when it holds what a pack could have
/// installed (a fresh clone loses the manifest, which is not versioned), else M9.
/// `pack status` says the same.
pub(super) fn unknown_text(root: &Path) -> String {
    let holds_more = native_pack::read_listed(&root.join(".armadai"), false)
        .is_ok_and(|f| f.iter().any(|(r, ..)| r != Path::new(PROJECT)));
    if holds_more {
        "the origin of this project's pack is unknown here (.armadai/link-manifest.yaml has \
         no packs: section, as after a fresh clone): pack update guesses nothing, nothing \
         was changed"
            .to_string()
    } else {
        "this project references no pack and records no installed pack: use \
         `armadai pack add <name>` or `armadai init --pack <name>`"
            .to_string()
    }
}

fn unknown(root: &Path) -> anyhow::Error {
    anyhow::anyhow!(unknown_text(root))
}

/// Refuse to follow a symbolic link on the way to `rel` under `base`.
fn no_link(base: &Path, rel: &Path) -> anyhow::Result<()> {
    let mut at = base.to_path_buf();
    for c in rel.components() {
        at.push(c);
        ensure!(
            !std::fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()),
            "{} is a symbolic link; it is not written through",
            at.display()
        );
    }
    Ok(())
}

fn write(
    base: &Path,
    rel: &Path,
    bytes: &[u8],
    exec: bool,
    dirs: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    no_link(base, rel)?;
    let at = base.join(rel);
    let mut missing = Vec::new();
    let mut up = at.parent();
    while let Some(d) = up.filter(|d| *d != base && !d.exists()) {
        missing.push(d.to_path_buf());
        up = d.parent();
    }
    std::fs::create_dir_all(at.parent().context("a file has a parent")?)?;
    dirs.extend(missing.into_iter().rev());
    std::fs::write(&at, bytes).with_context(|| format!("writing {}", at.display()))?;
    #[cfg(unix)]
    if exec {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&at)?.permissions().mode();
        std::fs::set_permissions(&at, std::fs::Permissions::from_mode(mode | 0o111))?;
    }
    #[cfg(not(unix))]
    let _ = exec;
    Ok(())
}

/// `pack update` for the pack the project installed by copy.
pub fn update_in(
    root: &Path,
    registry: &Path,
    known: &[RegistrySource],
    version: Option<&str>,
    env: Env,
) -> anyhow::Result<Done> {
    let mut installed = manifest::installed_packs(root).into_iter();
    let Some((name, recorded)) = installed.next() else {
        return Err(unknown(root));
    };
    if installed.next().is_some() {
        bail!(
            "several packs were installed by copy in this project: pack update takes one; \
             nothing was changed"
        );
    }
    let new = locate(&name, &recorded, registry, known, version)?;
    let pack = StarterPack::load(&new.dir)?;
    ensure!(
        pack.format == Some(1),
        "pack '{name}' is not a native pack (format: 1): pack update cannot apply it"
    );
    let shown = new.version.clone().unwrap_or_else(|| "(no version)".into());
    let id12 = new
        .id
        .as_deref()
        .map(status::id12)
        .unwrap_or_default()
        .to_string();
    let same = match (&recorded.version, &new.version) {
        (Some(a), Some(b)) => a == b,
        (None, None) => new.id.is_some() && recorded.id == new.id,
        _ => false,
    };
    if same {
        return Ok(Done {
            relink: true,
            lines: vec![format!("Pack {name} is up to date: {shown} ({id12})")],
            undo: None,
        });
    }

    refuse_invalid(root, registry, env, "armadai pack update")?;
    let base = root.join(".armadai");
    let incoming = native_pack::files(&new.dir)?;
    let exec: BTreeSet<&PathBuf> = (incoming.files.iter())
        .filter(|f| f.2)
        .map(|f| &f.0)
        .collect();
    let bytes: BTreeMap<PathBuf, Vec<u8>> = (incoming.files.iter())
        .map(|(r, b, _)| (r.clone(), b.clone()))
        .collect();
    let mut disk = BTreeMap::new();
    for p in recorded.files.keys().chain(bytes.keys()) {
        no_link(&base, p)?;
        let now = std::fs::read(base.join(p)).ok().map(|b| digest_of(&b));
        disk.insert(p.clone(), now);
    }
    let plan = plan_copy_update(&recorded.files, &disk, &bytes);

    let touched: Vec<PathBuf> = (plan.iter())
        .filter(|(_, a)| {
            !matches!(
                a,
                CopyAction::KeepEdited
                    | CopyAction::KeepNotInstalled
                    | CopyAction::KeepRemovedByHand
                    | CopyAction::KeepDroppedEdited
            )
        })
        .map(|(p, _)| base.join(p))
        .collect();
    let mut undo = Undo::new(&name, &shown, &touched)?;
    undo.record = Some((root.to_path_buf(), recorded.clone()));

    let mut lines = Vec::new();
    let mut warns = Vec::new();
    let mut files = recorded.files.clone();
    let applied = (|| -> anyhow::Result<()> {
        for (rel, action) in &plan {
            let shown_rel = rel.display();
            match action {
                CopyAction::Replace | CopyAction::Add => {
                    let new_bytes = &bytes[rel];
                    let digest = digest_of(new_bytes);
                    if disk[rel].as_ref() != Some(&digest) {
                        write(&base, rel, new_bytes, exec.contains(rel), &mut undo.dirs)?;
                        let verb = if *action == CopyAction::Add {
                            "added"
                        } else {
                            "updated"
                        };
                        lines.push(format!("  {verb} .armadai/{shown_rel}"));
                    }
                    files.insert(rel.clone(), digest);
                }
                CopyAction::Remove => {
                    std::fs::remove_file(base.join(rel))?;
                    prune(&base, rel);
                    files.remove(rel);
                    lines.push(format!(
                        "  removed .armadai/{shown_rel} (no longer in the pack)"
                    ));
                }
                CopyAction::MergeManifest => {
                    if let Some((text, lossy)) = merged(&base.join(rel), &bytes[rel])? {
                        if lossy {
                            warns.push(
                                "  warn: .armadai/project.yaml was rewritten in canonical form: \
                                 its comments, key order and inline forms are not kept"
                                    .into(),
                            );
                        }
                        write(&base, rel, text.as_bytes(), false, &mut undo.dirs)?;
                        lines.push(
                            "  merged .armadai/project.yaml (your keys kept, the pack's added)"
                                .into(),
                        );
                    }
                }
                CopyAction::KeepEdited => warns.push(format!(
                    "  warn: .armadai/{shown_rel} was edited since the pack installed it: kept, \
                     the pack's new version is not applied"
                )),
                CopyAction::KeepNotInstalled => warns.push(format!(
                    "  warn: .armadai/{shown_rel} exists and was not installed by the pack: kept, \
                     the pack's version is not applied"
                )),
                CopyAction::KeepRemovedByHand => warns.push(format!(
                    "  warn: .armadai/{shown_rel} was removed by hand: not added back"
                )),
                CopyAction::KeepDroppedEdited => warns.push(format!(
                    "  warn: .armadai/{shown_rel} is no longer in the pack but was edited: kept"
                )),
            }
        }
        Ok(())
    })();
    if let Err(e) = applied {
        undo.restore();
        return Err(e);
    }
    let after = errors_now(root, registry, env);
    if !after.is_empty() {
        for d in &after {
            eprintln!("{}", d.render());
        }
        undo.restore();
        bail!("pack {name} {shown} would make the project invalid; nothing was changed");
    }
    let record = InstalledPack {
        files,
        version: new.version,
        id: new.id,
        ..recorded.clone()
    };
    if let Err(e) = manifest::write_installed_pack(root, &name, record) {
        undo.restore();
        return Err(e).context("cannot record the pack in .armadai/link-manifest.yaml");
    }
    for w in &warns {
        eprintln!("{w}");
    }
    let was = recorded.version.as_deref().unwrap_or("(no version)");
    lines.push(format!("Pack {name}: {was} -> {shown} ({id12})"));
    Ok(Done {
        relink: true,
        lines,
        undo: Some(undo),
    })
}

/// The edited `project.yaml` with the pack's keys added (the user's values win), or `None`
/// when the pack has nothing to add: the file is then left as the user wrote it. The flag
/// says the canonical form loses something of the user's text.
fn merged(disk: &Path, pack: &[u8]) -> anyhow::Result<Option<(String, bool)>> {
    let read = |text: &str, what: &str| {
        armadai_model::yaml::from_str_manifest(text)
            .map_err(|e| anyhow::anyhow!("{what} project.yaml: {e}"))
    };
    let text = std::fs::read_to_string(disk)?;
    let mine = read(&text, "your")?;
    let theirs = read(&String::from_utf8_lossy(pack), "the pack's")?;
    let both = armadai_model::pack::merge_manifest_values(&theirs, &mine);
    if both == mine {
        return Ok(None);
    }
    // the canonical writer takes the typed manifest (a JSON value would write its numbers
    // as maps)
    let typed: armadai_model::Manifest = serde_json::from_value(both)
        .map_err(|e| anyhow::anyhow!("the merged project.yaml is not valid: {e}"))?;
    let written = armadai_model::render_manifest(&typed);
    // lossy: the user's own keys alone, written canonically, would not give their file back
    let own: armadai_model::Manifest =
        serde_json::from_value(mine).map_err(|e| anyhow::anyhow!("your project.yaml: {e}"))?;
    Ok(Some((
        written,
        text != armadai_model::render_manifest(&own),
    )))
}

/// Remove the folders a removed file leaves empty, up to `.armadai/`.
fn prune(base: &Path, rel: &Path) {
    let mut up = base.join(rel);
    while up.pop() && up != base {
        if std::fs::remove_dir(&up).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_users_grant_level_is_not_completed_with_the_packs_rules() {
        let dir = tempfile::tempdir().unwrap();
        let disk = dir.path().join("project.yaml");
        std::fs::write(
            &disk,
            "schema_version: 1\npolicy:\n  grants:\n    g:\n      allow:\n      - Bash(git push *)\n",
        )
        .unwrap();
        let pack = b"schema_version: 1\nname: p\npolicy:\n  grants:\n    g:\n      allow: [\"Bash(git push *)\", \"Bash(rm *)\"]\n";
        let (text, _) = merged(&disk, pack).unwrap().unwrap();
        assert!(
            text.contains("Bash(git push *)") && !text.contains("Bash(rm *)"),
            "{text}"
        );
    }

    use super::*;
    use CopyAction::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    /// One path per action, distinct digests, plus a dropped-and-removed one.
    #[test]
    fn plan_copy_update_table() {
        let d = |s: &str| digest_of(s.as_bytes());
        let recorded = BTreeMap::from([
            (p("agents/unchanged.md"), d("u1")),
            (p("agents/edited.md"), d("e1")),
            (p("agents/by-hand.md"), d("h1")),
            (p("agents/dropped.md"), d("d1")),
            (p("agents/dropped-edited.md"), d("x1")),
            (p("agents/gone.md"), d("g1")),
            (p("project.yaml"), d("m1")),
        ]);
        let disk = BTreeMap::from([
            (p("agents/unchanged.md"), Some(d("u1"))),
            (p("agents/edited.md"), Some(d("e-mine"))),
            (p("agents/by-hand.md"), None),
            (p("agents/new.md"), None),
            (p("agents/own.md"), Some(d("own-mine"))),
            (p("agents/dropped.md"), Some(d("d1"))),
            (p("agents/dropped-edited.md"), Some(d("x-mine"))),
            (p("agents/gone.md"), None),
            (p("project.yaml"), Some(d("m-mine"))),
        ]);
        let incoming = BTreeMap::from([
            (p("agents/unchanged.md"), b"u2".to_vec()),
            (p("agents/edited.md"), b"e2".to_vec()),
            (p("agents/by-hand.md"), b"h2".to_vec()),
            (p("agents/new.md"), b"n2".to_vec()),
            (p("agents/own.md"), b"own2".to_vec()),
            (p("project.yaml"), b"m2".to_vec()),
        ]);
        assert_eq!(
            plan_copy_update(&recorded, &disk, &incoming),
            BTreeMap::from([
                (p("agents/unchanged.md"), Replace),
                (p("agents/edited.md"), KeepEdited),
                (p("agents/by-hand.md"), KeepRemovedByHand),
                (p("agents/new.md"), Add),
                (p("agents/own.md"), KeepNotInstalled),
                (p("agents/dropped.md"), Remove),
                (p("agents/dropped-edited.md"), KeepDroppedEdited),
                (p("project.yaml"), MergeManifest),
            ])
        );
    }

    #[test]
    fn an_unchanged_project_yaml_is_replaced_and_an_edit_equal_to_the_pack_is_not_a_conflict() {
        let d = |s: &str| digest_of(s.as_bytes());
        let recorded = BTreeMap::from([(p("project.yaml"), d("m1")), (p("agents/a.md"), d("a1"))]);
        let disk = BTreeMap::from([
            (p("project.yaml"), Some(d("m1"))),
            (p("agents/a.md"), Some(d("a2"))),
        ]);
        let incoming = BTreeMap::from([
            (p("project.yaml"), b"m2".to_vec()),
            (p("agents/a.md"), b"a2".to_vec()),
        ]);
        assert_eq!(
            plan_copy_update(&recorded, &disk, &incoming),
            BTreeMap::from([(p("project.yaml"), Replace), (p("agents/a.md"), Replace)])
        );
    }

    #[test]
    fn undo_puts_back_the_files_the_folders_and_the_record() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join(".armadai");
        let old = InstalledPack {
            source: "embedded".into(),
            version: Some("1.0.0".into()),
            id: None,
            files: BTreeMap::from([(p("skills/s/SKILL.md"), "sha256:aa".to_string())]),
        };
        let kept = base.join("skills/s/SKILL.md");
        let edited = base.join("agents/a.md");
        std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
        std::fs::create_dir_all(edited.parent().unwrap()).unwrap();
        std::fs::write(&kept, "S").unwrap();
        std::fs::write(&edited, "A1").unwrap();
        manifest::write_installed_pack(root.path(), "demo", old.clone()).unwrap();
        let mut undo = Undo::new(
            "demo",
            "1.1.0",
            &[kept.clone(), edited.clone(), base.join("flows/new/f.yaml")],
        )
        .unwrap();
        undo.record = Some((root.path().to_path_buf(), old.clone()));
        // what an update does: removes a file and its folder, rewrites one, adds one in a
        // new folder, records another state
        std::fs::remove_file(&kept).unwrap();
        std::fs::remove_dir(kept.parent().unwrap()).unwrap();
        std::fs::write(&edited, "A2").unwrap();
        std::fs::create_dir_all(base.join("flows/new")).unwrap();
        std::fs::write(base.join("flows/new/f.yaml"), "f").unwrap();
        undo.dirs = vec![base.join("flows"), base.join("flows/new")];
        let mut now = old.clone();
        now.version = Some("1.1.0".into());
        manifest::write_installed_pack(root.path(), "demo", now).unwrap();
        undo.restore();
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "S");
        assert_eq!(std::fs::read_to_string(&edited).unwrap(), "A1");
        assert!(!base.join("flows").exists());
        assert_eq!(
            manifest::installed_packs(root.path()).get("demo"),
            Some(&old)
        );
    }
}
