//! The link manifest (`<root>/.armadai/link-manifest.yaml`) — a per-project,
//! per-target registry of what `link` wrote and how to undo it, written at
//! the point of effect (`cli::link::execute`'s single write loop) and
//! consumed by `cli::unlink::execute` instead of re-deriving the same facts
//! by regenerating against the *current* config.
//!
//! See `docs/superpowers/specs/2026-08-24-link-manifest-design.md` for the
//! full design (issue #338's second half); this module implements its §3
//! format and the read/write contract of §5/§6.
//!
//! Not versioned — `.armadai/` is already gitignored in this project, and
//! the manifest describes local machine state, not something to share. A
//! fresh clone or a deleted `.armadai/` therefore has no manifest at all;
//! [`lookup_target`] reports that (and any manifest this build cannot make
//! sense of) as [`Lookup::Fallback`], so the caller can fall back to the
//! #342 content-match guard and say so, per §4.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    /// Relative to the project root — never absolute (design §3): a
    /// manifest must survive the project moving on disk.
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

/// Whether `digest` both carries a prefix this build knows how to verify
/// and matches `content` under that algorithm. A digest with an
/// unrecognised prefix (a value some future build might write) is never a
/// match — that is what lets a caller fall back to the #342 guard for that
/// one entry instead of trusting a comparison it cannot actually perform.
pub fn digest_matches(digest: &str, content: &[u8]) -> bool {
    match digest.strip_prefix("sha256:") {
        Some(_) => digest_of(content) == digest,
        None => false,
    }
}

/// Result of looking up one target's entries in the manifest.
pub enum Lookup {
    /// A usable, possibly empty, set of entries recorded for this target.
    Found(Vec<ManifestEntry>),
    /// No manifest, an unreadable one, one whose `version` this build
    /// doesn't understand, or no entries recorded for this target at all.
    /// The caller has nothing reliable to act on and must fall back to the
    /// #342 content-match guard.
    Fallback,
}

/// Read the manifest (if any) and look up `target`'s entries.
pub fn lookup_target(root: &Path, target: &str) -> Lookup {
    let Some(manifest) = read(root) else {
        return Lookup::Fallback;
    };
    match manifest.targets.get(target) {
        Some(t) => Lookup::Found(t.entries.clone()),
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

/// Replace `target`'s entries with `entries`, leaving every other target's
/// entries in the manifest untouched (design §3/§8: grouped by target, full
/// replacement within one target only — a project linked to two targets
/// keeps both). Creates `.armadai/` and the manifest file if neither exists
/// yet. Never called for `--dry-run` (design §5): a preview writes nothing.
pub fn write_target(root: &Path, target: &str, entries: Vec<ManifestEntry>) -> std::io::Result<()> {
    let mut manifest = read(root).unwrap_or_else(|| Manifest {
        version: MANIFEST_VERSION,
        targets: BTreeMap::new(),
    });
    manifest.version = MANIFEST_VERSION;
    manifest.targets.insert(
        target.to_string(),
        TargetManifest {
            linked_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn digest_matches_recognises_its_own_digest() {
        let d = digest_of(b"content");
        assert!(digest_matches(&d, b"content"));
        assert!(!digest_matches(&d, b"different content"));
    }

    #[test]
    fn digest_matches_rejects_an_unrecognised_prefix() {
        // A hypothetical future algorithm's value must never be treated as
        // a match, even if the hex payload happens to be identical to a
        // sha256 digest of the same content — the prefix, not the payload,
        // gates verifiability.
        let sha = digest_of(b"content");
        let hex_only = sha.strip_prefix("sha256:").unwrap();
        let foreign = format!("fnv1a64:{hex_only}");
        assert!(!digest_matches(&foreign, b"content"));
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
        write_target(dir.path(), "claude", vec![]).unwrap();
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
            "version: 999\ntargets:\n  claude:\n    linked_at: \"now\"\n    entries: []\n",
        )
        .unwrap();
        assert!(matches!(
            lookup_target(dir.path(), "claude"),
            Lookup::Fallback
        ));
    }

    #[test]
    fn write_target_then_lookup_round_trips_entries() {
        let dir = tempfile::tempdir().unwrap();
        let entries = vec![
            ManifestEntry {
                path: PathBuf::from(".claude/agents/member.md"),
                produced_by: ProducedBy::agent("member"),
                outcome: Outcome::Created,
                digest: Some(digest_of(b"content")),
            },
            ManifestEntry {
                path: PathBuf::from(".claude/CLAUDE.md"),
                produced_by: ProducedBy::coordinator("coord"),
                outcome: Outcome::Skipped,
                digest: None,
            },
        ];
        write_target(dir.path(), "claude", entries.clone()).unwrap();

        match lookup_target(dir.path(), "claude") {
            Lookup::Found(found) => assert_eq!(found, entries),
            Lookup::Fallback => panic!("expected Found after write_target"),
        }
    }

    #[test]
    fn write_target_replaces_only_the_given_target() {
        let dir = tempfile::tempdir().unwrap();
        let claude_entries = vec![ManifestEntry {
            path: PathBuf::from(".claude/agents/a.md"),
            produced_by: ProducedBy::agent("a"),
            outcome: Outcome::Created,
            digest: Some(digest_of(b"a")),
        }];
        write_target(dir.path(), "claude", claude_entries.clone()).unwrap();

        let codex_entries = vec![ManifestEntry {
            path: PathBuf::from(".codex/agents/a.toml"),
            produced_by: ProducedBy::agent("a"),
            outcome: Outcome::Created,
            digest: Some(digest_of(b"a-toml")),
        }];
        write_target(dir.path(), "codex", codex_entries.clone()).unwrap();

        // Relinking claude with a different roster must not disturb codex's
        // entries — each target owns its own slice of the manifest.
        let new_claude_entries = vec![ManifestEntry {
            path: PathBuf::from(".claude/agents/b.md"),
            produced_by: ProducedBy::agent("b"),
            outcome: Outcome::Created,
            digest: Some(digest_of(b"b")),
        }];
        write_target(dir.path(), "claude", new_claude_entries.clone()).unwrap();

        match lookup_target(dir.path(), "claude") {
            Lookup::Found(found) => assert_eq!(found, new_claude_entries),
            Lookup::Fallback => panic!("expected Found for claude"),
        }
        match lookup_target(dir.path(), "codex") {
            Lookup::Found(found) => assert_eq!(found, codex_entries),
            Lookup::Fallback => panic!("expected Found for codex, untouched by the claude relink"),
        }
    }
}
