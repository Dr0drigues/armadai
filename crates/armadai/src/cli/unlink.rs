use std::path::{Path, PathBuf};

use crate::linker::model_resolution::{self, TargetKind};
use crate::linker::{self, LinkAgent};
use armadai_core::project;

/// A file `unlink` might remove, together with what removing it safely
/// requires. Used only by the **fallback** path (see [`unlink_via_fallback`])
/// — the manifest path ([`unlink_from_manifest`]) reads what to do straight
/// from the manifest's `outcome` field instead.
///
/// `unlink` prefers the link manifest (issue #338): a per-project record of
/// exactly what `link` wrote and how to undo it, written by `link` itself
/// (`cli::link::execute`, `linker::manifest`). Only when there is no usable
/// manifest for a target — a fresh clone, a deleted `.armadai/`, or a
/// project linked before the manifest existed — does `unlink` fall back to
/// recomputing what `link` *would* write today. For anything the linker
/// itself produces (agent/coordinator instruction files, skill copies,
/// prompt copies), the only safe rule in that degraded mode is: **delete a
/// file only if its on-disk content is byte-for-byte identical to what
/// would be generated right now.** A hand-written file at a would-be
/// generated path never matches and is always kept; a file the linker
/// really did write and that hasn't been touched since always matches and
/// is reclaimed.
///
/// Accepted limitation of the fallback, stated here and echoed in the CLI
/// output: content can differ from what `link` would generate right now for
/// two reasons `unlink` has no way to distinguish — the file was edited
/// since linking, or it was linked with different options (an explicit
/// `--model`, or an interactive prompt answer). Either way the file is kept,
/// becoming a visible orphan instead of a silent deletion. The manifest
/// path above doesn't have this limitation — it diffs against the digest of
/// what was actually written, not a regeneration.
///
/// `AlwaysDelete` covers the single opt-in case that isn't linker output at
/// all: the project config file removed via `--with-config`. There is
/// nothing generated to diff it against — the flag is the user's own
/// confirmation, so no content guard applies. Both paths share this case
/// via [`with_config_candidate`].
enum Candidate {
    Generated { path: PathBuf, expected: Vec<u8> },
    AlwaysDelete { path: PathBuf },
}

impl Candidate {
    fn path(&self) -> &Path {
        match self {
            Candidate::Generated { path, .. } | Candidate::AlwaysDelete { path } => path,
        }
    }
}

pub async fn execute(
    target: Option<crate::linker::LinkTarget>,
    coordinator_flag: Option<String>,
    dry_run: bool,
    with_config: bool,
    output: Option<PathBuf>,
    agents_filter: Option<Vec<String>>,
) -> anyhow::Result<()> {
    // 1. Find project config
    let (root, config) = project::find_project_config().ok_or_else(|| {
        anyhow::anyhow!(
            "No project config found (.armadai/config.yaml or armadai.yaml). \
             Run `armadai init --project` to create one."
        )
    })?;

    // 2. Determine target. Hoisted ahead of agent loading (unlike the old,
    // pre-manifest flow) because the manifest lookup below needs it and
    // must not depend on whether any agents are currently declared —
    // that's the orphan case (issue #338 case 2): an agent can be gone
    // from the config entirely and its manifest entry still be exactly
    // what tells `unlink` to remove its file.
    let target_name = target
        .map(|t| t.to_string())
        .or_else(|| config.link.as_ref().and_then(|l| l.target.clone()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No link target specified. Use --target or set link.target in armadai.yaml.\n\
                 Supported targets: claude, codex, copilot, gemini, opencode"
            )
        })?;

    // 3. The target's output directory and its own root directory —
    // used by the fallback below for path resolution, and by the
    // manifest path to *confirm* the manifest's own declared `root`
    // against this independently-computed one before trusting it at all
    // (`root_confirmed`, design review R1). It is never itself the
    // boundary the manifest path acts on afterwards — that is the
    // manifest's own `root`, once confirmed — but it is exactly what
    // that confirmation is checked against.
    let linker_impl = linker::create_linker(&target_name)?;
    let output_dir = output
        .clone()
        .or_else(|| {
            config
                .link
                .as_ref()
                .and_then(|l| l.overrides.get(&target_name))
                .and_then(|o| o.output.as_ref())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from(linker_impl.default_output_dir()));
    let target_root = root.join(&output_dir);

    // 4. Prefer the link manifest (issue #338's second half): it records
    // exactly what `link` wrote for this target, independent of the
    // current config, so it has none of the fallback's blind spots — the
    // orphan case above, the skills-recursion case, and the opencode
    // `--model` case (see the module doc). Only fall back to the #342
    // content-match guard when there is nothing usable recorded, and say
    // so — the user is in a degraded mode and should know why some files
    // might be kept that a manifest-driven unlink would have reclaimed.
    match linker::manifest::lookup_target(&root, &target_name) {
        linker::manifest::Lookup::Found(target_manifest)
            if linker::manifest::root_confirmed(&root, &target_root, &target_manifest.root) =>
        {
            unlink_from_manifest(
                &root,
                &target_name,
                target_manifest,
                dry_run,
                with_config,
                agents_filter.as_deref(),
            )
        }
        lookup => {
            // Either no manifest at all, or one whose declared root for
            // this target doesn't match what `unlink` computes right now
            // from the project's own config/`--output` (design review
            // R1) — a manifest is user-writable data, so its `root` is
            // never trusted on its own; only a root confirmed against one
            // `unlink` derives independently is trusted at all. Either
            // way, refuse the manifest wholesale and fall back, saying
            // why.
            match lookup {
                linker::manifest::Lookup::Found(_) => {
                    let w = crate::cli::style::warn();
                    anstream::eprintln!(
                        "{w}  The link manifest for target '{}' declares a root that \
                         doesn't match this project's current output directory — refusing \
                         to trust it and falling back to the #342 content-match guard. \
                         This can happen if the manifest was hand-edited, corrupted, or \
                         copied from another project.{w:#}",
                        target_name
                    );
                }
                linker::manifest::Lookup::Fallback => {
                    let w = crate::cli::style::warn();
                    anstream::eprintln!(
                        "{w}  No link manifest found for target '{}' — falling back to the \
                         #342 content-match guard: a file is only removed when its on-disk \
                         content still byte-for-byte matches what `link` would generate \
                         right now. This happens on a fresh clone, after `.armadai/` was \
                         deleted, or for a project linked before this armadai version. Some \
                         files may be kept that a manifest-driven unlink would have \
                         reclaimed exactly — re-run `link` to write one.{w:#}",
                        target_name
                    );
                }
            }
            unlink_via_fallback(
                root,
                config,
                target_name,
                linker_impl,
                output_dir,
                target_root,
                coordinator_flag,
                dry_run,
                with_config,
                agents_filter,
            )
            .await
        }
    }
}

/// Which project config file `--with-config` would remove, if any: the
/// active one among the three candidate paths. `None` when `--with-config`
/// wasn't passed, or when none of the candidate files exist. Shared by both
/// the manifest path and the fallback — this case is identical in either:
/// there is nothing generated to diff the config file against, so the flag
/// itself is the confirmation, manifest or not.
fn with_config_candidate(root: &Path, with_config: bool) -> Option<PathBuf> {
    if !with_config {
        return None;
    }
    let dotarmadai_config = root.join(".armadai").join("config.yaml");
    let legacy_yaml = root.join("armadai.yaml");
    let legacy_yml = root.join("armadai.yml");

    if dotarmadai_config.exists() {
        Some(dotarmadai_config)
    } else if legacy_yaml.exists() {
        Some(legacy_yaml)
    } else if legacy_yml.exists() {
        Some(legacy_yml)
    } else {
        None
    }
}

/// Consume the link manifest for `target_name`: for each recorded entry,
/// apply the inverse implied by its `outcome` (design §6) — never
/// regenerate, never consult the current config.
///
/// `outcome: Skipped`'s inverse is "do nothing": `link` found this file
/// already there and left it alone, so it was never `unlink`'s to remove
/// (issue #338 case 1 — the over-deletion of a hand-written `CLAUDE.md`).
///
/// `outcome: Created`'s inverse is "delete, but only if the digest still
/// matches" — the #342 guard, now checked against the digest of the
/// content `link` actually wrote instead of a regeneration that may no
/// longer be possible (the `opencode --model` case) or may no longer even
/// be *produced* by the current config (issue #338 case 2, the orphan: an
/// agent removed from the config still has its manifest entry, so its file
/// is still a candidate here — this is the case the fallback cannot fix).
///
/// Only entries the manifest actually names are ever candidates at all —
/// issue #338 case 3 (a user file dropped into a linked skill directory)
/// simply has no entry and is never considered, with no recursive sweep to
/// avoid it.
///
/// Security amendment (post-implementation review): every entry's `path`,
/// and every recorded `created_dirs` path, is checked with
/// [`linker::manifest::is_trusted`] against the target's own recorded
/// `root` before being acted on. A manifest is user-writable data — it
/// must be treated exactly as untrusted as any other file the tool didn't
/// just write itself — so a forged or hand-corrupted entry naming a path
/// outside the target's own tree (`../outside/victim.txt`, or an absolute
/// path unrelated to the target) is refused and reported, never deleted.
fn unlink_from_manifest(
    root: &Path,
    target_name: &str,
    target_manifest: linker::manifest::TargetManifest,
    dry_run: bool,
    with_config: bool,
    agents_filter: Option<&[String]>,
) -> anyhow::Result<()> {
    let linker::manifest::TargetManifest {
        root: target_root,
        created_dirs,
        entries,
        ..
    } = target_manifest;

    let filter_lower: Option<Vec<String>> =
        agents_filter.map(|f| f.iter().map(|s| s.to_lowercase()).collect());

    // `--agents` narrows to the named agents' own files *and* the
    // coordinator's — matching `produced_by`'s name for both kinds. This
    // mirrors the fallback path's actual (if accidental) behaviour: it
    // filters the roster *before* extracting the coordinator from it, so
    // naming a subset of agents that excludes the coordinator's own name
    // silently leaves no coordinator to regenerate a context file for
    // either. Skill/prompt entries are never filtered by `--agents` here,
    // matching the fallback too — its skill/prompt loops never consulted
    // it at all.
    let relevant: Vec<&linker::manifest::ManifestEntry> = entries
        .iter()
        .filter(|e| match (&filter_lower, e.produced_by.kind) {
            (
                Some(filter),
                linker::manifest::ProducedByKind::Agent
                | linker::manifest::ProducedByKind::Coordinator,
            ) => filter.contains(&e.produced_by.name.to_lowercase()),
            _ => true,
        })
        .collect();

    if let Some(filter) = agents_filter
        && !relevant.iter().any(|e| {
            matches!(
                e.produced_by.kind,
                linker::manifest::ProducedByKind::Agent
                    | linker::manifest::ProducedByKind::Coordinator
            )
        })
    {
        anyhow::bail!("No agents match the given filter: {}", filter.join(", "));
    }

    let config_candidate = with_config_candidate(root, with_config);

    if dry_run {
        print_manifest_dry_run(
            root,
            target_name,
            &target_root,
            &relevant,
            &entries,
            &created_dirs,
            config_candidate.as_deref(),
        )?;
        return Ok(());
    }

    let mut deleted = 0;
    let mut kept = 0;
    let mut absent = 0;
    let mut untrusted = 0;

    for entry in &relevant {
        if let Some(cause) =
            linker::manifest::diagnose_trust_failure(root, &target_root, &entry.path)
        {
            let e = crate::cli::style::err();
            let reason = trust_failure_reason(cause);
            anstream::eprintln!(
                "{e}  refused: manifest entry '{}' for target '{}' resolves outside its \
                 trusted root — ignoring it; {reason}{e:#}",
                entry.path.display(),
                target_name
            );
            untrusted += 1;
            continue;
        }

        // Resolved via `resolve_real`, not a raw `root.join(&entry.path)`
        // — the same path a symlinked intermediate would otherwise let
        // slip past the trust check above (design review R2) is also the
        // path every operation below actually acts on, and normalising
        // it here is what makes "already absent" describe the real
        // filesystem instead of a raw, un-normalised join that can
        // report a false absence for an otherwise-valid `a/../b` entry
        // whose literal `a` component doesn't exist (R4).
        let path = linker::manifest::resolve_real(root, &entry.path);
        if !path.exists() {
            if path.is_symlink() {
                // Present on disk, but its target is gone — not "already
                // absent" (issue #348, 3rd bullet), and its content can
                // never be verified against a digest either, so the same
                // conservative "kept" outcome other unverifiable entries
                // get applies here too — just with an accurate reason.
                let w = crate::cli::style::warn();
                anstream::println!(
                    "{w}  kept {} (broken symlink — its target is missing, so its \
                     content can't be verified){w:#}",
                    path.display()
                );
                kept += 1;
            } else {
                absent += 1;
            }
            continue;
        }
        match entry.outcome {
            linker::manifest::Outcome::Skipped => {
                let w = crate::cli::style::warn();
                anstream::println!(
                    "{w}  kept {} (hand-written — link recorded it as skipped){w:#}",
                    path.display()
                );
                kept += 1;
            }
            linker::manifest::Outcome::Created => {
                let check = entry
                    .digest
                    .as_deref()
                    .map(|d| linker::manifest::check_digest(d, &path));
                match check {
                    Some(linker::manifest::DigestCheck::Matches) => {
                        std::fs::remove_file(&path)?;
                        let m = crate::cli::style::muted();
                        anstream::println!("{m}  deleted {}{m:#}", path.display());
                        deleted += 1;
                    }
                    Some(linker::manifest::DigestCheck::Differs) => {
                        let w = crate::cli::style::warn();
                        anstream::println!(
                            "{w}  kept {} (content differs from what link wrote){w:#}",
                            path.display()
                        );
                        kept += 1;
                    }
                    Some(linker::manifest::DigestCheck::Unverifiable) | None => {
                        let w = crate::cli::style::warn();
                        anstream::println!(
                            "{w}  kept {} (cannot verify — file unreadable, or digest uses \
                             an algorithm this build doesn't recognise){w:#}",
                            path.display()
                        );
                        kept += 1;
                    }
                }
            }
        }
    }

    if let Some(config_path) = config_candidate {
        if config_path.exists() {
            std::fs::remove_file(&config_path)?;
            let m = crate::cli::style::muted();
            anstream::println!("{m}  deleted {}{m:#}", config_path.display());
            deleted += 1;
            if let Some(parent) = config_path.parent() {
                remove_empty_ancestors(parent, root);
            }
        } else {
            absent += 1;
        }
    }

    // Remove exactly the directories `link` itself created for this
    // target (design fix 1b) — deepest first, so a child never blocks its
    // own parent's removal. Every dir's fate is decided by the single
    // `decide_created_dir` function `--dry-run`'s preview also calls
    // (issue #348, 1st bullet: the two must never silently diverge on
    // this decision again — a real regression this chantier shipped
    // once).
    for dir in linker::manifest::deepest_first(&created_dirs) {
        match linker::manifest::decide_created_dir(root, &target_root, &dir, &entries) {
            linker::manifest::CreatedDirDecision::Eligible => {
                let dir_path = linker::manifest::resolve_real(root, &dir);
                let is_empty = std::fs::read_dir(&dir_path)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false);
                if dir_path.is_dir() && is_empty {
                    let _ = std::fs::remove_dir(&dir_path);
                }
            }
            linker::manifest::CreatedDirDecision::IsTargetRoot(cause) => {
                let e = crate::cli::style::err();
                let relation = target_root_relation(cause);
                let reason = trust_failure_reason(cause);
                anstream::eprintln!(
                    "{e}  refused: recorded directory '{}' for target '{}' {relation} the \
                     target's own root — ignoring it; {reason}{e:#}",
                    dir.display(),
                    target_name
                );
                untrusted += 1;
            }
            linker::manifest::CreatedDirDecision::Untrusted(cause) => {
                let e = crate::cli::style::err();
                let reason = trust_failure_reason(cause);
                anstream::eprintln!(
                    "{e}  refused: recorded directory '{}' for target '{}' resolves \
                     outside its trusted root — ignoring it; {reason}{e:#}",
                    dir.display(),
                    target_name
                );
                untrusted += 1;
            }
            linker::manifest::CreatedDirDecision::Implausible => {
                let e = crate::cli::style::err();
                anstream::eprintln!(
                    "{e}  refused: recorded directory '{}' for target '{}' does not \
                     correspond to any file link recorded creating — ignoring it; the \
                     manifest may be corrupt or forged{e:#}",
                    dir.display(),
                    target_name
                );
                untrusted += 1;
            }
        }
    }

    let o = crate::cli::style::ok();
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    anstream::println!(
        "\n{o}Unlinked{o:#} {a}'{}'{a:#}: {m}{} deleted, {} kept, {} already absent.{m:#}",
        target_name,
        deleted,
        kept,
        absent
    );
    if kept > 0 {
        anstream::println!(
            "{m}  Kept files are either hand-written (link recorded them as skipped), have \
             content that no longer matches what link wrote, or could not be verified. \
             Remove them by hand if you no longer want them.{m:#}"
        );
    }
    if untrusted > 0 {
        let e = crate::cli::style::err();
        // Deliberately not a single blanket cause here (issue #348, 2nd
        // bullet): the refusal(s) printed above already said, per item,
        // whether it was the manifest's own text or the filesystem that
        // was at fault — this summary must not paper back over that
        // distinction with one claim that doesn't hold for both.
        //
        // On **stderr**, like the per-item refusals it points at: as a
        // `println!` it told a `2>/dev/null` caller to consult reasons
        // that stream had just discarded. A pointer must live on the same
        // stream as what it points to.
        anstream::eprintln!(
            "{e}  {} manifest item(s) were refused and left untouched; each refusal \
             above names the item and why.{e:#}",
            untrusted
        );
        // A refusal is a failure to complete the requested work, not a
        // partial success — exit non-zero so scripted callers notice
        // (design review R5), even though every trustworthy entry above
        // was still handled.
        anyhow::bail!(
            "{} manifest item(s) for target '{}' were refused; the request did not \
             fully complete",
            untrusted,
            target_name
        );
    }

    Ok(())
}

/// Human-readable explanation for one [`linker::manifest::TrustFailure`]
/// cause — shared by the real pass and its `--dry-run` preview so the two
/// never phrase the same cause two different ways.
fn trust_failure_reason(cause: linker::manifest::TrustFailure) -> &'static str {
    match cause {
        linker::manifest::TrustFailure::ManifestEscapesRoot => {
            "the manifest may be corrupt or forged"
        }
        linker::manifest::TrustFailure::FilesystemDiverged => {
            "the manifest itself looks intact, but something on the filesystem has \
             changed since link ran (e.g. a new symlink) — inspect the target \
             directory before retrying"
        }
    }
}

/// How a recorded `created_dirs` entry relates to the target's own root,
/// for the two causes [`linker::manifest::CreatedDirDecision::IsTargetRoot`]
/// can have — shared by the real pass and its `--dry-run` preview, like
/// [`trust_failure_reason`], so the two never phrase the same cause two
/// different ways.
///
/// The verb has to move with the cause (issue #348): a manifest that
/// literally records `.claude` *names* the root, while a manifest that
/// correctly records `.claude/agents` and finds it symlinked to `.claude`
/// afterwards *now resolves to* it — saying "names" for the second blames
/// the manifest for text it never contained.
fn target_root_relation(cause: linker::manifest::TrustFailure) -> &'static str {
    match cause {
        linker::manifest::TrustFailure::ManifestEscapesRoot => "names",
        linker::manifest::TrustFailure::FilesystemDiverged => "now resolves to",
    }
}

/// `--dry-run` rendering for the manifest path — mirrors the fallback's own
/// dry-run output shape so the two paths read the same way to a user who
/// doesn't know (and shouldn't need to know) which one is active.
///
/// Applies the same *decisions* the real pass does — `created_dirs`
/// included (issue #348, 1st bullet: this was the actual defect, not just
/// the wording — the pre-fix dry run counted a directory "recorded for
/// cleanup" that the real pass would have refused, and exited 0 where the
/// real pass exits 1). Every refusal comes from the same
/// [`linker::manifest::decide_created_dir`] call, in the same
/// deepest-first order ([`linker::manifest::deepest_first`]), and anything
/// this preview would refuse makes it return an error too, so scripted
/// callers see the same signal a real run would give — a preview that
/// can't fail is not a preview.
///
/// One thing it deliberately does **not** mirror, because it cannot: the
/// real pass removes an eligible directory only if that directory is
/// still on disk and *empty at that moment*, which is only true after the
/// files inside it have been deleted — files this preview, by definition,
/// has not deleted. So the count below is what its wording says,
/// "recorded for cleanup", not "would be removed": a directory the user
/// deleted by hand still counts here and the real pass then quietly
/// touches nothing. Overstating that as an identical filesystem check is
/// what this doc used to do.
fn print_manifest_dry_run(
    root: &Path,
    target_name: &str,
    target_root: &Path,
    entries: &[&linker::manifest::ManifestEntry],
    all_entries: &[linker::manifest::ManifestEntry],
    created_dirs: &[PathBuf],
    config_candidate: Option<&Path>,
) -> anyhow::Result<()> {
    let h = crate::cli::style::header();
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    let w = crate::cli::style::warn();
    let e = crate::cli::style::err();
    anstream::println!(
        "{h}Dry run{h:#} — files that would be removed for {a}'{}'{a:#}:\n",
        target_name
    );

    let mut would_remove = 0;
    let mut would_keep = 0;
    let mut absent = 0;
    let mut untrusted = 0;

    for entry in entries {
        if let Some(cause) =
            linker::manifest::diagnose_trust_failure(root, target_root, &entry.path)
        {
            let reason = trust_failure_reason(cause);
            anstream::println!(
                "{e}  {} (would refuse — outside the trusted root; {reason}){e:#}",
                entry.path.display()
            );
            untrusted += 1;
            continue;
        }
        let path = linker::manifest::resolve_real(root, &entry.path);
        if !path.exists() {
            if path.is_symlink() {
                anstream::println!(
                    "{w}  {} (would keep — broken symlink, cannot verify content){w:#}",
                    path.display()
                );
                would_keep += 1;
            } else {
                anstream::println!("{m}  {} (already absent){m:#}", path.display());
                absent += 1;
            }
            continue;
        }
        match entry.outcome {
            linker::manifest::Outcome::Skipped => {
                anstream::println!(
                    "{w}  {} (would keep — hand-written, recorded as skipped){w:#}",
                    path.display()
                );
                would_keep += 1;
            }
            linker::manifest::Outcome::Created => {
                let check = entry
                    .digest
                    .as_deref()
                    .map(|d| linker::manifest::check_digest(d, &path));
                match check {
                    Some(linker::manifest::DigestCheck::Matches) => {
                        anstream::println!("{m}  {}{m:#}", path.display());
                        would_remove += 1;
                    }
                    Some(linker::manifest::DigestCheck::Differs) => {
                        anstream::println!(
                            "{w}  {} (would keep — content differs){w:#}",
                            path.display()
                        );
                        would_keep += 1;
                    }
                    Some(linker::manifest::DigestCheck::Unverifiable) | None => {
                        anstream::println!(
                            "{w}  {} (would keep — cannot verify){w:#}",
                            path.display()
                        );
                        would_keep += 1;
                    }
                }
            }
        }
    }

    if let Some(config_path) = config_candidate {
        if config_path.exists() {
            anstream::println!("{m}  {}{m:#}", config_path.display());
            would_remove += 1;
        } else {
            anstream::println!("{m}  {} (already absent){m:#}", config_path.display());
            absent += 1;
        }
    }

    // Same decision the real pass makes for each recorded directory (issue
    // #348, 1st bullet) — a refusal here counts into `untrusted` exactly
    // like an entry's does, so the preview's exit code matches what a real
    // run would do.
    let mut would_clean_dirs = 0;
    for dir in linker::manifest::deepest_first(created_dirs) {
        match linker::manifest::decide_created_dir(root, target_root, &dir, all_entries) {
            linker::manifest::CreatedDirDecision::Eligible => {
                would_clean_dirs += 1;
            }
            linker::manifest::CreatedDirDecision::IsTargetRoot(cause) => {
                let relation = target_root_relation(cause);
                let reason = trust_failure_reason(cause);
                anstream::println!(
                    "{e}  {} (would refuse — {relation} the target's own root; \
                     {reason}){e:#}",
                    dir.display()
                );
                untrusted += 1;
            }
            linker::manifest::CreatedDirDecision::Untrusted(cause) => {
                let reason = trust_failure_reason(cause);
                anstream::println!(
                    "{e}  {} (would refuse — outside the trusted root; {reason}){e:#}",
                    dir.display()
                );
                untrusted += 1;
            }
            linker::manifest::CreatedDirDecision::Implausible => {
                anstream::println!(
                    "{e}  {} (would refuse — matches no file link recorded creating; \
                     the manifest may be corrupt or forged){e:#}",
                    dir.display()
                );
                untrusted += 1;
            }
        }
    }

    anstream::println!(
        "\n{m}  {} would be removed, {} would be kept, {} already absent \
         ({} director{} recorded for cleanup).{m:#}",
        would_remove,
        would_keep,
        absent,
        would_clean_dirs,
        if would_clean_dirs == 1 { "y" } else { "ies" }
    );
    if would_keep > 0 {
        anstream::println!(
            "{m}  Kept files are either hand-written (recorded as skipped by link), have \
             content that no longer matches what link wrote, or could not be verified. \
             Remove them by hand if you no longer want them.{m:#}"
        );
    }
    if untrusted > 0 {
        anstream::println!(
            "{e}  {} manifest item(s) would be refused — see the reason(s) given for \
             each one above.{e:#}",
            untrusted
        );
        // Mirrors the real pass's own exit behaviour (design review R5):
        // a refusal is a failure to complete, not a partial success, so
        // the preview must fail too — otherwise it is not actually
        // showing what the real run would do (issue #348, 1st bullet).
        anyhow::bail!(
            "{} manifest item(s) for target '{}' would be refused; a real run would \
             not fully complete",
            untrusted,
            target_name
        );
    }

    Ok(())
}

/// The #342 fallback: recompute what `link` would write for the *current*
/// config, and delete only what still matches byte-for-byte. Entered only
/// when [`linker::manifest::lookup_target`] found nothing usable — see the
/// module doc and `execute`'s step 4 for when and why.
///
/// This is deliberately close to `unlink`'s pre-manifest body: the
/// content-match guard, the source-scoped skill sweep, and the bounded
/// empty-ancestor cleanup are the mitigation issue #342 shipped, and they
/// stay exactly as they were — the manifest path above is what changed,
/// not this one.
#[allow(clippy::too_many_arguments)]
async fn unlink_via_fallback(
    root: PathBuf,
    config: armadai_core::project::ProjectConfig,
    target_name: String,
    linker_impl: Box<dyn linker::Linker>,
    output_dir: PathBuf,
    target_root: PathBuf,
    coordinator_flag: Option<String>,
    dry_run: bool,
    with_config: bool,
    agents_filter: Option<Vec<String>>,
) -> anyhow::Result<()> {
    // Every agent in `.armadai/agents.yaml` is included automatically (it
    // does not need to be relisted in `agents:`), the same gate `link`,
    // `list` and `run` all widened for this format: an otherwise-empty
    // `agents:` list is only a real error when there is no declarations file
    // either. Without this, `unlink` reports the false "No agents declared in
    // project config." for exactly the project `link` just wrote three files
    // for, and removes nothing.
    if !armadai_core::agent_source::project_declares_agents(&root, &config) {
        anyhow::bail!("No agents declared in project config.");
    }

    // Resolve and load agents — file-backed and declared alike.
    let fragments = armadai_core::agent_source::project_fragments(&root);
    let (agents, warnings) =
        armadai_core::agent_source::load_all_agents(&config, &root, &fragments);
    // `unlink` writes no config — it only removes files `link` would have
    // written — so unlike `link` it never needs to refuse over a drop: warn
    // and remove whatever can still be resolved, same policy as `list`.
    for w in &warnings {
        let s = crate::cli::style::warn();
        anstream::eprintln!("{s}  warn: {}{s:#}", w.message());
    }

    let mut link_agents: Vec<LinkAgent> = agents.iter().map(LinkAgent::from).collect();

    if link_agents.is_empty() {
        anyhow::bail!("No agents could be resolved. Check your project config.");
    }

    // Resolve deprecated model aliases — `link` does this before
    // generating, so the content guard below must reproduce it too, or
    // every agent still using a since-renamed model would never match.
    for agent in &mut link_agents {
        armadai_core::model_aliases::resolve_model_deprecations(
            &mut agent.model,
            &mut agent.model_fallback,
        );
    }

    // Filter by --agents if provided
    if let Some(ref filter) = agents_filter {
        let filter_lower: Vec<String> = filter.iter().map(|s| s.to_lowercase()).collect();
        link_agents.retain(|a| filter_lower.contains(&a.name.to_lowercase()));
        if link_agents.is_empty() {
            anyhow::bail!("No agents match the given filter: {}", filter.join(", "));
        }
    }

    // Extract coordinator if configured (CLI flag takes priority over config)
    let coordinator_name =
        coordinator_flag.or_else(|| config.link.as_ref().and_then(|l| l.coordinator.clone()));
    let mut coordinator = coordinator_name.and_then(|name| {
        let idx = link_agents
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(&name))?;
        Some(link_agents.remove(idx))
    });

    // Model resolution — mirror what `link` computes for this target, so
    // the regenerated content used by the guard below matches what `link`
    // actually wrote. For `LlmEditor` targets (claude, gemini, codex) this
    // is a pure function of the current config, exactly like `link`'s own
    // step, so it reproduces byte-for-byte. For `Orchestrator` targets
    // (copilot, opencode), `link` may additionally honour an explicit
    // `--model` flag or an interactive prompt at link time — `unlink` takes
    // neither, so it can only reproduce the no-flag/non-interactive default
    // (`latest:*` resolution per agent). A link that used an explicit model
    // there produces content `unlink` cannot recompute; the guard then
    // correctly keeps those files rather than guessing why they differ. On
    // `opencode` specifically, this makes a `--model`-linked (or
    // interactively-answered) file permanently un-reclaimable by this
    // fallback — exactly why the manifest path above exists.
    let target_kind = model_resolution::classify_target(&target_name);
    match target_kind {
        TargetKind::LlmEditor { provider } => {
            #[cfg(feature = "providers-api")]
            {
                model_resolution::remap_models_for_llm_editor(&mut link_agents, provider).await;
                if let Some(ref mut coord) = coordinator {
                    model_resolution::remap_models_for_llm_editor(
                        std::slice::from_mut(coord),
                        provider,
                    )
                    .await;
                }
            }
            #[cfg(not(feature = "providers-api"))]
            {
                model_resolution::remap_models_for_llm_editor(&mut link_agents, provider);
                if let Some(ref mut coord) = coordinator {
                    model_resolution::remap_models_for_llm_editor(
                        std::slice::from_mut(coord),
                        provider,
                    );
                }
            }
        }
        TargetKind::Orchestrator => {
            model_resolution::resolve_latest_placeholders(&mut link_agents);
            if let Some(ref mut coord) = coordinator {
                model_resolution::resolve_latest_placeholders(std::slice::from_mut(coord));
            }
        }
    }

    // Regenerate the expected file list — same content `link` would write
    // today — so deletions can be gated on a content match instead of
    // trusting paths alone.
    let sources = &config.sources;
    let files = linker_impl.generate(&link_agents, coordinator.as_ref(), sources);

    if files.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}No files to remove.{m:#}");
        return Ok(());
    }

    // Resolve output paths relative to project root, keeping the generated
    // content alongside each path for the guard below.
    let mut candidates: Vec<Candidate> = files
        .into_iter()
        .map(|f| {
            let default_dir = PathBuf::from(linker_impl.default_output_dir());
            let relative = f
                .path
                .strip_prefix(&default_dir)
                .unwrap_or(&f.path)
                .to_path_buf();
            Candidate::Generated {
                path: root.join(&output_dir).join(relative),
                expected: f.content.into_bytes(),
            }
        })
        .collect();

    // Include skill files — but only the ones the skill's *source*
    // directory still names. `link` copies exactly those paths into
    // `<output_dir>/skills/<name>/`; anything else found there afterwards
    // (issue #338 case 3 — the worst measured outcome) was placed by the
    // user and has no source-side counterpart, so it is never even
    // considered, let alone swept recursively.
    let (skill_dirs, _) = project::resolve_all_skills(&config, &root);
    for skill_dir in &skill_dirs {
        let skill_name = skill_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let dest_dir = root.join(&output_dir).join("skills").join(skill_name);
        if !dest_dir.exists() {
            continue;
        }
        for (relative, expected) in collect_source_files(skill_dir) {
            candidates.push(Candidate::Generated {
                path: dest_dir.join(&relative),
                expected,
            });
        }
    }

    // Include prompt files, gated the same way: the expected content is
    // whatever the source prompt file currently holds.
    let (prompt_paths, _) = project::resolve_all_prompts(&config, &root);
    for prompt_path in &prompt_paths {
        let filename = prompt_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.md");
        if let Ok(expected) = std::fs::read(prompt_path) {
            candidates.push(Candidate::Generated {
                path: root.join(&output_dir).join("prompts").join(filename),
                expected,
            });
        }
    }

    // Optionally include the project config file itself — shared with the
    // manifest path via `with_config_candidate`.
    if let Some(config_path) = with_config_candidate(&root, with_config) {
        candidates.push(Candidate::AlwaysDelete { path: config_path });
    }

    // Dry run
    if dry_run {
        let h = crate::cli::style::header();
        let a = crate::cli::style::accent();
        let m = crate::cli::style::muted();
        let w = crate::cli::style::warn();
        anstream::println!(
            "{h}Dry run{h:#} — files that would be removed for {a}'{}'{a:#}:\n",
            target_name
        );
        let mut would_remove = 0;
        let mut would_keep = 0;
        let mut absent = 0;
        for candidate in &candidates {
            let path = candidate.path();
            if !path.exists() {
                if path.is_symlink() {
                    anstream::println!(
                        "{w}  {} (would keep — broken symlink, cannot verify content){w:#}",
                        path.display()
                    );
                    would_keep += 1;
                } else {
                    anstream::println!("{m}  {} (already absent){m:#}", path.display());
                    absent += 1;
                }
                continue;
            }
            match candidate {
                Candidate::AlwaysDelete { .. } => {
                    anstream::println!("{m}  {}{m:#}", path.display());
                    would_remove += 1;
                }
                Candidate::Generated { expected, .. } => {
                    if content_matches(path, expected) {
                        anstream::println!("{m}  {}{m:#}", path.display());
                        would_remove += 1;
                    } else {
                        anstream::println!(
                            "{w}  {} (would keep — content differs){w:#}",
                            path.display()
                        );
                        would_keep += 1;
                    }
                }
            }
        }
        anstream::println!(
            "\n{m}  {} would be removed, {} would be kept, {} already absent.{m:#}",
            would_remove,
            would_keep,
            absent
        );
        if would_keep > 0 {
            anstream::println!(
                "{m}  Kept files were left in place for the reason given on each line \
                 above: content that differs from what link would generate now \
                 (possibly edited since linking, or linked with different options — \
                 e.g. --model, or an interactive prompt answer; unlink cannot tell \
                 which), or a broken symlink whose content cannot be compared at all. \
                 Remove them by hand if you no longer want them.{m:#}"
            );
        }
        return Ok(());
    }

    // Delete existing files whose content still matches what the linker
    // would generate today.
    let mut deleted = 0;
    let mut kept = 0;
    let mut absent = 0;
    let mut deleted_generated: Vec<PathBuf> = Vec::new();
    let mut deleted_config: Vec<PathBuf> = Vec::new();

    for candidate in &candidates {
        let path = candidate.path();
        if !path.exists() {
            if path.is_symlink() {
                // Present on disk, its target gone. The manifest path
                // stopped calling this "already absent" for issue #348's
                // 3rd bullet; the same reasoning holds here word for word
                // — a dangling link is not an absence, and its content
                // can never match what `link` would generate, so the
                // guard's own conservative "kept" outcome is the right
                // one. The fallback is the *degraded* mode, where odd
                // trees are more likely, not less.
                let w = crate::cli::style::warn();
                anstream::println!(
                    "{w}  kept {} (broken symlink — its target is missing, so its \
                     content can't be compared){w:#}",
                    path.display()
                );
                kept += 1;
            } else {
                absent += 1;
            }
            continue;
        }

        match candidate {
            Candidate::AlwaysDelete { .. } => {
                std::fs::remove_file(path)?;
                let m = crate::cli::style::muted();
                anstream::println!("{m}  deleted {}{m:#}", path.display());
                deleted += 1;
                deleted_config.push(path.to_path_buf());
            }
            Candidate::Generated { expected, .. } => {
                if content_matches(path, expected) {
                    std::fs::remove_file(path)?;
                    let m = crate::cli::style::muted();
                    anstream::println!("{m}  deleted {}{m:#}", path.display());
                    deleted += 1;
                    deleted_generated.push(path.to_path_buf());
                } else {
                    let w = crate::cli::style::warn();
                    anstream::println!("{w}  kept {} (content differs){w:#}", path.display());
                    kept += 1;
                }
            }
        }
    }

    // Clean up empty ancestor directories left behind — bounded so the
    // cascade can never remove the target's own root directory (issue
    // #338 case 1's second half). Linker-generated paths are bounded by
    // `target_root` (e.g. `.claude/`); the project config file (if
    // removed) is bounded by the project root instead, since it lives
    // outside the target's tree entirely.
    for path in &deleted_generated {
        if let Some(parent) = path.parent() {
            remove_empty_ancestors(parent, &target_root);
        }
    }
    for path in &deleted_config {
        if let Some(parent) = path.parent() {
            remove_empty_ancestors(parent, &root);
        }
    }

    let o = crate::cli::style::ok();
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    anstream::println!(
        "\n{o}Unlinked{o:#} {a}'{}'{a:#}: {m}{} deleted, {} kept, {} already \
         absent.{m:#}",
        target_name,
        deleted,
        kept,
        absent
    );
    if kept > 0 {
        anstream::println!(
            "{m}  Kept files were left in place for the reason given on each line \
             above: content that differs from what link would generate now \
             (possibly edited since linking, or linked with different options — \
             e.g. --model, or an interactive prompt answer; unlink cannot tell \
             which), or a broken symlink whose content cannot be compared at all. \
             Remove them by hand if you no longer want them.{m:#}"
        );
    }

    Ok(())
}

/// Whether `path`'s on-disk bytes are identical to `expected`. Exact byte
/// comparison on purpose — no whitespace or line-ending normalisation. A
/// read failure (permissions, race with an external delete, ...) is treated
/// as "does not match": erring toward keeping a file is the whole point of
/// this guard.
fn content_matches(path: &Path, expected: &[u8]) -> bool {
    std::fs::read(path)
        .map(|actual| actual == expected)
        .unwrap_or(false)
}

/// Collect every file under a skill's *source* directory, keyed by its path
/// relative to that directory, together with its bytes. This mirrors
/// exactly what `link` copies into `<output_dir>/skills/<name>/` (see
/// `cli::link::collect_dir_files`) — including its valid-UTF-8-only gate: a
/// binary asset (e.g. `logo.png`) that `link` silently skips must be
/// skipped here too, or it would surface as a destination candidate that
/// was never actually written, inflating the "already absent" count with a
/// path that was never a real deletion candidate. The relative paths
/// returned here — and only those — are eligible for `unlink` to reclaim. A
/// file in the destination whose relative path isn't in this list was
/// placed there by the user after linking and must never be touched.
fn collect_source_files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    collect_source_files_recursive(dir, dir, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn collect_source_files_recursive(
    base: &Path,
    current: &Path,
    files: &mut Vec<(PathBuf, Vec<u8>)>,
) {
    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_source_files_recursive(base, &path, files);
        } else if path.is_file()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            let relative = path.strip_prefix(base).unwrap_or(&path).to_path_buf();
            files.push((relative, content.into_bytes()));
        }
    }
}

/// Walk up from `path` removing empty directories, stopping at `stop_at`
/// (exclusive: `stop_at` itself is never removed, no matter how empty it
/// is). Callers pass the boundary that must survive — the target's root
/// directory for linker-generated paths, the project root for the config
/// file — so this function has no target-specific knowledge of its own.
fn remove_empty_ancestors(path: &Path, stop_at: &Path) {
    let mut current = path.to_path_buf();
    while current.starts_with(stop_at) && current != stop_at {
        if std::fs::read_dir(&current)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false)
        {
            if std::fs::remove_dir(&current).is_err() {
                break;
            }
        } else {
            break;
        }
        match current.parent() {
            Some(p) => current = p.to_path_buf(),
            None => break,
        }
    }
}
