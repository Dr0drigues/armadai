//! Family `R` — rightsizing: the cost of the context loaded by default.
//!
//! Where the `A` rules judge one asset's own quality, `R` asks what every
//! invocation pays for. Same shape as every other family: each rule is a pure
//! function of [`AuditContext`], and the reverse pass is what read the disk.

use std::path::Path;

use super::{AuditContext, Finding, Severity};

/// R01 — a `SKILL.md` past the token threshold whose skill directory has no
/// `references/` at all.
///
/// Size alone is a bad signal, measured: across 460 real skills, 46% of those
/// above the p90 have a `references/` directory against 67% below it — large
/// skills are *more* often split. So both conditions are required, which is
/// also what keeps a correctly-structured 41795-word skill out of the report.
///
/// Counterpart of `A05` for skills (`A05` covers agents' `system_prompt`).
/// `A09` validates a skill's structure but never its size.
pub(super) fn r01_oversized_skill(ctx: &AuditContext) -> Vec<Finding> {
    ctx.config
        .skills
        .iter()
        // Anti-cascade, same as A05: a skill that failed to parse is A01/A09's
        // job — one root cause, one finding. No SKILL.md is A09's too, and
        // `source_path` then points at the directory, so a size claim about it
        // would be meaningless.
        .filter(|s| s.issues.is_empty() && s.has_skill_md)
        .filter(|s| s.body_tokens > ctx.settings.skill_token_threshold)
        .filter(|s| !has_references(&s.source_path))
        .map(|s| Finding {
            rule: "R01",
            severity: Severity::Warning,
            file: s.source_path.clone(),
            related: Vec::new(),
            message: format!(
                "skill '{}' is ~{} tokens (threshold {}) and has no references/, so all of it \
                 loads on every invocation",
                s.name, s.body_tokens, ctx.settings.skill_token_threshold
            ),
            suggestion: Some(
                "split the detail into references/ — the Agent Skills standard loads those on \
                 demand, and armadai already installs them (core/skill.rs)"
                    .to_string(),
            ),
        })
        .collect()
}

/// Whether the skill directory holding this `SKILL.md` has a non-empty
/// `references/` **directory**. An empty one is not progressive disclosure,
/// and neither is a plain file that happens to be named `references`:
/// `read_dir` fails on both, which is exactly the answer we want.
///
/// The only filesystem read any *rule* makes (the other one in this tree is
/// `AuditSettings::from_project` loading config), and unavoidable: "is this
/// skill split" is a question about the disk, and the reverse pass cannot
/// answer it without carrying a directory listing it has no other use for.
///
/// A relative `source_path` resolves against the process cwd — measured. That
/// is correct for `armadai audit <relative-path>`, since the reverse pass read
/// the very same relative path moments earlier from the same cwd, and nothing
/// between the two calls moves it. Tests must not rely on it: the cwd is
/// process-global and `IsolatedProjectDir` moves it mid-suite.
fn has_references(skill_md: &Path) -> bool {
    skill_md
        .parent()
        .map(|dir| dir.join("references"))
        .and_then(|refs| std::fs::read_dir(refs).ok())
        .is_some_and(|mut entries| entries.next().is_some())
}

/// Extensions that turn a backticked token into a claim about a file rather
/// than a piece of prose. Read through [`Path::extension`], so a bare `.md`
/// (which has no extension, only a leading dot) never qualifies.
const SOURCE_EXT: [&str; 8] = ["rs", "toml", "md", "yaml", "yml", "json", "sh", "ts"];

/// Directory names the index never descends into: build output and vendored
/// trees, which hold no path anyone cites and can dwarf the source tree.
const SKIPPED_DIRS: [&str; 6] = [".git", "target", "node_modules", "dist", "build", ".venv"];

/// Depth cap for the index walk — deeper than any real source tree, shallow
/// enough that a pathological directory cannot stall the audit.
const MAX_INDEX_DEPTH: u32 = 16;

/// R02 — a repo path cited in the root instructions file that resolves to
/// nothing.
///
/// Counterpart of `A10`, which does this for `@agent` mentions. This is the
/// rule for the stale map: an instructions file that places modules where
/// `ls` shows none of them is read as authoritative, so it costs more than
/// no map at all.
///
/// **False positives are the whole difficulty**, and the filters below are
/// what the rule is. Measured on this repo's own `CLAUDE.md`: the loose form
/// (any backticked token containing `/`) reported 23 paths, **22 of them
/// false** — crate-relative fragments (`cli/`, `web/`), user-config paths
/// (`~/.config/armadai/`), a bare extension (`.md`), a convention filename
/// (`armadai.yaml`). Each filter below removes one of those classes and has
/// its own negative test; a filter with no test is a filter that can silently
/// stop working.
///
/// A citation resolves if it exists under the instructions file's directory
/// **or** anywhere deeper in the tree, component-wise. That second branch is
/// not laxity: multi-crate repos cite modules relative to their crate
/// (`cli/mod.rs`, not `crates/armadai/src/cli/mod.rs`), and root-only
/// resolution flags every one of them. Measured against the pre-#382
/// `CLAUDE.md`, suffix resolution reported 16 stale paths and 0 crate-prefix
/// false positives, where root-only resolution reported 24 with 9 false.
///
/// Known limits, both deliberate:
/// - On a case-insensitive filesystem (macOS by default, measured) `exists()`
///   answers `true` for a path whose case differs from the file's. A human
///   reader is not so forgiving, so this is a false negative — and a
///   platform-dependent one. Detecting it means re-listing every parent to
///   compare names exactly, for a defect nobody has hit.
/// - A path-shaped *convention* (`.armadai/agents.yaml` in a repo that ships
///   no example of one) is reported. Nothing short of reading the sentence
///   distinguishes it from a location claim.
pub(super) fn r02_stale_path(ctx: &AuditContext) -> Vec<Finding> {
    let Some(instructions) = &ctx.config.instructions else {
        return Vec::new();
    };
    // `ReverseLinker::parse` builds this as `root.join("CLAUDE.md")`, so the
    // parent *is* the audited root. Resolving against the cwd instead is the
    // trap already tracked at `config.rs:303`.
    let base = match instructions.source_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        // A bare `CLAUDE.md`: the reverse pass read it from the cwd, so that
        // is where its siblings are.
        _ => Path::new("."),
    };

    let cited = cited_paths(&instructions.content);
    if cited.is_empty() {
        // Nothing to resolve, so nothing to index either.
        return Vec::new();
    }
    let index = index_source_files(base);

    cited
        .into_iter()
        .filter(|p| !resolves(base, &index, p))
        .map(|p| Finding {
            rule: "R02",
            severity: Severity::Warning,
            file: instructions.source_path.clone(),
            related: Vec::new(),
            message: format!("instructions cite `{p}`, which does not exist"),
            suggestion: Some(
                "fix or drop the reference — a stale map is read as authoritative".to_string(),
            ),
        })
        .collect()
}

/// Whether a cited path exists under `base`, or anywhere in `index` as a
/// whole-component suffix. [`Path::ends_with`] compares components, so
/// `li/mod.rs` is **not** satisfied by `crates/cli/mod.rs`.
fn resolves(base: &Path, index: &[std::path::PathBuf], cited: &str) -> bool {
    let rel = Path::new(cited);
    base.join(rel).exists() || index.iter().any(|found| found.ends_with(rel))
}

/// Every file under `base` carrying a [`SOURCE_EXT`] extension. Only those can
/// ever satisfy a citation, since [`has_repo_path_shape`] rejects candidates
/// without one — so the index stays a fraction of the tree (505 of 600 files
/// on this repo, measured).
///
/// The one filesystem walk any rule performs, and unavoidable: "does this
/// path exist" is a question about the disk, and the reverse pass cannot
/// answer it without carrying a full listing it has no other use for.
fn index_source_files(base: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    collect_source_files(base, MAX_INDEX_DEPTH, &mut out);
    out
}

fn collect_source_files(dir: &Path, depth: u32, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // `DirEntry::file_type` does not follow symlinks, so a symlinked
        // cycle cannot make this walk diverge.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            let name = entry.file_name();
            if depth == 0 || SKIPPED_DIRS.iter().any(|d| name == *d) {
                continue;
            }
            collect_source_files(&entry.path(), depth - 1, out);
        } else if kind.is_file() && has_source_ext(&entry.path()) {
            out.push(entry.path());
        }
    }
}

fn has_source_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SOURCE_EXT.contains(&e))
}

/// Distinct backticked tokens from `text` that claim a repo file, in order of
/// first appearance. Fenced blocks are examples, not claims, so they are
/// skipped entirely.
fn cited_paths(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for candidate in backticked(line) {
            let candidate = strip_line_reference(candidate);
            if is_placeholder(candidate) || !has_repo_path_shape(candidate) {
                continue;
            }
            // One path, one finding — same as A10 does for @mentions.
            if !out.iter().any(|seen| seen == candidate) {
                out.push(candidate.to_string());
            }
        }
    }
    out
}

/// Inline-code spans on one line.
///
/// A markdown table cell is no special case: splitting on the backtick keeps
/// `| `a/b.rs` | `c/d.rs` |` as two spans, since the pipe never enters one.
///
/// An **unclosed** backtick is, and cannot be resolved: every span after it
/// pairs with the wrong delimiter, so code reads as prose and prose as code.
/// Measured on realistic lines, that direction only ever loses a real
/// citation (a false negative) — the prose spans it promotes carry a space,
/// which [`is_placeholder`] drops. Left as is, deliberately: guarding on
/// backtick parity would cost recall and buy no precision.
fn backticked(line: &str) -> Vec<&str> {
    line.split('`')
        .skip(1)
        .step_by(2)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Drops a trailing `:12` or `:40-52` — the `path:line` form this repo's docs
/// use everywhere. The line number is not part of the claim; the path is.
fn strip_line_reference(candidate: &str) -> &str {
    let Some((head, tail)) = candidate.rsplit_once(':') else {
        return candidate;
    };
    let is_line_ref = !tail.is_empty()
        && tail
            .split('-')
            .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    if is_line_ref { head } else { candidate }
}

/// A token that describes a shape rather than a file. The space check also
/// covers a backticked command line (`armadai audit src/x.rs`), which reads
/// as a path to any naive extractor.
fn is_placeholder(candidate: &str) -> bool {
    const PLACEHOLDER_ROOTS: [&str; 4] = ["path", "foo", "bar", "example"];
    candidate.contains(' ')
        || candidate.contains('<')
        || candidate.contains('*')
        || candidate.contains("...")
        || candidate
            .split('/')
            .next()
            .is_some_and(|first| PLACEHOLDER_ROOTS.contains(&first))
}

/// Whether a token has the form of a path *inside this repository*.
///
/// Four rejections, each measured against a real false positive on our own
/// `CLAUDE.md`: an absolute or home-relative path (a fact about the machine,
/// not the repo), a URL, a bare filename naming a convention (`armadai.yaml`)
/// and anything without a source extension — which is every crate-relative
/// directory fragment (`cli/`, `core/orchestration/es/`), the largest class of
/// all.
fn has_repo_path_shape(candidate: &str) -> bool {
    if candidate.starts_with('/') || candidate.starts_with('~') {
        return false;
    }
    if candidate.contains("://") {
        return false;
    }
    // A bare filename is the name of a convention, not a location.
    let Some((first, _)) = candidate.split_once('/') else {
        return false;
    };
    // A schemeless host (`models.dev/api.json`) reads exactly like a relative
    // path. A dotfile directory (`.github/…`) does not.
    if first.contains('.') && !first.starts_with('.') {
        return false;
    }
    Path::new(candidate)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SOURCE_EXT.contains(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::reverse::{ImportedConfig, ImportedInstructions, ImportedSkill, ParseIssue};
    use crate::audit::rules::{AuditSettings, test_support::skill};

    fn ctx_for<'a>(config: &'a ImportedConfig, settings: &'a AuditSettings) -> AuditContext<'a> {
        AuditContext {
            config,
            settings,
            usage: None,
        }
    }

    /// A skill whose directory really exists under `root`, with no
    /// `references/` unless the caller adds one.
    ///
    /// Every R01 test goes through this rather than through `skill()`'s
    /// relative default path: `has_references` resolves a relative path
    /// against the process cwd, which is shared by the whole test binary and
    /// moved mid-suite by `IsolatedProjectDir`. A fixture on disk makes each
    /// assertion depend on the fixture alone.
    fn skill_on_disk(root: &Path, name: &str, body_tokens: usize) -> ImportedSkill {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = skill(name, body_tokens);
        s.source_path = dir.join("SKILL.md");
        s
    }

    fn config_of(skills: Vec<ImportedSkill>) -> ImportedConfig {
        ImportedConfig {
            skills,
            ..Default::default()
        }
    }

    #[test]
    fn r01_flags_a_big_skill_with_no_references() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_of(vec![skill_on_disk(dir.path(), "heavy", 5000)]);
        let settings = AuditSettings::default();
        let f = r01_oversized_skill(&ctx_for(&config, &settings));
        assert_eq!(f.len(), 1, "expected exactly one finding, got {f:?}");
        assert_eq!(f[0].rule, "R01");
        assert!(
            f[0].message.contains("5000"),
            "the message must carry the measured size: {}",
            f[0].message
        );
        assert!(
            f[0].message.contains("3000"),
            "the message must carry the threshold it was judged against: {}",
            f[0].message
        );
    }

    #[test]
    fn r01_leaves_a_small_skill_alone() {
        let dir = tempfile::tempdir().unwrap();
        // Below the default threshold, and no references/ either — so the size
        // condition is the only thing that can be keeping this one quiet.
        let config = config_of(vec![skill_on_disk(dir.path(), "light", 700)]);
        let settings = AuditSettings::default();
        assert!(r01_oversized_skill(&ctx_for(&config, &settings)).is_empty());
    }

    #[test]
    fn r01_honours_a_lowered_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_of(vec![skill_on_disk(dir.path(), "light", 700)]);
        let settings = AuditSettings {
            skill_token_threshold: 500,
            ..AuditSettings::default()
        };
        let f = r01_oversized_skill(&ctx_for(&config, &settings));
        assert_eq!(f.len(), 1, "expected exactly one finding, got {f:?}");
        assert!(
            f[0].message.contains("threshold 500"),
            "the configured threshold must be the one reported: {}",
            f[0].message
        );
    }

    #[test]
    fn r01_leaves_a_big_but_split_skill_alone() {
        // The `quality-playbook` case: 41795 words with 16 references. Big and
        // correctly structured must not be flagged.
        let dir = tempfile::tempdir().unwrap();
        let s = skill_on_disk(dir.path(), "split", 40_000);
        let refs = dir.path().join("split/references");
        std::fs::create_dir_all(&refs).unwrap();
        std::fs::write(refs.join("detail.md"), "detail").unwrap();

        let config = config_of(vec![s]);
        let settings = AuditSettings::default();
        assert!(
            r01_oversized_skill(&ctx_for(&config, &settings)).is_empty(),
            "a split skill must never be flagged, however big"
        );
    }

    #[test]
    fn r01_treats_an_empty_references_dir_as_no_disclosure() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill_on_disk(dir.path(), "hollow", 5000);
        std::fs::create_dir_all(dir.path().join("hollow/references")).unwrap();

        let config = config_of(vec![s]);
        let settings = AuditSettings::default();
        assert_eq!(
            r01_oversized_skill(&ctx_for(&config, &settings)).len(),
            1,
            "an empty references/ is not progressive disclosure"
        );
    }

    #[test]
    fn r01_treats_a_references_file_as_no_disclosure() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill_on_disk(dir.path(), "decoy", 5000);
        // A regular file, not a directory: nothing here loads on demand.
        std::fs::write(dir.path().join("decoy/references"), "not a dir").unwrap();

        let config = config_of(vec![s]);
        let settings = AuditSettings::default();
        assert_eq!(
            r01_oversized_skill(&ctx_for(&config, &settings)).len(),
            1,
            "a file named references is not progressive disclosure"
        );
    }

    #[test]
    fn r01_does_not_stack_on_an_unparsable_skill() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = skill_on_disk(dir.path(), "broken", 9000);
        s.issues = vec![ParseIssue {
            file: s.source_path.clone(),
            message: "invalid yaml".into(),
        }];
        let config = config_of(vec![s]);
        let settings = AuditSettings::default();
        assert!(
            r01_oversized_skill(&ctx_for(&config, &settings)).is_empty(),
            "A01/A09 own parse failures — one root cause, one finding"
        );
    }

    #[test]
    fn r01_does_not_stack_on_a_skill_without_skill_md() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = skill_on_disk(dir.path(), "headless", 9000);
        s.has_skill_md = false;
        let config = config_of(vec![s]);
        let settings = AuditSettings::default();
        assert!(
            r01_oversized_skill(&ctx_for(&config, &settings)).is_empty(),
            "A09 owns a missing SKILL.md — there is no file to call oversized"
        );
    }

    /// The registry entry is a condition like any other: without this test,
    /// forgetting it leaves every unit test above green while `armadai audit`
    /// never runs R01.
    #[test]
    fn r01_is_wired_into_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_of(vec![skill_on_disk(dir.path(), "heavy", 5000)]);
        let settings = AuditSettings::default();
        let findings = crate::audit::rules::run_rules(&ctx_for(&config, &settings));
        let r01: Vec<_> = findings.iter().filter(|f| f.rule == "R01").collect();
        assert_eq!(
            r01.len(),
            1,
            "run_rules must emit R01; all findings: {findings:?}"
        );
    }

    // ---- R02 -------------------------------------------------------------

    fn instructions_saying(dir: &Path, body: &str) -> ImportedConfig {
        ImportedConfig {
            instructions: Some(ImportedInstructions {
                source_path: dir.join("CLAUDE.md"),
                content: body.to_string(),
            }),
            ..Default::default()
        }
    }

    /// Creates an empty file at `rel` under `root`, parents included, so a
    /// citation of it has something real to resolve to.
    fn touch(root: &Path, rel: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    fn r02_on(config: &ImportedConfig) -> Vec<Finding> {
        let settings = AuditSettings::default();
        r02_stale_path(&ctx_for(config, &settings))
    }

    fn cited_in(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.message.as_str()).collect()
    }

    #[test]
    fn r02_flags_a_path_that_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(dir.path(), "See `src/gone/mod.rs` for details.");
        let f = r02_on(&config);
        assert_eq!(f.len(), 1, "got {:?}", cited_in(&f));
        assert_eq!(f[0].rule, "R02");
        assert_eq!(f[0].severity, Severity::Warning);
        assert_eq!(
            f[0].file,
            dir.path().join("CLAUDE.md"),
            "the finding is anchored on the file making the claim"
        );
        assert!(
            f[0].message.contains("src/gone/mod.rs"),
            "the message must name the path: {}",
            f[0].message
        );
    }

    /// Also pins *where* a citation is resolved: the fixture lives in the
    /// tempdir next to `CLAUDE.md`, nowhere near the test binary's cwd.
    #[test]
    fn r02_leaves_an_existing_path_alone() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "src/here/mod.rs");
        let config = instructions_saying(dir.path(), "See `src/here/mod.rs`.");
        assert_eq!(cited_in(&r02_on(&config)), Vec::<&str>::new());
    }

    /// The citation inside the fence is backticked on purpose: a *bare* path
    /// in a fence is no candidate to begin with, so a fixture without the
    /// backticks stays green with the fence tracking removed — measured, it
    /// was this test's first form.
    #[test]
    fn r02_ignores_paths_inside_a_code_fence() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(
            dir.path(),
            "An instructions file looks like this:\n\n```markdown\nThe engine is in \
             `src/example/never.rs`.\n```\n\nDone.\n",
        );
        assert_eq!(
            cited_in(&r02_on(&config)),
            Vec::<&str>::new(),
            "a fenced block is an example, not a claim"
        );
    }

    #[test]
    fn r02_ignores_placeholders_globs_and_command_lines() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(
            dir.path(),
            "Use `path/to/thing.rs`, `<your>/file.rs`, `crates/*/src/lib.rs`, `foo/bar.rs`, \
             and run `armadai audit src/here/mod.rs`.",
        );
        assert_eq!(
            cited_in(&r02_on(&config)),
            Vec::<&str>::new(),
            "a placeholder, a glob and a command line all describe a shape, not a file"
        );
    }

    #[test]
    fn r02_ignores_backticked_prose_that_is_not_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(
            dir.path(),
            "The `Provider` trait, `complete()`, and `OrchestrationPattern`.",
        );
        assert_eq!(cited_in(&r02_on(&config)), Vec::<&str>::new());
    }

    /// A module directory carries no extension, so nothing tells us whether
    /// the author meant `es/` or `es.rs` — and this repo's own instructions
    /// are full of crate-relative fragments like `cli/` that resolve nowhere
    /// from the root. Measured: judging them produced 22 false positives on
    /// our own `CLAUDE.md`.
    #[test]
    fn r02_ignores_a_directory_fragment() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(
            dir.path(),
            "Orchestration lives under `core/orchestration/es/`, the CLI in `cli/`.",
        );
        assert_eq!(cited_in(&r02_on(&config)), Vec::<&str>::new());
    }

    /// `armadai.yaml` in prose is the name of a convention, not a location.
    /// `.mcp.json` is the case that makes the rejection load-bearing: a
    /// dotfile passes the host-shape check and carries a real extension, so
    /// only "a path has a directory part" keeps it out.
    #[test]
    fn r02_ignores_a_bare_filename() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(
            dir.path(),
            "Config lives in `armadai.yaml`, see `.md` and `.mcp.json`.",
        );
        assert_eq!(cited_in(&r02_on(&config)), Vec::<&str>::new());
    }

    /// The index skips symlinks, so a cited path that *is* one resolves only
    /// through the direct `exists()` check — which follows links.
    #[cfg(unix)]
    #[test]
    fn r02_resolves_a_symlinked_path() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "real/engine.rs");
        std::fs::create_dir_all(dir.path().join("alias")).unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("real/engine.rs"),
            dir.path().join("alias/engine.rs"),
        )
        .unwrap();
        let config = instructions_saying(dir.path(), "The engine is at `alias/engine.rs`.");
        assert_eq!(cited_in(&r02_on(&config)), Vec::<&str>::new());
    }

    /// Whether `/usr/local/bin/tool.sh` exists is a fact about the machine
    /// running the audit, not about the repository.
    #[test]
    fn r02_ignores_an_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(dir.path(), "Install to `/opt/armadai/hook.sh`.");
        assert_eq!(cited_in(&r02_on(&config)), Vec::<&str>::new());
    }

    #[test]
    fn r02_ignores_a_home_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(
            dir.path(),
            "User agents live in `~/.config/armadai/list.yaml`.",
        );
        assert_eq!(cited_in(&r02_on(&config)), Vec::<&str>::new());
    }

    #[test]
    fn r02_ignores_a_url() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(
            dir.path(),
            "See `https://github.com/anthropics/armadai/blob/master/README.md`.",
        );
        assert_eq!(cited_in(&r02_on(&config)), Vec::<&str>::new());
    }

    /// A schemeless host reads exactly like a relative path but is not one.
    #[test]
    fn r02_ignores_a_schemeless_host() {
        let dir = tempfile::tempdir().unwrap();
        let config =
            instructions_saying(dir.path(), "The catalog comes from `models.dev/api.json`.");
        assert_eq!(cited_in(&r02_on(&config)), Vec::<&str>::new());
    }

    /// A `path:line` reference is the form this repo's docs use everywhere.
    /// The line number is not part of the claim; the path is.
    #[test]
    fn r02_strips_a_line_reference_before_resolving() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "src/here/mod.rs");
        let config = instructions_saying(
            dir.path(),
            "Gone: `src/gone/mod.rs:12`. Here: `src/here/mod.rs:40-52`.",
        );
        let f = r02_on(&config);
        assert_eq!(f.len(), 1, "got {:?}", cited_in(&f));
        assert!(
            f[0].message.contains("src/gone/mod.rs")
                && !f[0].message.contains("src/gone/mod.rs:12"),
            "the resolved path is reported, without the line number: {}",
            f[0].message
        );
    }

    /// The instructions cite modules relative to a crate, not to the repo
    /// root — the convention every multi-crate project uses. Resolving only
    /// at the root would flag all of them.
    #[test]
    fn r02_resolves_a_path_that_exists_deeper_in_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "crates/app/src/cli/mod.rs");
        let config = instructions_saying(dir.path(), "Commands live in `cli/mod.rs`.");
        assert_eq!(cited_in(&r02_on(&config)), Vec::<&str>::new());
    }

    /// Suffix matching is component-wise: `li/mod.rs` is not satisfied by
    /// `cli/mod.rs`, however well the strings end.
    #[test]
    fn r02_matches_whole_components_not_a_string_suffix() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "crates/cli/mod.rs");
        let config = instructions_saying(dir.path(), "Commands live in `li/mod.rs`.");
        let f = r02_on(&config);
        assert_eq!(f.len(), 1, "got {:?}", cited_in(&f));
    }

    #[test]
    fn r02_reports_a_repeated_path_once() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(
            dir.path(),
            "First `src/gone/mod.rs`.\nAnd again `src/gone/mod.rs`.\n",
        );
        let f = r02_on(&config);
        assert_eq!(f.len(), 1, "one path, one finding; got {:?}", cited_in(&f));
    }

    /// Characterization, not a filter: an unclosed backtick re-pairs every
    /// span after it, so real code spans become prose. The paths on that line
    /// go unreported — the safe direction — and the mis-paired prose spans
    /// carry a space, which the placeholder guard drops. Changing this must be
    /// a deliberate decision, not a side effect.
    #[test]
    fn an_unclosed_backtick_silences_its_line_instead_of_inventing_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(
            dir.path(),
            "The `Provider` trait uses `src/gone/a.rs and `src/gone/b.rs`.",
        );
        assert_eq!(cited_in(&r02_on(&config)), Vec::<&str>::new());
    }

    #[test]
    fn r02_is_silent_without_an_instructions_file() {
        let config = ImportedConfig::default();
        assert_eq!(cited_in(&r02_on(&config)), Vec::<&str>::new());
    }

    #[test]
    fn r02_is_wired_into_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let config = instructions_saying(dir.path(), "See `src/gone/mod.rs`.");
        let settings = AuditSettings::default();
        let findings = crate::audit::rules::run_rules(&ctx_for(&config, &settings));
        let r02: Vec<_> = findings.iter().filter(|f| f.rule == "R02").collect();
        assert_eq!(
            r02.len(),
            1,
            "run_rules must emit R02; all findings: {findings:?}"
        );
    }
}
