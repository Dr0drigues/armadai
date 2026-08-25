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

/// `Created`: `link` wrote this path, **or** found it already present with
/// exactly the bytes it would have written — a pre-existing file that
/// happens to be byte-identical to `link`'s own output is `link`'s to
/// reclaim, not a stranger's to leave alone (design §12 R6: without this,
/// `link ; link ; unlink` would remove nothing at all). `Skipped`: the
/// path existed with *different* content, and `link` left it untouched.
///
/// The inverse is derived from this and never stored (design §3):
/// `Skipped`'s inverse is "do nothing" — `link` produced nothing at this
/// path, so `unlink` has nothing of its own to undo there. `Created`'s
/// inverse is "delete, but only if the digest still matches".
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
/// prefix, then re-append whatever suffix doesn't exist yet, lexically —
/// there is nothing on disk to resolve for a path that isn't there, and
/// that lexical append happens for *every* call whose `field` has any
/// missing tail component, not just as a rare fallback. `project_root`
/// existing or not has no bearing on it either way — it almost always
/// exists.
///
/// The pure-lexical [`resolve`] fallback below is narrower than that: it
/// only fires when canonicalising the longest existing prefix this
/// function actually found still fails (a permission error, for
/// instance) — never merely because some suffix of `field` doesn't exist
/// yet.
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

/// **Which side is at fault** when a manifest-recorded path is refused: the
/// manifest's own text, or the filesystem it is being resolved against.
/// Issue #348: these two causes were collapsed into a single "the manifest
/// may be corrupt or forged" message, which is simply false for the second
/// case — the manifest is exactly what `link` wrote, and the disk moved
/// under it. The two call for different next steps from the user (fix or
/// regenerate the manifest, versus investigate what changed on disk), so
/// callers must keep them distinct.
///
/// Used for both refusal kinds a recorded path can hit, because the
/// question — "is the text wrong, or did the disk move?" — is the same one
/// either way and must be answered the same way:
///
/// - failing [`is_trusted`] (the path lands outside the trusted root), via
///   [`diagnose_trust_failure`];
/// - resolving *onto* the target's own root, via [`decide_created_dir`]'s
///   [`CreatedDirDecision::IsTargetRoot`] — reachable by a pure filesystem
///   mutation with the manifest untouched (replace `.claude/agents` with a
///   symlink to `.claude`), so it must not hardcode either cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustFailure {
    /// The manifest's own text is what puts the path where it must not be:
    /// even under pure lexical resolution (no filesystem access) it
    /// escapes the trusted root, or names that root itself.
    ManifestEscapesRoot,
    /// Under lexical resolution the path is exactly where it should be;
    /// only resolving it against the real filesystem (following symlinks)
    /// puts it outside the root, or onto the root itself — something on
    /// disk changed since `link` ran.
    FilesystemDiverged,
}

/// Which side is at fault, given whether the recorded path's own text is
/// already enough to condemn it: if pure lexical resolution alone lands it
/// where it must not be, the manifest is wrong; if only resolving against
/// the real filesystem does, the disk moved under an intact manifest.
///
/// One function so the two refusal kinds ([`diagnose_trust_failure`] and
/// [`CreatedDirDecision::IsTargetRoot`]) cannot answer it differently —
/// which is exactly the bug issue #348 found in the second one, where the
/// cause was hardcoded to "corrupt or forged".
fn fault_side(text_alone_condemns: bool) -> TrustFailure {
    if text_alone_condemns {
        TrustFailure::ManifestEscapesRoot
    } else {
        TrustFailure::FilesystemDiverged
    }
}

/// Diagnose why `candidate` fails [`is_trusted`] under `target_root` — or
/// `None` if it doesn't. See [`TrustFailure`] for what the two possible
/// causes mean and why callers must not collapse them into one message.
pub fn diagnose_trust_failure(
    project_root: &Path,
    target_root: &Path,
    candidate: &Path,
) -> Option<TrustFailure> {
    if is_trusted(project_root, target_root, candidate) {
        return None;
    }
    let lexical_root = resolve(project_root, target_root);
    let lexical_candidate = resolve(project_root, candidate);
    Some(fault_side(!lexical_candidate.starts_with(&lexical_root)))
}

/// Whether `dir` (a recorded `created_dirs` entry) is a plausible ancestor
/// of at least one `Created` entry — i.e. a directory `link` could
/// plausibly have created while writing one of its own files.
///
/// This is the second half of the `created_dirs` trust boundary (residue
/// of design review R3, issue #348's 4th bullet): [`is_trusted`] alone
/// only bounds *where* a recorded directory may resolve — inside the
/// target's own root — not *whether* it corresponds to anything `link`
/// actually wrote. A forged or hand-corrupted `created_dirs` entry naming
/// a pre-existing, currently-empty directory the user made by hand (e.g.
/// `.claude/notes/`) passes [`is_trusted`] trivially as long as it sits
/// under the target root, and would otherwise be removed on nothing more
/// than "empty and trusted". Requiring it to be an ancestor of at least
/// one `Created` entry ties the removal back to real generated content —
/// [`create_dir_all_recording`] is only ever called immediately before
/// writing a file that becomes a `Created` entry (see [`write_files`]), so
/// every legitimate `created_dirs` entry satisfies this by construction.
/// A `Skipped` entry never causes a directory to be created, so it never
/// counts here either.
pub fn created_dir_is_plausible(dir: &Path, entries: &[ManifestEntry]) -> bool {
    entries
        .iter()
        .any(|e| e.outcome == Outcome::Created && e.path.starts_with(dir))
}

/// A recorded `created_dirs` entry's fate — decided once by
/// [`decide_created_dir`] and shared by both `unlink`'s real pass and its
/// `--dry-run` preview, so the two can never silently diverge on this
/// decision again (issue #348's 1st bullet: they once did — the dry run
/// announced a directory safe to clean up that the real pass would have
/// refused).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatedDirDecision {
    /// Safe to remove, once actually confirmed empty on disk.
    Eligible,
    /// Refused: resolves onto the target's own root — `link` never
    /// records that (design review R3). Carries which side put it there
    /// (issue #348): the manifest's text names the root outright, or the
    /// disk has since merged a legitimately recorded subdirectory into it
    /// (a symlink appeared). The second is reachable with the manifest
    /// completely untouched, so this must never be reported as "corrupt
    /// or forged" unconditionally.
    IsTargetRoot(TrustFailure),
    /// Refused: fails the trust boundary — see [`TrustFailure`] for why.
    Untrusted(TrustFailure),
    /// Refused: doesn't correspond to any file `link` actually recorded
    /// creating — see [`created_dir_is_plausible`].
    Implausible,
}

/// The order `created_dirs` must be walked in: deepest first, so a child
/// directory is always removed before the parent whose emptiness its own
/// removal is what makes possible.
///
/// Extracted (issue #348) because `unlink`'s real pass sorted and its
/// `--dry-run` preview did not, so the two listed the same recorded
/// directories in different orders — a preview whose output doesn't match
/// the run it previews. One function, called by both.
pub fn deepest_first(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut sorted = dirs.to_vec();
    sorted.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    sorted
}

/// Decide `dir`'s fate against `target_root`'s trust boundary and
/// `entries`' recorded `Created` paths. Pure and side-effect free — the
/// caller does the actual filesystem check (still empty? still a
/// directory?) only for the [`CreatedDirDecision::Eligible`] case.
pub fn decide_created_dir(
    project_root: &Path,
    target_root: &Path,
    dir: &Path,
    entries: &[ManifestEntry],
) -> CreatedDirDecision {
    let resolved_target_root = resolve_real(project_root, target_root);
    let dir_path = resolve_real(project_root, dir);
    if dir_path == resolved_target_root {
        // Same lexical-versus-real split `diagnose_trust_failure` makes,
        // for the same reason (issue #348): if the recorded text already
        // names the root, the manifest is wrong; if only the resolved
        // paths coincide, the manifest is intact and the filesystem
        // merged the two.
        return CreatedDirDecision::IsTargetRoot(fault_side(
            resolve(project_root, dir) == resolve(project_root, target_root),
        ));
    }
    if let Some(cause) = diagnose_trust_failure(project_root, target_root, dir) {
        return CreatedDirDecision::Untrusted(cause);
    }
    if !created_dir_is_plausible(dir, entries) {
        return CreatedDirDecision::Implausible;
    }
    CreatedDirDecision::Eligible
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

/// One decision made while writing a target's linked files, as data rather
/// than a side-effecting print call — so every caller can format it
/// however it likes (`link`'s styled `anstream` lines, the shell wizard's
/// plain ones) without re-deriving what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOutcome {
    /// Wrote (or, with `force`, overwrote) `path`.
    Wrote(PathBuf),
    /// `path` already existed with exactly the bytes this write would have
    /// produced — nothing changed on disk, but the manifest still records
    /// it as [`Outcome::Created`] (see that type's doc: `link; link;
    /// unlink` must still remove it).
    UpToDate(PathBuf),
    /// `path` already existed with *different* content and `force` was not
    /// set — left untouched, recorded as [`Outcome::Skipped`].
    SkippedExisting(PathBuf),
}

/// Write every `(path, content, produced_by)` tuple in `files` under
/// `root`, and record a link manifest entry for each one at the point of
/// effect — this is `link`'s actual write path (the loop that used to sit
/// inline in `cli::link::execute`, lines 333-441 before this function
/// existed), now the **only** place in the codebase that writes linker
/// output to disk.
///
/// Two guarantees come from going through this function instead of a
/// second hand-rolled loop — exactly the two issue #347 measured missing
/// from the shell wizard's own copy:
///
/// - **The exists-guard**: a pre-existing file whose content differs from
///   what would be written is left untouched unless `force` is set — the
///   same rule `link` has always had (`link.rs:295` before the extraction).
/// - **The manifest write**: every decision (`Wrote`/`UpToDate`/
///   `SkippedExisting`) becomes a [`ManifestEntry`] via [`write_target`],
///   so `unlink` can act on exactly what happened here instead of
///   re-deriving it later by regenerating against the *current* config
///   (the #342 fallback's blind spots — see `cli::unlink`'s module doc).
///
/// `target_root` is the target's own resolved root (e.g.
/// `<project>/.claude`, or a custom `--output`) — recorded into the
/// manifest via [`write_target`] and used here to decide which created
/// ancestor directories are eligible to be recorded as `created_dirs`
/// (never the root itself, never anything above it: `unlink` must never
/// remove `.claude/` itself, see [`create_dir_all_recording`]'s caller
/// discipline).
pub fn write_files(
    root: &Path,
    target_name: &str,
    output_dir: &Path,
    target_root: &Path,
    files: Vec<(PathBuf, String, ProducedBy)>,
    force: bool,
) -> std::io::Result<Vec<FileOutcome>> {
    // The previous manifest write for this exact target, keyed by path —
    // used only to recognise a file that is still exactly what `link`
    // itself wrote last time, even when the freshly generated content has
    // since changed (issue #348, coordination note 2): without this, such
    // a file gets downgraded to `Skipped`/no digest below purely because
    // the *upstream* agent source changed, not because anyone touched the
    // file — mislabelling `link`'s own untouched output as hand-written,
    // permanently (nothing short of a `--force` relink ever corrects it).
    let previous_digests: BTreeMap<PathBuf, String> = match lookup_target(root, target_name) {
        Lookup::Found(t) => t
            .entries
            .into_iter()
            .filter(|e| e.outcome == Outcome::Created)
            .filter_map(|e| e.digest.map(|d| (e.path, d)))
            .collect(),
        Lookup::Fallback => BTreeMap::new(),
    };

    let mut outcomes = Vec::with_capacity(files.len());
    let mut manifest_entries: Vec<ManifestEntry> = Vec::with_capacity(files.len());
    let mut created_dirs: Vec<PathBuf> = Vec::new();

    for (path, content, produced_by) in files {
        let relative_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();

        if path.exists() && !force {
            // A pre-existing file whose bytes are *already* exactly what
            // this write would produce is this write's to reclaim, not a
            // stranger's to leave alone (design review R6 — see
            // `Outcome`'s own doc).
            let matches_expected = std::fs::read(&path)
                .map(|actual| actual == content.as_bytes())
                .unwrap_or(false);
            if matches_expected {
                manifest_entries.push(ManifestEntry {
                    path: relative_path,
                    produced_by,
                    outcome: Outcome::Created,
                    digest: Some(digest_of(content.as_bytes())),
                });
                outcomes.push(FileOutcome::UpToDate(path));
                continue;
            }

            // The freshly generated content differs from what's on disk —
            // but if the on-disk bytes are still exactly what `link`
            // itself wrote last time (per the previous manifest), this is
            // still `link`'s own file, just stale relative to an upstream
            // change, not a stranger's to leave mislabelled forever.
            let still_own_output = previous_digests
                .get(&relative_path)
                .is_some_and(|old_digest| check_digest(old_digest, &path) == DigestCheck::Matches);
            if still_own_output {
                manifest_entries.push(ManifestEntry {
                    path: relative_path.clone(),
                    produced_by,
                    outcome: Outcome::Created,
                    digest: previous_digests.get(&relative_path).cloned(),
                });
                outcomes.push(FileOutcome::SkippedExisting(path));
                continue;
            }

            manifest_entries.push(ManifestEntry {
                path: relative_path,
                produced_by,
                outcome: Outcome::Skipped,
                digest: None,
            });
            outcomes.push(FileOutcome::SkippedExisting(path));
            continue;
        }

        if let Some(parent) = path.parent() {
            for created in create_dir_all_recording(parent)? {
                // Never record the target's own root, or anything above
                // it — only its descendants are ever eligible for
                // `unlink` to remove later.
                if created.starts_with(target_root) && created != target_root {
                    created_dirs.push(created.strip_prefix(root).unwrap_or(&created).to_path_buf());
                }
            }
        }
        std::fs::write(&path, &content)?;
        // `Outcome::Created` regardless of whether `path` pre-existed —
        // this is `Created` in the sense of "this write produced it", not
        // "the path was new", which is the fact `unlink` actually needs: it
        // must delete this on a matching digest either way. A pre-existing
        // file reaches this branch only via `force`, i.e. the same
        // explicit confirmation that let the write overwrite it in the
        // first place.
        manifest_entries.push(ManifestEntry {
            path: relative_path,
            produced_by,
            outcome: Outcome::Created,
            digest: Some(digest_of(content.as_bytes())),
        });
        outcomes.push(FileOutcome::Wrote(path));
    }

    if let Err(e) = write_target(
        root,
        target_name,
        output_dir.to_path_buf(),
        created_dirs,
        manifest_entries,
    ) {
        // Deliberately non-fatal: every file above was already written (or
        // correctly left alone); refusing to report success over a
        // manifest write failure (a permissions issue on `.armadai/`, a
        // full disk, ...) would be a worse outcome than a degraded
        // `unlink` next time. `unlink` falls back to the #342 guard and
        // says so if this manifest never lands.
        tracing::warn!("Failed to write link manifest: {:?}", e);
    }

    Ok(outcomes)
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

    // ── write_files (issue #347's shared write path) ──────────────────

    /// A fresh file is written, reported as `Wrote`, and gets a manifest
    /// entry recorded as `created` with a digest of its actual content.
    ///
    /// Mutation this catches: if the manifest write were dropped from
    /// this function (the exact defect #347 measured in the shell
    /// wizard's independent copy of this loop), `lookup_target` below
    /// would return `Fallback` instead of `Found`.
    #[test]
    fn write_files_writes_a_fresh_file_and_records_it_created() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target_root = root.join(".claude");
        let path = target_root.join("agents/solo.md");
        let content = "hello".to_string();

        let outcomes = write_files(
            root,
            "claude",
            Path::new(".claude"),
            &target_root,
            vec![(path.clone(), content.clone(), ProducedBy::agent("solo"))],
            false,
        )
        .unwrap();

        assert_eq!(outcomes, vec![FileOutcome::Wrote(path.clone())]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);

        match lookup_target(root, "claude") {
            Lookup::Found(found) => {
                assert_eq!(found.entries.len(), 1);
                assert_eq!(found.entries[0].outcome, Outcome::Created);
                assert_eq!(found.entries[0].digest, Some(digest_of(content.as_bytes())));
            }
            Lookup::Fallback => panic!("write_files must leave a usable manifest behind"),
        }
    }

    /// A pre-existing file whose content already matches is reported
    /// `UpToDate` — left alone on disk — but still recorded `created` in
    /// the manifest (design review R6: `link; link; unlink` must still
    /// remove it), **with a digest** — `unlink`'s `Created` branch reads
    /// `entry.digest` unconditionally (`check_digest`, called with
    /// `entry.digest.as_deref()`), so a `Created` entry with `digest: None`
    /// would make `unlink` report every such file "cannot be verified —
    /// unreadable, or an unrecognised digest algorithm" and keep it
    /// forever, never actually comparing content at all (M2, independent
    /// review of #347: this exact assertion was missing, so a mutant
    /// hard-coding `digest: None` in this branch survived undetected).
    ///
    /// Mutation this catches: if a byte-identical pre-existing file were
    /// recorded `Skipped` instead of `Created` (the pre-#338 behaviour),
    /// the `outcome` assertion fails; if `digest` were `None` or wrong
    /// (M2's own mutant) instead of the actual content's digest, the
    /// `digest` assertion fails.
    #[test]
    fn write_files_reports_up_to_date_but_still_records_created() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target_root = root.join(".claude");
        let path = target_root.join("agents/solo.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "hello").unwrap();

        let outcomes = write_files(
            root,
            "claude",
            Path::new(".claude"),
            &target_root,
            vec![(path.clone(), "hello".to_string(), ProducedBy::agent("solo"))],
            false,
        )
        .unwrap();

        assert_eq!(outcomes, vec![FileOutcome::UpToDate(path)]);
        match lookup_target(root, "claude") {
            Lookup::Found(found) => {
                assert_eq!(found.entries[0].outcome, Outcome::Created);
                assert_eq!(
                    found.entries[0].digest,
                    Some(digest_of(b"hello")),
                    "a Created entry must carry the digest of the content actually on \
                     disk, or unlink's digest check has nothing to compare against"
                );
            }
            Lookup::Fallback => panic!("write_files must leave a usable manifest behind"),
        }
    }

    /// The exists-guard (issue #347's second gap): a pre-existing file
    /// whose content differs is left completely untouched when `force` is
    /// false, and recorded `Skipped` — never `Created` — so `unlink` never
    /// treats it as its own to remove.
    ///
    /// Mutation this catches: if the guard were removed (or `force`
    /// defaulted to `true`), the hand-written content would be
    /// overwritten — this test's content assertion would fail.
    #[test]
    fn write_files_refuses_to_overwrite_a_hand_written_file_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target_root = root.join(".claude");
        let path = target_root.join("agents/solo.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let hand_written = "# written by a human\n";
        std::fs::write(&path, hand_written).unwrap();

        let outcomes = write_files(
            root,
            "claude",
            Path::new(".claude"),
            &target_root,
            vec![(
                path.clone(),
                "# generated content\n".to_string(),
                ProducedBy::agent("solo"),
            )],
            false,
        )
        .unwrap();

        assert_eq!(outcomes, vec![FileOutcome::SkippedExisting(path.clone())]);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            hand_written,
            "a hand-written file must never be touched without force"
        );
        match lookup_target(root, "claude") {
            Lookup::Found(found) => {
                assert_eq!(found.entries[0].outcome, Outcome::Skipped);
                assert_eq!(found.entries[0].digest, None);
            }
            Lookup::Fallback => panic!("write_files must leave a usable manifest behind"),
        }
    }

    /// The other half of the guard: `force: true` does overwrite a
    /// differing pre-existing file, and records it `Created` — the same
    /// explicit confirmation `link --force` has always required.
    ///
    /// Mutation this catches: if `force` were ignored (the guard always
    /// applied), this test's content assertion would fail — the file
    /// would still hold the old hand-written text.
    #[test]
    fn write_files_overwrites_with_force() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target_root = root.join(".claude");
        let path = target_root.join("agents/solo.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "old content").unwrap();

        let new_content = "new content".to_string();
        let outcomes = write_files(
            root,
            "claude",
            Path::new(".claude"),
            &target_root,
            vec![(path.clone(), new_content.clone(), ProducedBy::agent("solo"))],
            true,
        )
        .unwrap();

        assert_eq!(outcomes, vec![FileOutcome::Wrote(path.clone())]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), new_content);
        match lookup_target(root, "claude") {
            Lookup::Found(found) => assert_eq!(found.entries[0].outcome, Outcome::Created),
            Lookup::Fallback => panic!("write_files must leave a usable manifest behind"),
        }
    }

    /// Issue #348, coordination note 2: a re-`link` that leaves a file
    /// untouched (no `--force`, because the *newly generated* content
    /// differs from what's on disk) must not mislabel that file `Skipped`
    /// in the manifest when the on-disk bytes are still exactly what
    /// `link` itself wrote last time — only the upstream source changed,
    /// not the file.
    ///
    /// Mutation this catches: if the previous-digest check were removed
    /// (or always returned `false`), the second write's entry would come
    /// back `Skipped`/`None` instead of `Created` with the digest of
    /// what's actually on disk — this test's outcome/digest assertions
    /// would fail.
    #[test]
    fn write_files_relink_of_untouched_own_output_stays_created_even_when_content_changed_upstream()
    {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target_root = root.join(".claude");
        let path = target_root.join("agents/solo.md");

        // First link: writes "v1".
        write_files(
            root,
            "claude",
            Path::new(".claude"),
            &target_root,
            vec![(path.clone(), "v1".to_string(), ProducedBy::agent("solo"))],
            false,
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "v1");

        // Second link: the agent's source changed upstream, so the
        // *newly generated* content is "v2" — but the file on disk is
        // still exactly "v1", untouched by anyone since the first link
        // wrote it.
        let outcomes = write_files(
            root,
            "claude",
            Path::new(".claude"),
            &target_root,
            vec![(path.clone(), "v2".to_string(), ProducedBy::agent("solo"))],
            false,
        )
        .unwrap();

        // Conservative file-system behaviour is unchanged: without
        // --force, the file itself is never overwritten.
        assert_eq!(outcomes, vec![FileOutcome::SkippedExisting(path.clone())]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "v1");

        match lookup_target(root, "claude") {
            Lookup::Found(found) => {
                assert_eq!(
                    found.entries[0].outcome,
                    Outcome::Created,
                    "a file that is still exactly what link wrote last time must stay \
                     attributed to link, not be downgraded to hand-written just \
                     because the upstream source has since changed"
                );
                assert_eq!(
                    found.entries[0].digest,
                    Some(digest_of(b"v1")),
                    "the recorded digest must match what's actually on disk, so a \
                     later unlink can still reclaim this file"
                );
            }
            Lookup::Fallback => panic!("write_files must leave a usable manifest behind"),
        }
    }

    // ── resolve_real (issue #348: no direct unit tests before this) ───

    /// The defining property `resolve_real` exists for: it follows a
    /// symlink for the part of the path that actually exists on disk,
    /// where a purely lexical join would just stop at the symlink's own
    /// (lexically valid) path.
    ///
    /// Mutation this catches: if `resolve_real` were replaced by the
    /// pure-lexical `resolve` fallback unconditionally (dropping the
    /// `canonicalize` call entirely), the equality assertion against the
    /// real, canonicalised target would fail — the result would instead
    /// equal the un-followed `root.join("link-name/child.md")`.
    #[test]
    #[cfg(unix)]
    fn resolve_real_follows_a_symlinked_existing_path_to_its_real_location() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let real_target = root.join("real-target");
        std::fs::create_dir_all(&real_target).unwrap();
        let link_name = root.join("link-name");
        std::os::unix::fs::symlink(&real_target, &link_name).unwrap();

        let resolved = resolve_real(root, Path::new("link-name/child.md"));
        let expected = std::fs::canonicalize(&real_target)
            .unwrap()
            .join("child.md");
        assert_eq!(
            resolved, expected,
            "resolve_real must follow the symlink to where it actually points"
        );
        assert_ne!(
            resolved,
            root.join("link-name/child.md"),
            "a purely lexical join (never following the symlink) must not be what \
             this function returns — that is exactly the boundary bypass R2 fixed"
        );
    }

    /// The other half of `resolve_real`'s contract: for a tail that
    /// doesn't exist yet, it canonicalises the longest existing prefix and
    /// re-appends the missing suffix lexically, rather than failing or
    /// falling back to a fully lexical resolution.
    ///
    /// Mutation this catches: if the suffix re-append loop dropped a
    /// component, or canonicalised the wrong prefix, this test's equality
    /// assertion against the manually constructed expected path would
    /// fail.
    #[test]
    fn resolve_real_canonicalizes_the_existing_prefix_and_appends_the_missing_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let existing = root.join("existing");
        std::fs::create_dir_all(&existing).unwrap();

        let resolved = resolve_real(root, Path::new("existing/missing/deep/file.md"));
        let expected = std::fs::canonicalize(&existing)
            .unwrap()
            .join("missing")
            .join("deep")
            .join("file.md");
        assert_eq!(resolved, expected);
    }

    /// Issue #338's R4, pinned with its own test for the first time (issue
    /// #348): `a/../b` must resolve to `<root>/b` even when `a` itself
    /// doesn't exist as a real directory to canonicalise through. The OS
    /// cannot traverse into a nonexistent `a` to cancel the following
    /// `..`, so the *whole* literal path fails to resolve via
    /// canonicalisation even though it logically names `<root>/b` — this
    /// is exactly the false-absence trap `resolve_real`'s fallback to pure
    /// lexical resolution (when canonicalising the found prefix itself
    /// fails) exists to avoid.
    ///
    /// Mutation this catches: if the `Err(_) => resolve(project_root,
    /// field)` fallback in `resolve_real` were removed (propagating the
    /// canonicalize error, or returning the unresolved intermediate path
    /// instead), this test's equality assertion would fail.
    #[test]
    fn resolve_real_normalizes_a_parent_escape_through_a_missing_intermediate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Deliberately do NOT create `root/a` — it must never need to
        // exist for this to resolve correctly.
        let resolved = resolve_real(root, Path::new("a/../b"));
        assert_eq!(resolved, root.join("b"));
    }

    // ── root_confirmed (issue #348: no direct unit tests before this) ─

    #[test]
    fn root_confirmed_true_for_textually_identical_roots() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(root_confirmed(
            root,
            Path::new(".claude"),
            Path::new(".claude")
        ));
    }

    /// The property that actually justifies `resolve_real` over a plain
    /// `PathBuf` equality check: two textually different roots that
    /// resolve to the same real directory must still be confirmed as the
    /// same root.
    ///
    /// Mutation this catches: if `root_confirmed` compared the two paths
    /// with plain equality (or `resolve` instead of `resolve_real`)
    /// instead of resolving both through the filesystem first, this
    /// test's `assert!` would fail.
    #[test]
    #[cfg(unix)]
    fn root_confirmed_true_when_the_declared_root_is_reached_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let real_dir = root.join("real-claude");
        std::fs::create_dir_all(&real_dir).unwrap();
        let link_dir = root.join(".claude");
        std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();

        assert!(
            root_confirmed(root, Path::new(".claude"), Path::new("real-claude")),
            "two textually different roots that resolve to the same real directory \
             must be confirmed as the same root"
        );
    }

    /// Mutation this catches: if `root_confirmed` were hardcoded to `true`
    /// (or its equality inverted), this test would fail to catch two
    /// genuinely different directories being confirmed as the same root.
    #[test]
    fn root_confirmed_false_for_genuinely_different_roots() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::create_dir_all(root.join(".codex")).unwrap();
        assert!(!root_confirmed(
            root,
            Path::new(".claude"),
            Path::new(".codex")
        ));
    }

    // ── diagnose_trust_failure (issue #348, 2nd bullet) ────────────────

    /// A path that escapes the trusted root even under pure lexical
    /// resolution — no filesystem access needed to see it — is the
    /// manifest's own text being wrong.
    #[test]
    fn diagnose_trust_failure_reports_manifest_escapes_root_for_a_textual_escape() {
        let project_root = PathBuf::from("/project");
        assert_eq!(
            diagnose_trust_failure(
                &project_root,
                Path::new(".claude"),
                Path::new(".claude/../../outside/victim.txt"),
            ),
            Some(TrustFailure::ManifestEscapesRoot)
        );
    }

    /// A path that stays inside the trusted root textually, but resolves
    /// outside once an intermediate symlink (added after `link` ran) is
    /// followed, must be reported as a filesystem change — not blamed on
    /// the manifest, which never named anything outside its root at all.
    ///
    /// Mutation this catches: if `diagnose_trust_failure` always returned
    /// `ManifestEscapesRoot` for any `is_trusted` failure (the pre-#348
    /// behaviour, collapsed into one cause), this test's equality
    /// assertion would fail.
    #[test]
    #[cfg(unix)]
    fn diagnose_trust_failure_reports_filesystem_diverged_when_only_a_symlink_moved() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target_root = root.join(".claude");
        std::fs::create_dir_all(target_root.join("agents")).unwrap();
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();

        // Text-wise, `.claude/agents/keys.md` stays fully inside `.claude`
        // — no `..`, no absolute escape. Only the *filesystem* changes:
        // `.claude/agents` becomes a symlink to somewhere outside the
        // project.
        std::fs::remove_dir_all(target_root.join("agents")).unwrap();
        std::os::unix::fs::symlink(&outside, target_root.join("agents")).unwrap();

        let cause = diagnose_trust_failure(
            root,
            Path::new(".claude"),
            Path::new(".claude/agents/keys.md"),
        );
        assert_eq!(
            cause,
            Some(TrustFailure::FilesystemDiverged),
            "a legitimate path whose intermediate directory was symlinked away \
             since link ran must not be reported as if the manifest's own text \
             were at fault"
        );
    }

    // ── created_dir_is_plausible (issue #348, 4th bullet / R3 residue) ─

    #[test]
    fn created_dir_is_plausible_true_for_an_ancestor_of_a_created_entry() {
        let entries = vec![entry(".claude/agents/solo.md", "solo", b"x")];
        assert!(created_dir_is_plausible(
            Path::new(".claude/agents"),
            &entries
        ));
    }

    /// The exact residue issue #348 names: a directory with no matching
    /// `Created` entry at all — e.g. a pre-existing, empty user directory
    /// a forged manifest merely claims to have created — must not be
    /// considered plausible.
    #[test]
    fn created_dir_is_plausible_false_for_a_directory_with_no_matching_entry() {
        let entries = vec![entry(".claude/agents/solo.md", "solo", b"x")];
        assert!(!created_dir_is_plausible(
            Path::new(".claude/my-own-empty-dir"),
            &entries
        ));
    }

    /// A `Skipped` entry never caused `create_dir_all` to run (`link`
    /// only creates directories immediately before an actual write), so a
    /// directory must not be considered plausible on the strength of a
    /// `Skipped` entry living under it alone.
    #[test]
    fn created_dir_is_plausible_false_when_the_only_matching_entry_is_skipped() {
        let entries = vec![ManifestEntry {
            path: PathBuf::from(".claude/notes.md"),
            produced_by: ProducedBy::agent("solo"),
            outcome: Outcome::Skipped,
            digest: None,
        }];
        assert!(!created_dir_is_plausible(Path::new(".claude"), &entries));
    }

    // ── issue #348: the two causes IsTargetRoot can have ──────────────

    /// A manifest that literally records the target's own root is the
    /// manifest's own fault.
    ///
    /// Mutation this catches: hardcode `IsTargetRoot`'s cause to
    /// `FilesystemDiverged` and this fails; its sibling below fails on the
    /// opposite hardcoding, so neither value can be pinned by accident.
    #[test]
    fn decide_created_dir_blames_the_manifest_when_its_text_names_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target_root = root.join(".claude");
        std::fs::create_dir_all(target_root.join("agents")).unwrap();
        let entries = vec![entry(".claude/agents/solo.md", "solo", b"body")];

        assert_eq!(
            decide_created_dir(root, &target_root, Path::new(".claude"), &entries),
            CreatedDirDecision::IsTargetRoot(TrustFailure::ManifestEscapesRoot)
        );
    }

    /// The same arm, reached with the manifest untouched: a recorded
    /// subdirectory that a symlink has since collapsed onto the root. This
    /// is a filesystem change, and `unlink` must not call it a forged
    /// manifest.
    ///
    /// Mutation this catches: hardcode `IsTargetRoot`'s cause to
    /// `ManifestEscapesRoot` — the value the code shipped with — and this
    /// fails.
    #[test]
    #[cfg(unix)]
    fn decide_created_dir_blames_the_filesystem_when_a_symlink_collapsed_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target_root = root.join(".claude");
        std::fs::create_dir_all(&target_root).unwrap();
        // `.claude/agents -> .` resolves to `.claude` itself, while the
        // recorded text `.claude/agents` names a subdirectory.
        std::os::unix::fs::symlink(".", target_root.join("agents")).unwrap();
        let entries = vec![entry(".claude/agents/solo.md", "solo", b"body")];

        assert_eq!(
            decide_created_dir(root, &target_root, Path::new(".claude/agents"), &entries),
            CreatedDirDecision::IsTargetRoot(TrustFailure::FilesystemDiverged)
        );
    }

    // ── issue #348: created_dirs removal order ────────────────────────

    /// `created_dirs` must be walked deepest first, so removing a child is
    /// what makes its parent empty enough to remove in the same pass.
    /// `unlink`'s real pass sorted and its `--dry-run` preview did not, so
    /// the two listed the same directories in different orders; both now
    /// call this.
    ///
    /// Mutation this catches: drop the `Reverse` (or the sort entirely) and
    /// the returned order no longer puts the deepest path first.
    #[test]
    fn deepest_first_orders_children_before_their_parents() {
        let dirs = vec![
            PathBuf::from(".claude"),
            PathBuf::from(".claude/skills/writer/refs"),
            PathBuf::from(".claude/agents"),
            PathBuf::from(".claude/skills/writer"),
        ];
        assert_eq!(
            deepest_first(&dirs),
            vec![
                PathBuf::from(".claude/skills/writer/refs"),
                PathBuf::from(".claude/skills/writer"),
                PathBuf::from(".claude/agents"),
                PathBuf::from(".claude"),
            ]
        );
    }

    /// The counterpart of
    /// `write_files_relink_of_untouched_own_output_stays_created_even_when_content_changed_upstream`:
    /// the previous-digest reclaim added for coordination note 2 must stay
    /// unable to claim a file `link` never wrote. A hand-written file is
    /// recorded `skipped` with no digest, so no digest of it ever enters
    /// the reclaim's map, and repeated `link` runs — with the generated
    /// content changing every time — must leave that verdict alone.
    ///
    /// Mutation this catches: have the differing-content branch record
    /// `Outcome::Created` with the digest of what is on disk instead of
    /// `Outcome::Skipped`/`None`, i.e. let `link` claim whatever it finds
    /// in its way. Every assertion in the loop below fails on the first
    /// pass.
    #[test]
    fn write_files_never_claims_a_hand_written_file_however_often_link_runs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target_root = root.join(".claude");
        let path = target_root.join("agents/solo.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let hand_written = "mine, not link's";
        std::fs::write(&path, hand_written).unwrap();

        for pass in 1..=3 {
            let generated = format!("generated revision {pass}");
            let outcomes = write_files(
                root,
                "claude",
                Path::new(".claude"),
                &target_root,
                vec![(path.clone(), generated, ProducedBy::agent("solo"))],
                false,
            )
            .unwrap();
            assert_eq!(outcomes, vec![FileOutcome::SkippedExisting(path.clone())]);
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                hand_written,
                "pass {pass}: the file itself must be untouched"
            );
            match lookup_target(root, "claude") {
                Lookup::Found(found) => {
                    assert_eq!(
                        found.entries[0].outcome,
                        Outcome::Skipped,
                        "pass {pass}: a file link never wrote stays hand-written"
                    );
                    assert_eq!(
                        found.entries[0].digest, None,
                        "pass {pass}: and carries no digest — a digest is exactly what \
                         would authorise unlink to delete it"
                    );
                }
                Lookup::Fallback => panic!("write_files must leave a usable manifest behind"),
            }
        }
    }
}
