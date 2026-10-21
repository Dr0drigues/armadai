use std::path::{Path, PathBuf};

use crate::compile;
use crate::compile::patch::{self, PatchKind};
use crate::linker;

pub fn execute(
    targets: Vec<crate::linker::LinkTarget>,
    dry_run: bool,
    output: Option<PathBuf>,
    agents_filter: Option<Vec<String>>,
) -> anyhow::Result<()> {
    let root = compile::find_project(&std::env::current_dir()?)?;
    // targets and outputs only: a project that no longer validates can still be unlinked
    let project = armadai_model::load_project(&root).project;
    let targets: Vec<String> = match &project {
        Some(p) => crate::cli::link::targets_of(&targets, p),
        None if !targets.is_empty() => targets.iter().map(|t| t.to_string()).collect(),
        None => vec!["claude".into()],
    };
    if output.is_some() && targets.len() > 1 {
        anyhow::bail!("--output names one directory: unlink one target at a time with it");
    }
    for target_name in targets {
        let output_dir = match &project {
            Some(p) => crate::cli::link::output_of(&target_name, output.as_deref(), p),
            None => output
                .clone()
                .unwrap_or_else(|| PathBuf::from(compile::default_output(&target_name))),
        };
        compile::check_output(&root, &output_dir, output.is_some())?;
        // The target's own root, independently computed: the manifest's declared
        // `root` is trusted only once confirmed against it (design review R1).
        let target_root = root.join(&output_dir);
        match linker::manifest::lookup_target(&root, &target_name) {
            linker::manifest::Lookup::Found(target_manifest)
                if linker::manifest::root_confirmed(&root, &target_root, &target_manifest.root) =>
            {
                unlink_from_manifest(
                    &root,
                    &target_name,
                    target_manifest,
                    dry_run,
                    agents_filter.as_deref(),
                )?
            }
            lookup => {
                let w = crate::cli::style::warn();
                if matches!(lookup, linker::manifest::Lookup::Found(_)) {
                    anstream::eprintln!(
                        "{w}  The link manifest for target '{}' declares a root that \
                         doesn't match this project's current output directory — refusing \
                         to trust it and falling back to the content-match guard. \
                         This can happen if the manifest was hand-edited, corrupted, or \
                         copied from another project.{w:#}",
                        target_name
                    );
                } else {
                    anstream::eprintln!(
                        "{w}  No link manifest found for target '{}' — falling back to the \
                         content-match guard: a file is only removed when its on-disk \
                         content still byte-for-byte matches what `link` would generate \
                         right now. Re-run `link` to write a manifest.{w:#}",
                        target_name
                    );
                }
                unlink_via_fallback(
                    &root,
                    &target_name,
                    &output_dir,
                    dry_run,
                    agents_filter.as_deref(),
                )?
            }
        }
    }
    Ok(())
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
    agents_filter: Option<&[String]>,
) -> anyhow::Result<()> {
    let linker::manifest::TargetManifest {
        root: target_root,
        created_dirs,
        entries,
        patched,
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

    if dry_run {
        print_manifest_dry_run(
            root,
            target_name,
            &target_root,
            &relevant,
            &entries,
            &created_dirs,
            &patched,
        )?;
        return Ok(());
    }

    // a shared file unlink could not undo stops it before any removal
    if agents_filter.is_none() {
        for f in patched
            .iter()
            .filter(|f| patch_is_trusted(root, &target_root, f))
        {
            patch::preflight_remove(root, &target_root, f)?;
        }
    }

    let mut deleted = 0;
    let mut kept = 0;
    let mut absent = 0;
    let mut untrusted = 0;

    for entry in &relevant {
        if let Some(cause) = linker::manifest::diagnose_trust_failure(
            root,
            trust_root(entry, &target_root),
            &entry.path,
        ) {
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

    // Files shared with the user: only what link added comes out (`--agents` never
    // narrows them, like skills). A recorded path outside its place is refused.
    if agents_filter.is_none() {
        for f in &patched {
            if !patch_is_trusted(root, &target_root, f) {
                let e = crate::cli::style::err();
                anstream::eprintln!(
                    "{e}  refused: patched file '{}' for target '{}' is not where link \
                     patches; the manifest may be corrupt or forged{e:#}",
                    f.path.display(),
                    target_name
                );
                untrusted += 1;
                continue;
            }
            patch::remove_patch(root, &target_root, f)?;
            let m = crate::cli::style::muted();
            anstream::println!("{m}  unpatched {}{m:#}", root.join(&f.path).display());
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
            "{m}  Kept files were left in place for the reason given on each line above: \
             {KEPT_FILE_REASONS}. Remove them by hand if you no longer want them.{m:#}"
        );
    }
    // undone: the record goes too (kept files are no longer link's), unless a
    // refusal left something of link's in place; `--agents` undid only a part
    if untrusted == 0 && agents_filter.is_none() {
        linker::manifest::remove_target(root, target_name)?;
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

/// Why the manifest path can leave a file in place — one list, shared by
/// the real pass and its `--dry-run` preview so the two cannot drift into
/// two different accounts of the same set.
///
/// Open-ended on purpose (issue #348, third round): the closed list of
/// three it replaces omitted the broken-symlink outcome the very same
/// code path prints, so a user reading the footer could not find their
/// own case in it. The fallback's footer had already been opened for
/// exactly this reason.
const KEPT_FILE_REASONS: &str = "hand-written (link recorded them as skipped), content \
     that no longer matches what link wrote, a file that could not be verified, or a \
     broken symlink whose content cannot be compared at all";

/// Human-readable explanation for one [`linker::manifest::TrustFailure`]
/// cause — shared by the real pass and its `--dry-run` preview so the two
/// never phrase the same cause two different ways.
///
/// Neither wording dates the fault (issue #348, third round): the earlier
/// `FilesystemDiverged` text said the filesystem "has changed since link
/// ran", which is a claim the code cannot make. All it compares is the
/// recorded text against the filesystem *as it is now* — it keeps no
/// snapshot of how the filesystem looked at link time, so it can name
/// which side puts the path where it is, never when that became true. The
/// symlink may well have been there before `link` ever ran.
fn trust_failure_reason(cause: linker::manifest::TrustFailure) -> &'static str {
    match cause {
        linker::manifest::TrustFailure::ManifestEscapesRoot => {
            "the manifest may be corrupt or forged"
        }
        linker::manifest::TrustFailure::FilesystemDiverged => {
            "the manifest's own text does not put it there — something on the \
             filesystem does, most likely a symlink along the path; inspect the \
             target directory before retrying"
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
/// literally records `.claude` — under any spelling of it — *names* the
/// root, while a manifest that correctly records `.claude/agents` and
/// finds a symlink collapsing it onto `.claude` *resolves onto* it.
/// Saying "names" for the second blames the manifest for text it never
/// contained; saying "resolves onto" for the first sends the user to
/// inspect a filesystem where nothing is wrong.
///
/// "resolves onto", not "now resolves onto": the adverb dated a change
/// this code cannot observe — see [`trust_failure_reason`].
fn target_root_relation(cause: linker::manifest::TrustFailure) -> &'static str {
    match cause {
        linker::manifest::TrustFailure::ManifestEscapesRoot => "names",
        linker::manifest::TrustFailure::FilesystemDiverged => "resolves onto",
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
/// "eligible for cleanup", not "would be removed": a directory the user
/// deleted by hand still counts here and the real pass then quietly
/// touches nothing. Overstating that as an identical filesystem check is
/// what this doc used to do.
///
/// "eligible", not "recorded" (issue #348, third round): the counter is
/// the number of directories that passed
/// [`linker::manifest::decide_created_dir`], which is neither how many
/// the manifest records nor how many would actually be removed — a
/// manifest recording one directory that the guard refuses used to print
/// "0 directories recorded for cleanup", denying a record it holds.
fn print_manifest_dry_run(
    root: &Path,
    target_name: &str,
    target_root: &Path,
    entries: &[&linker::manifest::ManifestEntry],
    all_entries: &[linker::manifest::ManifestEntry],
    created_dirs: &[PathBuf],
    patched: &[patch::PatchedFile],
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
        if let Some(cause) = linker::manifest::diagnose_trust_failure(
            root,
            trust_root(entry, target_root),
            &entry.path,
        ) {
            let reason = trust_failure_reason(cause);
            // On **stderr**, like the refusal the real pass prints for the
            // same item (issue #348, third round): the preview must be
            // readable the same way the run it previews is. As a
            // `println!` a caller journalling `2>errors.log` got the
            // reasons from a real pass and nothing at all from its
            // preview, while one parsing stdout got the opposite.
            anstream::eprintln!(
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

    for f in patched {
        if patch_is_trusted(root, target_root, f) {
            let what = match f.kind {
                PatchKind::SettingsHooks(_) => "ArmadAI hooks",
                PatchKind::MarkdownBlock => "ArmadAI block",
                PatchKind::JsonKeys(_) => "ArmadAI keys",
            };
            anstream::println!(
                "{m}  {} ({what} would be removed){m:#}",
                root.join(&f.path).display()
            );
        } else {
            anstream::eprintln!(
                "{e}  {} (would refuse — not where link patches; the manifest may be \
                 corrupt or forged){e:#}",
                f.path.display()
            );
            untrusted += 1;
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
                anstream::eprintln!(
                    "{e}  {} (would refuse — {relation} the target's own root; \
                     {reason}){e:#}",
                    dir.display()
                );
                untrusted += 1;
            }
            linker::manifest::CreatedDirDecision::Untrusted(cause) => {
                let reason = trust_failure_reason(cause);
                anstream::eprintln!(
                    "{e}  {} (would refuse — outside the trusted root; {reason}){e:#}",
                    dir.display()
                );
                untrusted += 1;
            }
            linker::manifest::CreatedDirDecision::Implausible => {
                anstream::eprintln!(
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
         ({} director{} eligible for cleanup).{m:#}",
        would_remove,
        would_keep,
        absent,
        would_clean_dirs,
        if would_clean_dirs == 1 { "y" } else { "ies" }
    );
    if would_keep > 0 {
        anstream::println!(
            "{m}  Files listed as kept would be left in place for the reason given on \
             each line above: {KEPT_FILE_REASONS}. Remove them by hand if you no longer \
             want them.{m:#}"
        );
    }
    if untrusted > 0 {
        // Same sentence the real pass prints, on the same stream, with the
        // one word that has to differ (issue #348, third round: the two
        // summaries had drifted into two formulations of the same fact,
        // which is what extracting `trust_failure_reason` and
        // `target_root_relation` was supposed to stop).
        anstream::eprintln!(
            "{e}  {} manifest item(s) would be refused and left untouched; each \
             refusal above names the item and why.{e:#}",
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

/// Where a manifest entry must resolve: native files live anywhere in the project
/// (outside `.armadai/`), everything else under the target's own root.
fn trust_root<'a>(entry: &linker::manifest::ManifestEntry, target_root: &'a Path) -> &'a Path {
    if entry.produced_by.kind == linker::manifest::ProducedByKind::Native
        && !entry.path.starts_with(".armadai")
    {
        Path::new(".")
    } else {
        target_root
    }
}

/// Link patches the root `CLAUDE.md` and `opencode.json`, and files under the
/// target's own root only.
fn patch_is_trusted(root: &Path, target_root: &Path, f: &patch::PatchedFile) -> bool {
    match f.kind {
        PatchKind::MarkdownBlock => f.path == Path::new("CLAUDE.md"),
        PatchKind::JsonKeys(_) => f.path == Path::new("opencode.json"),
        PatchKind::SettingsHooks(_) => linker::manifest::is_trusted(root, target_root, &f.path),
    }
}

/// Without a manifest (review R-I2): recompile the project and remove only the files
/// whose bytes still equal what `link` would write now. The files shared with the
/// user (`settings.json`, the root `CLAUDE.md`, `opencode.json`) are never touched: without the
/// record of what link added, nothing there is provably link's.
fn unlink_via_fallback(
    root: &Path,
    target_name: &str,
    output_dir: &Path,
    dry_run: bool,
    agents_filter: Option<&[String]>,
) -> anyhow::Result<()> {
    let (project, _) = armadai_model::load_checked(root).map_err(|ds| {
        for d in &ds {
            eprintln!("{}", d.render());
        }
        anyhow::anyhow!("no link manifest and the project is invalid: nothing to compare against")
    })?;
    let plan = compile::compiler_for(target_name, output_dir)?.compile(&project, root)?;
    let target_root = root.join(output_dir);
    let candidates: Vec<(PathBuf, Vec<u8>)> = (plan.artifacts.into_iter())
        .filter(|a| match agents_filter {
            Some(f) => a.kind != compile::ArtifactKind::Agent || f.contains(&a.produced_by.name),
            None => true,
        })
        .map(|a| (target_root.join(a.path), a.content))
        .collect();

    let m = crate::cli::style::muted();
    let w = crate::cli::style::warn();
    let (mut deleted, mut kept, mut absent) = (0, 0, 0);
    if dry_run {
        let h = crate::cli::style::header();
        anstream::println!("{h}Dry run{h:#} — files that would be removed for '{target_name}':\n");
    }
    for (path, expected) in &candidates {
        if !path.exists() {
            if path.is_symlink() {
                anstream::println!(
                    "{w}  kept {} (broken symlink — its content can't be compared){w:#}",
                    path.display()
                );
                kept += 1;
            } else {
                absent += 1;
            }
            continue;
        }
        if !content_matches(path, expected) {
            anstream::println!("{w}  kept {} (content differs){w:#}", path.display());
            kept += 1;
            continue;
        }
        if dry_run {
            anstream::println!("{m}  {}{m:#}", path.display());
        } else {
            std::fs::remove_file(path)?;
            anstream::println!("{m}  deleted {}{m:#}", path.display());
            if let Some(parent) = path.parent() {
                remove_empty_ancestors(parent, &target_root);
            }
        }
        deleted += 1;
    }
    if !plan.blocks.is_empty() || target_name == "claude" {
        anstream::eprintln!(
            "{w}  hooks/CLAUDE.md block not removed: no manifest records what link added{w:#}"
        );
    }
    if dry_run {
        anstream::println!(
            "\n{m}  {deleted} would be removed, {kept} would be kept, {absent} already absent.{m:#}"
        );
    } else {
        let o = crate::cli::style::ok();
        anstream::println!(
            "\n{o}Unlinked{o:#} '{target_name}': {m}{deleted} deleted, {kept} kept, {absent} already absent.{m:#}"
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
