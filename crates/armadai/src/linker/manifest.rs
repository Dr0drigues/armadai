//! The link manifest (`<root>/.armadai/link-manifest.yaml`) — a per-project,
//! per-target registry of what `link` wrote and how to undo it, written at
//! the point of effect (`cli::link::execute`'s single write loop) and
//! consumed by `cli::unlink::execute` instead of re-deriving the same facts
//! by regenerating against the *current* config.
//!
//! See `docs/superpowers/specs/2026-08-24-link-manifest-design.md` for the
//! full design (issue #338's second half); this module implements its §3
//! format and the read/write contract of §5/§6, amended after a security
//! review (see the spec's own amendment section) to add a per-target trust
//! `root` and recorded `created_dirs` — the two facts `unlink` needs to act
//! on a manifest entry without trusting an unvalidated path or guessing an
//! ancestor-cleanup boundary.
//!
//! Not versioned — `.armadai/` is already gitignored in this project, and
//! the manifest describes local machine state, not something to share. A
//! fresh clone or a deleted `.armadai/` therefore has no manifest at all;
//! [`lookup_target`] reports that (and any manifest this build cannot make
//! sense of) as [`Lookup::Fallback`], so the caller can fall back to the
//! #342 content-match guard and say so, per §4.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The only manifest format this build writes or trusts. A manifest whose
/// `version` differs — written by an older or a future `armadai` — is
/// treated exactly like a missing one, per §4: its shape isn't guaranteed to
/// match what this build expects.
const MANIFEST_VERSION: u32 = 1;

fn manifest_path(root: &Path) -> PathBuf {
    root.join(".armadai").join("link-manifest.yaml")
}

/// What produced a manifest entry. `kind` is the stated extension point for
/// the declarative chain the design's §2/§7 anticipates (`Config →
/// Governance → Meta-agents → Rules → Agents → link → native configs`):
/// today it is always `agent`, `coordinator`, `skill` or `prompt`; a later
/// stage adds a value here, not a new field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducedBy {
    pub kind: ProducedByKind,
    pub name: String,
}

impl ProducedBy {
    pub fn agent(name: impl Into<String>) -> Self {
        Self {
            kind: ProducedByKind::Agent,
            name: name.into(),
        }
    }

    pub fn coordinator(name: impl Into<String>) -> Self {
        Self {
            kind: ProducedByKind::Coordinator,
            name: name.into(),
        }
    }

    pub fn skill(name: impl Into<String>) -> Self {
        Self {
            kind: ProducedByKind::Skill,
            name: name.into(),
        }
    }

    pub fn prompt(name: impl Into<String>) -> Self {
        Self {
            kind: ProducedByKind::Prompt,
            name: name.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProducedByKind {
    Agent,
    Coordinator,
    Skill,
    Prompt,
}

/// Whether `link` wrote this path, or found it already there and left it
/// alone. The inverse is derived from this and never stored (design §3): a
/// `Skipped` entry's inverse is "do nothing" — `link` produced nothing at
/// this path, so `unlink` has nothing of its own to undo there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Created,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Relative to the project root when the target's own `root` is
    /// (design §3) — never absolute in that case, so the manifest survives
    /// the project moving on disk. The one documented exception: when a
    /// target's `root` is itself absolute (an absolute `--output`), the
    /// entry's path is absolute too, for the same reason the root is —
    /// see [`TargetManifest::root`].
    pub path: PathBuf,
    pub produced_by: ProducedBy,
    pub outcome: Outcome,
    /// `sha256:<hex>` of the content `link` actually wrote. Present iff
    /// `outcome == Created`. The `sha256:` prefix is load-bearing, not
    /// decorative: it is what lets a build that changes the digest
    /// algorithm recognise a value it cannot reproduce and treat it as
    /// unverifiable instead of comparing incomparable hashes (design §8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetManifest {
    pub linked_at: String,
    /// Where `link` was allowed to write for this target — the resolved
    /// output directory (`.claude` by default, or a custom `--output`,
    /// which may legitimately point outside the project root, e.g.
    /// `../sibling/out`, or be absolute). This is the security boundary a
    /// post-implementation review added: every entry and every
    /// `created_dirs` path is checked against this root (see
    /// [`is_trusted`]) before `unlink` acts on it, so a forged or corrupted
    /// manifest entry naming a path outside the target's own tree is
    /// refused rather than deleted.
    pub root: PathBuf,
    /// Directories `link` itself created (via `create_dir_all`) while
    /// writing this target, in the order [`create_dir_all_recording`]
    /// returns them (each call's own result is deepest-first). This is the
    /// recorded inverse of that one side effect `link` has that isn't a
    /// file write: without it, `unlink` would have to guess which
    /// ancestor directories are safe to remove from the deleted files'
    /// paths alone — which is exactly the bug this field replaces (a
    /// nested `--output` could be removed too eagerly, or `.claude/`
    /// itself could be removed too readily, depending on which way the
    /// guess erred).
    #[serde(default)]
    pub created_dirs: Vec<PathBuf>,
    pub entries: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    #[serde(default)]
    pub targets: BTreeMap<String, TargetManifest>,
}

/// `sha256:<hex>` of `content`.
pub fn digest_of(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("sha256:{:x}", hasher.finalize())
}

/// The result of checking a `Created` entry's digest against what is
/// actually on disk right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestCheck {
    /// The file's bytes hash to exactly the recorded digest.
    Matches,
    /// The file was read successfully and its digest differs — it has
    /// been edited since `link` wrote it.
    Differs,
    /// The file couldn't be read (permissions, a race with an external
    /// delete, ...), or the recorded digest's prefix isn't one this build
    /// knows how to verify (a value some future algorithm might write).
    /// Deliberately distinct from `Differs`: nothing here says the content
    /// actually differs, only that it cannot be confirmed either way —
    /// erring toward keeping the file either way, but for a caller to
    /// report honestly rather than claim "content differs" about
    /// something it never actually compared.
    Unverifiable,
}

/// Check `digest` (a `ManifestEntry::digest`) against the file at `path`.
pub fn check_digest(digest: &str, path: &Path) -> DigestCheck {
    if digest.strip_prefix("sha256:").is_none() {
        return DigestCheck::Unverifiable;
    }
    match std::fs::read(path) {
        Ok(actual) => {
            if digest_of(&actual) == digest {
                DigestCheck::Matches
            } else {
                DigestCheck::Differs
            }
        }
        Err(_) => DigestCheck::Unverifiable,
    }
}

/// Lexically normalise `path` — resolve `.` and `..` components without
/// touching the filesystem. `std::fs::canonicalize` isn't usable for this:
/// a path `unlink` is about to check may no longer exist (a file already
/// deleted this run, or a target that was never actually created), and
/// canonicalisation requires the path to exist. This is not a substitute
/// for `canonicalize` where symlinks matter — it proves containment against
/// the *components* a manifest entry names, which is exactly what's needed
/// to catch a forged `path: ../../etc/passwd` before it's ever compared
/// against a boundary, rather than trusting the OS to refuse it later.
pub fn lexically_normalize(path: &Path) -> PathBuf {
    let mut out: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                // Can't climb above a root/prefix — mirrors what the OS
                // does when resolving an absolute path.
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                _ => out.push(component),
            },
            other => out.push(other),
        }
    }
    out.into_iter().collect()
}

/// Resolve `field` (a target's `root`, an entry's `path`, or a
/// `created_dirs` entry) against `project_root` — absolute as-is, relative
/// joined onto `project_root` — then lexically normalised only, with no
/// filesystem access at all. Kept as the fallback [`resolve_real`] uses
/// when nothing along the path exists yet to canonicalise.
fn resolve(project_root: &Path, field: &Path) -> PathBuf {
    let joined = if field.is_absolute() {
        field.to_path_buf()
    } else {
        project_root.join(field)
    };
    lexically_normalize(&joined)
}

/// Resolve `field` into its real, symlink-free location as far as the
/// filesystem allows *right now*: canonicalise the longest existing
/// prefix, then re-append whatever suffix doesn't exist yet (lexically —
/// there is nothing on disk to resolve for a path that isn't there).
/// Falls back to pure lexical [`resolve`] only when nothing along the
/// path exists at all, including `project_root` itself — a case where
/// there is nothing to delete or list either way.
///
/// This exists because lexical normalisation alone is **not** a security
/// boundary against a symlink, and a review measured exactly that: with
/// `.claude/agents` symlinked to a directory outside the project, the
/// entry `.claude/agents/keys.md` passes a purely lexical containment
/// check against `.claude` — the text of the path never leaves the
/// nominal tree — while the file it actually names lives somewhere else
/// entirely. Resolving symlinks for the part of the path that exists is
/// what closes that; it is why every trust decision in this module goes
/// through this function and not [`resolve`] alone.
pub fn resolve_real(project_root: &Path, field: &Path) -> PathBuf {
    let joined = if field.is_absolute() {
        field.to_path_buf()
    } else {
        project_root.join(field)
    };

    let mut existing = joined.clone();
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(name) => suffix.push(name.to_os_string()),
            None => break,
        }
        match existing.parent() {
            Some(p) => existing = p.to_path_buf(),
            None => break,
        }
    }

    match std::fs::canonicalize(&existing) {
        Ok(mut canon) => {
            for part in suffix.into_iter().rev() {
                canon.push(part);
            }
            canon
        }
        Err(_) => resolve(project_root, field),
    }
}

/// Whether `candidate` (an entry's `path`, or a `created_dirs` path)
/// resolves under `target_root` (that target's own recorded
/// [`TargetManifest::root`]) once both are resolved per [`resolve_real`]
/// — filesystem-aware, so a symlinked intermediate directory is followed
/// to where it actually points before the comparison, not just textually
/// normalised.
///
/// This is the actual security boundary every manifest-driven filesystem
/// action must pass: `unlink` calls this before deleting a file or removing
/// a recorded directory, so a forged entry (`path: ../outside/victim.txt`,
/// an absolute path unrelated to the target, or a path that only escapes
/// via a symlink) is refused rather than acted on — while a legitimate
/// entry under a `root` that itself points outside the project
/// (`--output ../sibling/out`) still passes, because the boundary is the
/// target's own declared root, not the project root. `unlink` additionally
/// confirms that declared root against one it computes independently
/// (see [`root_confirmed`]) before trusting it as a boundary at all — this
/// function alone only proves containment *within whatever root it is
/// given*.
pub fn is_trusted(project_root: &Path, target_root: &Path, candidate: &Path) -> bool {
    let root = resolve_real(project_root, target_root);
    let resolved = resolve_real(project_root, candidate);
    resolved.starts_with(&root)
}

/// Whether a target's declared manifest `root` matches `computed_root` —
/// the same output directory `link`/`unlink` would compute right now from
/// the project's own config/`--output` — once both are resolved per
/// [`resolve_real`].
///
/// [`is_trusted`] alone only proves an entry resolves under whatever
/// `root` the manifest itself claims: a forged `root: /` (or any root wide
/// enough to contain anything) would make every entry pass that check
/// trivially, reproducing the exact defect the trust boundary exists to
/// close. Confirming the declared root against one computed independently
/// — never taken from the manifest — is what actually constrains it: a
/// manifest whose `root` doesn't match the project it sits in is not a
/// manifest for this project, and callers must refuse it wholesale (fall
/// back to the #342 guard) rather than partially trust it.
pub fn root_confirmed(project_root: &Path, computed_root: &Path, declared_root: &Path) -> bool {
    resolve_real(project_root, computed_root) == resolve_real(project_root, declared_root)
}

/// Result of looking up one target's manifest data.
pub enum Lookup {
    /// A usable — possibly empty — target manifest.
    Found(TargetManifest),
    /// No manifest, an unreadable one, one whose `version` this build
    /// doesn't understand, or no key for this target at all (the target
    /// was never linked with a manifest-writing build, or `.armadai/` was
    /// deleted). The caller has nothing reliable to act on and must fall
    /// back to the #342 content-match guard.
    Fallback,
}

/// Read the manifest (if any) and look up `target`'s data.
pub fn lookup_target(root: &Path, target: &str) -> Lookup {
    let Some(manifest) = read(root) else {
        return Lookup::Fallback;
    };
    match manifest.targets.get(target) {
        Some(t) => Lookup::Found(t.clone()),
        None => Lookup::Fallback,
    }
}

/// Read and parse the manifest, if one exists, is well-formed YAML, and
/// declares a `version` this build understands. Anything else — missing
/// file, parse error, unknown version — is `None`: there is no partial
/// manifest to salvage, only a usable one or none at all.
fn read(root: &Path) -> Option<Manifest> {
    let raw = std::fs::read_to_string(manifest_path(root)).ok()?;
    let manifest: Manifest = serde_yaml_ng::from_str(&raw).ok()?;
    if manifest.version != MANIFEST_VERSION {
        return None;
    }
    Some(manifest)
}

/// Replace `target`'s data with `root`/`created_dirs`/`entries`, leaving
/// every other target's data in the manifest untouched (design §3/§8:
/// grouped by target, full replacement within one target only — a project
/// linked to two targets keeps both). Creates `.armadai/` and the manifest
/// file if neither exists yet. Never called for `--dry-run` (design §5): a
/// preview writes nothing.
pub fn write_target(
    root: &Path,
    target: &str,
    target_root: PathBuf,
    created_dirs: Vec<PathBuf>,
    entries: Vec<ManifestEntry>,
) -> std::io::Result<()> {
    let mut manifest = read(root).unwrap_or_else(|| Manifest {
        version: MANIFEST_VERSION,
        targets: BTreeMap::new(),
    });
    manifest.version = MANIFEST_VERSION;
    manifest.targets.insert(
        target.to_string(),
        TargetManifest {
            linked_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            root: target_root,
            created_dirs,
            entries,
        },
    );

    let path = manifest_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let yaml = serde_yaml_ng::to_string(&manifest)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, yaml)
}

/// Create `dir` and any missing ancestors — like `std::fs::create_dir_all`
/// — but also return exactly the directories that did not already exist,
/// deepest first (`dir` itself, then its parent if that was also missing,
/// and so on up to the first ancestor that already existed).
///
/// This is `link`'s side of the fix for the ancestor-cleanup boundary bug a
/// review found: `create_dir_all` was an effect `link` recorded nothing
/// about, so `unlink` had to *guess* which directories it was safe to
/// remove afterwards from the deleted files' paths alone — and guessed
/// wrong for a nested `--output`. Recording exactly what was created lets
/// `unlink` reverse exactly that, nothing more, nothing less.
pub fn create_dir_all_recording(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut missing = Vec::new();
    let mut current = dir.to_path_buf();
    loop {
        if current.exists() {
            break;
        }
        missing.push(current.clone());
        match current.parent() {
            Some(p) => current = p.to_path_buf(),
            None => break,
        }
    }
    std::fs::create_dir_all(dir)?;
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, agent: &str, content: &[u8]) -> ManifestEntry {
        ManifestEntry {
            path: PathBuf::from(path),
            produced_by: ProducedBy::agent(agent),
            outcome: Outcome::Created,
            digest: Some(digest_of(content)),
        }
    }

    #[test]
    fn digest_of_is_prefixed_and_deterministic() {
        let a = digest_of(b"hello");
        let b = digest_of(b"hello");
        assert_eq!(a, b);
        assert!(a.starts_with("sha256:"));
        // 64 hex chars after the prefix.
        assert_eq!(a.len(), "sha256:".len() + 64);
    }

    #[test]
    fn digest_of_differs_on_different_content() {
        assert_ne!(digest_of(b"hello"), digest_of(b"world"));
    }

    #[test]
    fn check_digest_matches_the_actual_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.md");
        std::fs::write(&file, b"content").unwrap();
        let d = digest_of(b"content");
        assert_eq!(check_digest(&d, &file), DigestCheck::Matches);
    }

    #[test]
    fn check_digest_differs_when_content_was_edited() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.md");
        std::fs::write(&file, b"edited").unwrap();
        let d = digest_of(b"content");
        assert_eq!(check_digest(&d, &file), DigestCheck::Differs);
    }

    #[test]
    fn check_digest_is_unverifiable_for_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("does-not-exist.md");
        let d = digest_of(b"content");
        assert_eq!(check_digest(&d, &file), DigestCheck::Unverifiable);
    }

    #[test]
    fn check_digest_is_unverifiable_for_an_unrecognised_prefix() {
        // A hypothetical future algorithm's value must never be treated as
        // a match — or as a confirmed mismatch — even if the hex payload
        // happens to equal a sha256 digest of the same content: the
        // prefix, not the payload, gates verifiability.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.md");
        std::fs::write(&file, b"content").unwrap();
        let sha = digest_of(b"content");
        let hex_only = sha.strip_prefix("sha256:").unwrap();
        let foreign = format!("fnv1a64:{hex_only}");
        assert_eq!(check_digest(&foreign, &file), DigestCheck::Unverifiable);
    }

    #[test]
    fn lexically_normalize_resolves_dot_and_dot_dot_without_touching_disk() {
        assert_eq!(
            lexically_normalize(Path::new("a/b/../c")),
            PathBuf::from("a/c")
        );
        assert_eq!(
            lexically_normalize(Path::new("./a/./b")),
            PathBuf::from("a/b")
        );
        // A leading `..` with nothing to cancel stays — it's a real
        // upward escape, not noise to discard.
        assert_eq!(
            lexically_normalize(Path::new("../a")),
            PathBuf::from("../a")
        );
        assert_eq!(
            lexically_normalize(Path::new("a/../../b")),
            PathBuf::from("../b")
        );
    }

    #[test]
    fn is_trusted_accepts_a_path_under_the_targets_root() {
        let project_root = PathBuf::from("/project");
        assert!(is_trusted(
            &project_root,
            Path::new(".claude"),
            Path::new(".claude/agents/solo.md"),
        ));
    }

    #[test]
    fn is_trusted_rejects_a_forged_parent_escape() {
        let project_root = PathBuf::from("/project");
        // Textually starts with `.claude/...` but climbs out via `..`
        // before coming back in — exactly the kind of entry a hand-edited
        // or forged manifest could contain.
        assert!(!is_trusted(
            &project_root,
            Path::new(".claude"),
            Path::new(".claude/../../outside/victim.txt"),
        ));
    }

    #[test]
    fn is_trusted_rejects_an_absolute_path_outside_the_root() {
        let project_root = PathBuf::from("/project");
        assert!(!is_trusted(
            &project_root,
            Path::new(".claude"),
            Path::new("/etc/passwd"),
        ));
    }

    #[test]
    fn is_trusted_accepts_a_legitimate_output_dir_outside_the_project_root() {
        // `--output ../sibling/out` is a legitimate use of the flag — the
        // trust boundary is the target's own declared root, not the
        // project root, so entries under it must still be accepted.
        let project_root = PathBuf::from("/work/project");
        assert!(is_trusted(
            &project_root,
            Path::new("../sibling/out"),
            Path::new("../sibling/out/agents/solo.md"),
        ));
    }

    #[test]
    fn is_trusted_rejects_escaping_past_a_relative_root_that_itself_climbs_out() {
        let project_root = PathBuf::from("/work/project");
        assert!(!is_trusted(
            &project_root,
            Path::new("../sibling/out"),
            Path::new("../sibling/../../elsewhere/victim.txt"),
        ));
    }

    #[test]
    fn create_dir_all_recording_reports_only_the_newly_created_ancestors() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing");
        std::fs::create_dir_all(&existing).unwrap();

        let nested = existing.join("a").join("b");
        let created = create_dir_all_recording(&nested).unwrap();

        assert!(nested.is_dir());
        // `existing` pre-existed and must not be reported.
        assert!(!created.contains(&existing));
        assert!(created.contains(&nested));
        assert!(created.contains(&existing.join("a")));
        assert_eq!(created.len(), 2);
    }

    #[test]
    fn create_dir_all_recording_reports_nothing_when_the_dir_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let created = create_dir_all_recording(dir.path()).unwrap();
        assert!(created.is_empty());
    }

    #[test]
    fn lookup_target_falls_back_when_no_manifest_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            lookup_target(dir.path(), "claude"),
            Lookup::Fallback
        ));
    }

    #[test]
    fn lookup_target_falls_back_when_target_absent_from_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write_target(
            dir.path(),
            "claude",
            PathBuf::from(".claude"),
            vec![],
            vec![],
        )
        .unwrap();
        assert!(matches!(
            lookup_target(dir.path(), "codex"),
            Lookup::Fallback
        ));
    }

    #[test]
    fn lookup_target_falls_back_on_unknown_version() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            manifest_path(dir.path()),
            "version: 999\ntargets:\n  claude:\n    linked_at: \"now\"\n    root: .claude\n    entries: []\n",
        )
        .unwrap();
        assert!(matches!(
            lookup_target(dir.path(), "claude"),
            Lookup::Fallback
        ));
    }

    #[test]
    fn write_target_then_lookup_round_trips_entries_root_and_created_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let entries = vec![
            entry(".claude/agents/member.md", "member", b"content"),
            ManifestEntry {
                path: PathBuf::from(".claude/CLAUDE.md"),
                produced_by: ProducedBy::coordinator("coord"),
                outcome: Outcome::Skipped,
                digest: None,
            },
        ];
        let created_dirs = vec![PathBuf::from(".claude/agents")];
        write_target(
            dir.path(),
            "claude",
            PathBuf::from(".claude"),
            created_dirs.clone(),
            entries.clone(),
        )
        .unwrap();

        match lookup_target(dir.path(), "claude") {
            Lookup::Found(found) => {
                assert_eq!(found.entries, entries);
                assert_eq!(found.root, PathBuf::from(".claude"));
                assert_eq!(found.created_dirs, created_dirs);
            }
            Lookup::Fallback => panic!("expected Found after write_target"),
        }
    }

    #[test]
    fn write_target_replaces_only_the_given_target() {
        let dir = tempfile::tempdir().unwrap();
        let claude_entries = vec![entry(".claude/agents/a.md", "a", b"a")];
        write_target(
            dir.path(),
            "claude",
            PathBuf::from(".claude"),
            vec![],
            claude_entries,
        )
        .unwrap();

        let codex_entries = vec![entry(".codex/agents/a.toml", "a", b"a-toml")];
        write_target(
            dir.path(),
            "codex",
            PathBuf::from(".codex"),
            vec![],
            codex_entries.clone(),
        )
        .unwrap();

        // Relinking claude with a different roster must not disturb codex's
        // entries — each target owns its own slice of the manifest.
        let new_claude_entries = vec![entry(".claude/agents/b.md", "b", b"b")];
        write_target(
            dir.path(),
            "claude",
            PathBuf::from(".claude"),
            vec![],
            new_claude_entries.clone(),
        )
        .unwrap();

        match lookup_target(dir.path(), "claude") {
            Lookup::Found(found) => assert_eq!(found.entries, new_claude_entries),
            Lookup::Fallback => panic!("expected Found for claude"),
        }
        match lookup_target(dir.path(), "codex") {
            Lookup::Found(found) => assert_eq!(found.entries, codex_entries),
            Lookup::Fallback => panic!("expected Found for codex, untouched by the claude relink"),
        }
    }
}
