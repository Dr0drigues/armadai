//! Remote starter registries (B2 Lot B): fetch starter packs from git (Lot 1)
//! or archive (Lot 2) sources into a cache, discovered by the existing
//! `StarterPack` convention (+ an optional `armadai-starters.yaml` manifest).
//!
//! Cache-dir resolution and pack discovery (`starters_cache_dir`,
//! `discover_packs`) live in `armadai_core::starter` — pure filesystem/YAML
//! logic with no coupling to fetching, re-exported here for the fetchers
//! below (which own the git/archive sync path).

use std::path::{Path, PathBuf};

use armadai_core::project::find_project_config;
use armadai_core::registries::{
    RegistrySource, SourceKind, cache_key, load_user_registries, mask_url_secrets, redact_url,
};
#[cfg(test)]
use armadai_core::starter::discover_packs;
pub use armadai_core::starter::starters_cache_dir;

/// Cache dir for one source URL.
pub fn source_cache_dir(url: &str) -> PathBuf {
    starters_cache_dir().join(cache_key(url))
}

/// Gather typed starter registry sources (user ∪ project), deduplicated by
/// URL. Keeps the full [`RegistrySource`] so `sync_starters` can dispatch via
/// `resolved_kind()` with an explicit `kind:` honored when set.
pub fn effective_starter_sources() -> Vec<RegistrySource> {
    let user = load_user_registries();
    let project = find_project_config()
        .map(|(_, cfg)| cfg)
        .and_then(|cfg| cfg.registries);

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let project_starters = project.map(|cfg| cfg.starters).unwrap_or_default();
    for source in user.starters.into_iter().chain(project_starters) {
        if seen.insert(source.url.clone()) {
            out.push(source);
        }
    }
    out
}

pub trait StarterFetcher {
    fn fetch(&self, url: &str, dest: &Path) -> anyhow::Result<()>;
}

/// Git-backed fetcher: clone if absent, else pull. Shells out to `git`.
pub struct GitFetcher;

impl StarterFetcher for GitFetcher {
    fn fetch(&self, url: &str, dest: &Path) -> anyhow::Result<()> {
        if dest.join(".git").is_dir() {
            run_git(&[
                "-C",
                dest.to_str().unwrap_or("."),
                "pull",
                "--ff-only",
                "-q",
            ])?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            run_git(&[
                "clone",
                "--depth",
                "1",
                "-q",
                url,
                dest.to_str().unwrap_or("."),
            ])?;
        }
        Ok(())
    }
}

fn run_git(args: &[&str]) -> anyhow::Result<()> {
    let out = std::process::Command::new("git").args(args).output()?;
    if !out.status.success() {
        anyhow::bail!(git_failure(args, &String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

/// The error for a failed `git` run, with any URL credential in argv or stderr masked.
fn git_failure(args: &[&str], stderr: &str) -> String {
    let msg = format!("git {} failed: {}", args.join(" "), stderr.trim());
    args.iter()
        .fold(msg, |msg, arg| mask_url_secrets(&msg, arg))
}

/// Check whether an archive entry's path is safe to extract under a fixed
/// `dest` directory (zip-slip guard).
///
/// Rejects:
/// - an absolute path (leading `/`, i.e. a `RootDir` path component),
/// - any `..` (`ParentDir`) path component, at any depth,
/// - Windows-style separators (`\`) or a drive-absolute prefix (`C:\...`,
///   `C:/...`) — we only expect POSIX-style entries from `tar -tzf`/
///   `unzip -Z1`, so anything shaped like this is already suspicious and
///   `Path` on a non-Windows host wouldn't otherwise recognize it as
///   absolute.
///
/// Pure and testable in isolation from the actual shell-out extraction.
fn archive_entry_is_safe(name: &str) -> bool {
    if name.is_empty() || name.contains('\\') {
        return false;
    }
    let mut chars = name.chars();
    if let (Some(letter), Some(':')) = (chars.next(), chars.next())
        && letter.is_ascii_alphabetic()
    {
        // Drive-absolute, e.g. `C:/foo` — not a `RootDir` component on a
        // non-Windows host, so it must be checked explicitly.
        return false;
    }
    !Path::new(name).components().any(|c| {
        matches!(
            c,
            std::path::Component::RootDir | std::path::Component::ParentDir
        )
    })
}

/// `program` with stdin closed: `tar`, `unzip` and `curl` never wait on the terminal.
fn tool(program: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    cmd.stdin(std::process::Stdio::null());
    cmd
}

/// List entry paths inside an archive without extracting it, so they can be
/// validated by [`archive_entry_is_safe`] before anything is written to disk.
///
/// The listing is read as it comes: past `limits` (entries, length of one name,
/// total length of the names) the lister is killed and the archive refused, so a
/// small archive declaring huge names never fills memory.
fn list_archive_entries(
    archive: &Path,
    is_zip: bool,
    label: &str,
    limits: ExtractLimits,
) -> anyhow::Result<Vec<String>> {
    use std::io::{BufRead, Read};
    let mut cmd = if is_zip {
        let mut c = tool("unzip");
        c.arg("-Z1").arg(archive);
        c
    } else {
        let mut c = tool("tar");
        c.arg("-tzf").arg(archive);
        c
    };
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    // Drained apart (a full stderr pipe would stall the lister); its first KiB is kept.
    let mut stderr = child.stderr.take().expect("piped stderr");
    let stderr = std::thread::spawn(move || {
        let mut kept = Vec::new();
        let _ = (&mut stderr).take(1024).read_to_end(&mut kept);
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
        kept
    });
    let mut stdout = std::io::BufReader::new(child.stdout.take().expect("piped stdout"));
    let (mut entries, mut names_bytes, mut line) = (Vec::new(), 0usize, Vec::new());
    let refusal = loop {
        line.clear();
        let max_line = limits.name_bytes as u64 + 1; // the name and its newline
        // A read error refuses too, so the lister is still killed and reaped below.
        match (&mut stdout).take(max_line).read_until(b'\n', &mut line) {
            Ok(0) => break None,
            Ok(_) => {}
            Err(e) => break Some(format!("cannot read its listing: {e}")),
        }
        if line.pop_if(|b| *b == b'\n').is_none() && line.len() > limits.name_bytes {
            break Some(format!(
                "an entry name is longer than {} bytes",
                limits.name_bytes
            ));
        }
        if line.is_empty() {
            continue;
        }
        names_bytes += line.len();
        if names_bytes > limits.names_bytes {
            break Some(format!(
                "its entry names exceed {} bytes in total",
                limits.names_bytes
            ));
        }
        if entries.len() == limits.entries {
            break Some(format!("more than the {} entries allowed", limits.entries));
        }
        entries.push(String::from_utf8_lossy(&line).into_owned());
    };
    if let Some(reason) = refusal {
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!("refusing archive {label}: {reason}");
    }
    let status = child.wait()?;
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        anyhow::bail!(
            "failed to list archive entries for {label}: {}",
            String::from_utf8_lossy(&stderr)
        );
    }
    Ok(entries)
}

/// Extract a downloaded archive (.tar.gz/.tgz or .zip) into `dest` by shelling
/// out to system `tar`/`unzip` (no extra crate). `dest` is created.
///
/// Before extracting, every entry is listed and validated with
/// [`archive_entry_is_safe`]: a malicious archive with a `../../etc/x` or
/// absolute-path entry (zip-slip) is rejected up front rather than allowed to
/// write outside `dest`. `label` names the archive in messages. An archive over
/// `limits` (entry count, then total file size) is refused before extraction.
fn extract_archive(
    archive: &Path,
    dest: &Path,
    label: &str,
    limits: ExtractLimits,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    let name = archive.to_string_lossy().to_lowercase();
    let is_zip = name.ends_with(".zip");

    let entries = list_archive_entries(archive, is_zip, label, limits)?;
    check_extracted_size(archive, is_zip, limits.bytes, label)?;
    if let Some(unsafe_entry) = entries.iter().find(|e| !archive_entry_is_safe(e)) {
        anyhow::bail!(
            "refusing to extract archive {label}: unsafe entry path '{unsafe_entry}' (zip-slip guard)"
        );
    }

    // `-P ""`: an encrypted entry fails instead of prompting for a password on the tty.
    let status = if is_zip {
        tool("unzip")
            .args([
                "-q",
                "-o",
                "-P",
                "",
                archive.to_str().unwrap_or(""),
                "-d",
                dest.to_str().unwrap_or(""),
            ])
            .status()?
    } else {
        // .tar.gz / .tgz
        tool("tar")
            .args([
                "-xzf",
                archive.to_str().unwrap_or(""),
                "-C",
                dest.to_str().unwrap_or(""),
            ])
            .status()?
    };
    if !status.success() {
        anyhow::bail!("failed to extract archive {label}");
    }
    Ok(())
}

/// Upper bounds on what one archive may hold once extracted.
#[derive(Clone, Copy)]
struct ExtractLimits {
    /// Total size of its files.
    bytes: u64,
    /// Number of entries (files, directories, links): each costs an inode.
    entries: usize,
    /// Length of one entry name, in bytes.
    name_bytes: usize,
    /// Total length of the entry names: what the listing keeps in memory.
    names_bytes: usize,
}

const EXTRACT_LIMITS: ExtractLimits = ExtractLimits {
    bytes: 1 << 30, // 1 GiB
    entries: 100_000,
    // PATH_MAX on Linux (macOS: 1024): a longer name cannot be extracted anyway.
    name_bytes: 4096,
    // 100,000 names of 670 bytes on average; real packs hold names under 200 bytes.
    names_bytes: 64 << 20, // 64 MiB
};

/// Refuse an archive whose files add up to more than `max_bytes`, by streaming their
/// contents (`tar -xO`, `unzip -p`) and counting, before anything is written to disk.
fn check_extracted_size(
    archive: &Path,
    is_zip: bool,
    max_bytes: u64,
    label: &str,
) -> anyhow::Result<()> {
    use std::io::Read;
    let mut cmd = if is_zip {
        let mut c = tool("unzip");
        c.args(["-p", "-P", ""]).arg(archive);
        c
    } else {
        let mut c = tool("tar");
        c.arg("-xzOf").arg(archive);
        c
    };
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let (mut total, mut buf) = (0u64, vec![0u8; 64 * 1024]);
    loop {
        let n = stdout.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > max_bytes {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "refusing archive {label}: its files exceed {max_bytes} bytes once extracted"
            );
        }
    }
    // A corrupt archive is reported by the extraction itself.
    let _ = child.wait();
    Ok(())
}

/// Archive-backed fetcher: download via `curl`, then extract via `tar`/`unzip`.
pub struct ArchiveFetcher;

impl StarterFetcher for ArchiveFetcher {
    fn fetch(&self, url: &str, dest: &Path) -> anyhow::Result<()> {
        fetch_archive("curl", url, dest, &std::env::temp_dir())
    }
}

/// Download into a private temp dir under `tmp_root`, extract into a staging dir next
/// to `dest`, check it, then swap it in: a failed sync leaves the previous cache intact
/// and a partial pack is never visible. Both temp dirs are removed on return; ones a
/// killed sync left behind are swept on the next sync (see [`sweep_orphans`]).
fn fetch_archive(curl: &str, url: &str, dest: &Path, tmp_root: &Path) -> anyhow::Result<()> {
    let shown = redact_url(url);
    let parent = dest
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no parent directory for {}", dest.display()))?;
    // `+` never occurs in a cache key: no other source's prefix can match this one.
    let staging_prefix = format!(
        "{STAGING_PREFIX}{}+",
        dest.file_name().unwrap_or_default().to_string_lossy()
    );
    sweep_orphans(parent, STAGING_PREFIX, true);
    sweep_orphans(tmp_root, "armadai-dl-", false);

    let download = private_tempdir("armadai-dl-", tmp_root)?;
    // The extension makes extract_archive pick tar vs unzip.
    let src = download
        .path()
        .join(format!("archive.{}", archive_ext(url)));
    download_to(curl, url, &src)?;

    // Same filesystem as `dest`, so the swap is a rename; the leading dot keeps it
    // out of pack discovery even if a crash leaves it behind.
    std::fs::create_dir_all(parent)
        .and_then(|()| private_tempdir(&staging_prefix, parent))
        .map_err(|e| anyhow::anyhow!("cannot stage {shown} in {}: {e}", parent.display()))
        .and_then(|staging| {
            let new = staging.path().join("new");
            extract_archive(&src, &new, &shown, EXTRACT_LIMITS)?;
            reject_escaping_links(&new, &shown)?;
            swap_in(&new, dest, &staging.path().join("old"))
                .map_err(|e| anyhow::anyhow!("could not install archive {shown}: {e}"))
        })
}

/// Replace `dest` (if any) by `new`, parking the old copy at `old`; if `new` cannot
/// take its place, the old copy goes back. A symlinked `dest` is replaced, not followed.
fn swap_in(new: &Path, dest: &Path, old: &Path) -> anyhow::Result<()> {
    let had_old = std::fs::symlink_metadata(dest).is_ok();
    if had_old {
        std::fs::rename(dest, old)
            .map_err(|e| anyhow::anyhow!("cannot move {} aside: {e}", dest.display()))?;
    }
    if let Err(e) = std::fs::rename(new, dest) {
        if had_old {
            let _ = std::fs::rename(old, dest);
        }
        anyhow::bail!("cannot move the new copy to {}: {e}", dest.display());
    }
    Ok(())
}

/// How old a temp dir must be before a sync treats it as left by a killed run.
const ORPHAN_AGE: std::time::Duration = std::time::Duration::from_secs(3600);

/// Name prefix of a sync's staging dir: `<prefix><cache key>+<random>`.
const STAGING_PREFIX: &str = ".armadai-staging-";

/// Remove `prefix*` dirs in `dir` older than [`ORPHAN_AGE`], whatever source left them.
/// With `restore`, they are staging dirs: a source whose cache dir (`<cache key>` in
/// their name) is missing first gets back the copy an interrupted swap parked in `old`.
fn sweep_orphans(dir: &Path, prefix: &str, restore: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > ORPHAN_AGE));
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(prefix).filter(|_| stale) else {
            continue;
        };
        // `+` never occurs in a cache key.
        if restore && let Some((key, _)) = rest.split_once('+') {
            let dest = dir.join(key);
            if !key.is_empty() && std::fs::symlink_metadata(&dest).is_err() {
                let _ = std::fs::rename(path.join("old"), &dest);
            }
        }
        let _ = std::fs::remove_dir_all(&path);
    }
}

/// A uniquely named temp dir, private (0700) on unix, removed on drop.
fn private_tempdir(prefix: &str, parent: &Path) -> std::io::Result<tempfile::TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    #[cfg(unix)]
    builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
    builder.tempdir_in(parent)
}

/// Refuse an extracted tree holding a symlink that resolves outside `root`
/// (or nowhere): a pack must not reach the rest of the disk through its cache.
fn reject_escaping_links(root: &Path, label: &str) -> anyhow::Result<()> {
    let root = root.canonicalize()?;
    let mut dirs = vec![root.clone()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            let kind = std::fs::symlink_metadata(&path)?.file_type();
            if kind.is_symlink() {
                let link = path.strip_prefix(&root).unwrap_or(&path).display();
                match path.canonicalize() {
                    Err(_) => anyhow::bail!("refusing archive {label}: link '{link}' is dangling"),
                    Ok(t) if !t.starts_with(&root) => anyhow::bail!(
                        "refusing archive {label}: link '{link}' points outside the archive"
                    ),
                    Ok(_) => {}
                }
            } else if kind.is_dir() {
                dirs.push(path);
            }
        }
    }
    Ok(())
}

fn archive_ext(url: &str) -> &'static str {
    let u = url.to_lowercase();
    let u = u.split(['?', '#']).next().unwrap_or_default();
    if u.ends_with(".zip") {
        "zip"
    } else if u.ends_with(".tgz") {
        "tgz"
    } else {
        "tar.gz"
    }
}

/// Upper bounds for one archive download (curl `--max-time`, `--max-filesize`).
const DOWNLOAD_MAX_SECS: &str = "300";
const DOWNLOAD_MAX_BYTES: &str = "104857600"; // 100 MiB

/// Download `url` to `out` with the `curl` program (`update` uses it too).
/// HTTPS only, redirects followed (still HTTPS), proxies from the standard env vars.
/// `-q` (must stay first) ignores `.curlrc`, which could e.g. turn TLS checks off;
/// `--globoff` keeps `{}`/`[]` in the URL literal instead of expanding to many requests.
/// On any failure `out` is removed, so a truncated file is never extracted.
pub(crate) fn download_to(curl: &str, url: &str, out: &Path) -> anyhow::Result<()> {
    let shown = redact_url(url);
    if !url
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
    {
        anyhow::bail!("download {shown} refused: archive sources must use an https:// URL");
    }
    let result = tool(curl)
        .args([
            "-q",
            "--globoff",
            "-fsSL",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-time",
            DOWNLOAD_MAX_SECS,
            "--max-filesize",
            DOWNLOAD_MAX_BYTES,
            "-o",
        ])
        .arg(out)
        .arg(url)
        .output();
    let output = match result {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => anyhow::bail!(
            "download {shown} failed: `{curl}` not found in PATH; install curl (bundled with Windows 10+, macOS and most Linux distributions)"
        ),
        Err(e) => anyhow::bail!("download {shown} failed: could not run `{curl}`: {e}"),
    };
    if !output.status.success() {
        let _ = std::fs::remove_file(out);
        anyhow::bail!(
            "download {shown} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Fetch one source into its cache dir. Git only in Lot 1; archive is Lot 2.
pub fn fetch_starter_source(source: &RegistrySource) -> anyhow::Result<PathBuf> {
    let dest = source_cache_dir(&source.url);
    match source.resolved_kind() {
        SourceKind::Git => {
            GitFetcher.fetch(&source.url, &dest)?;
            Ok(dest)
        }
        SourceKind::Archive => {
            ArchiveFetcher.fetch(&source.url, &dest)?;
            Ok(dest)
        }
    }
}

/// The warning for a source that failed to sync, with its credentials masked
/// in the URL and in the error (git echoes its argv and URL).
fn sync_warning(url: &str, err: &anyhow::Error) -> String {
    let line = format!("  warn: failed to sync starter source {url}: {err}");
    mask_url_secrets(&line, url)
}

/// Sync all sources; warn+continue on failure. Returns the dirs that synced OK.
pub fn sync_starters(sources: &[RegistrySource]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for s in sources {
        match fetch_starter_source(s) {
            Ok(d) => dirs.push(d),
            Err(e) => eprintln!("{}", sync_warning(&s.url, &e)),
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    #[test]
    fn discover_by_convention_scans_pack_yaml() {
        let tmp = std::env::temp_dir().join(format!("armadai-st-conv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        write(&tmp.join("rust-qa/pack.yaml"), "name: rust-qa\n");
        write(&tmp.join("nested/web/pack.yaml"), "name: web\n");
        write(&tmp.join("not-a-pack/readme.md"), "x");
        let packs = discover_packs(&tmp);
        let names: Vec<_> = packs
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"rust-qa".to_string()));
        assert!(names.contains(&"web".to_string()));
        assert!(!names.iter().any(|n| n == "not-a-pack"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discover_with_manifest_restricts_to_listed() {
        let tmp = std::env::temp_dir().join(format!("armadai-st-man-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        write(&tmp.join("rust-qa/pack.yaml"), "name: rust-qa\n");
        write(&tmp.join("hidden/pack.yaml"), "name: hidden\n");
        write(
            &tmp.join("armadai-starters.yaml"),
            "packs:\n  - path: rust-qa\n",
        );
        let packs = discover_packs(&tmp);
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].file_name().unwrap().to_string_lossy(), "rust-qa");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn archive_entry_is_safe_accepts_normal_nested_path() {
        assert!(archive_entry_is_safe("dir/sub/file.txt"));
    }

    #[test]
    fn archive_entry_is_safe_accepts_flat_path() {
        assert!(archive_entry_is_safe("a/b/c"));
    }

    #[test]
    fn archive_entry_is_safe_rejects_leading_parent_dir() {
        assert!(!archive_entry_is_safe("../escape"));
    }

    #[test]
    fn archive_entry_is_safe_rejects_embedded_parent_dir() {
        assert!(!archive_entry_is_safe("a/../../b"));
    }

    #[test]
    fn archive_entry_is_safe_rejects_absolute_path() {
        assert!(!archive_entry_is_safe("/abs"));
    }

    #[test]
    fn archive_entry_is_safe_rejects_windows_separator() {
        assert!(!archive_entry_is_safe("a\\..\\b"));
    }

    #[test]
    fn archive_entry_is_safe_rejects_windows_drive_absolute() {
        assert!(!archive_entry_is_safe("C:/windows/system32"));
        assert!(!archive_entry_is_safe("C:\\windows\\system32"));
    }

    #[test]
    fn archive_entry_is_safe_rejects_empty() {
        assert!(!archive_entry_is_safe(""));
    }

    #[test]
    fn discover_rejects_parent_traversal_in_manifest() {
        let tmp = std::env::temp_dir().join(format!("armadai-trav-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("ok")).unwrap();
        fs::write(tmp.join("ok/pack.yaml"), "name: ok\n").unwrap();
        fs::write(
            tmp.join("armadai-starters.yaml"),
            "packs:\n  - path: ../escape\n  - path: ok\n",
        )
        .unwrap();
        let packs = discover_packs(&tmp);
        // The `../escape` entry is rejected; only `ok` survives.
        assert_eq!(packs.len(), 1);
        assert!(packs[0].ends_with("ok"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn git_fetcher_clones_local_repo() {
        // Build a local git repo containing a pack, fetch it, discover the pack.
        // (Skip gracefully if `git` is unavailable.)
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let tmp = std::env::temp_dir().join(format!("armadai-st-git-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let repo = tmp.join("origin");
        write(&repo.join("demo/pack.yaml"), "name: demo\n");
        for args in [
            vec!["init", "-q"],
            vec!["add", "-A"],
            vec![
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-qm",
                "x",
            ],
        ] {
            std::process::Command::new("git")
                .current_dir(&repo)
                .args(&args)
                .output()
                .unwrap();
        }
        let dest = tmp.join("dest");
        GitFetcher.fetch(repo.to_str().unwrap(), &dest).unwrap();
        let packs = discover_packs(&dest);
        assert!(
            packs
                .iter()
                .any(|p| p.file_name().unwrap().to_string_lossy() == "demo")
        );
        let _ = fs::remove_dir_all(&tmp);
    }
}

#[cfg(test)]
pub(crate) mod archive_tests {
    use super::*;
    use std::fs;

    fn write(p: &std::path::Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    // Extract a local .tar.gz built with system `tar` and confirm packs discovered.
    #[test]
    fn archive_fetcher_extracts_local_targz() {
        if std::process::Command::new("tar")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let tmp = std::env::temp_dir().join(format!("armadai-arc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        // Build a source tree with a pack, tar it.
        let srcroot = tmp.join("srcroot");
        write(&srcroot.join("rust-qa/pack.yaml"), "name: rust-qa\n");
        let archive = tmp.join("packs.tar.gz");
        std::process::Command::new("tar")
            .args([
                "-czf",
                archive.to_str().unwrap(),
                "-C",
                srcroot.to_str().unwrap(),
                ".",
            ])
            .output()
            .unwrap();
        // Extract via the fetcher's extract helper (file:// or local path).
        let dest = tmp.join("dest");
        extract_archive(&archive, &dest, "test", EXTRACT_LIMITS).unwrap();
        let packs = discover_packs(&dest);
        assert!(
            packs
                .iter()
                .any(|p| p.file_name().unwrap().to_string_lossy() == "rust-qa")
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Runs `f` again while exec hits ETXTBSY: on Linux, a fork from a concurrent test
    /// can briefly hold the fake script's write handle open.
    #[cfg(unix)]
    pub(crate) fn retry_busy<T>(f: impl Fn() -> anyhow::Result<T>) -> anyhow::Result<T> {
        for _ in 0..20 {
            match f() {
                Err(e) if e.to_string().contains("Text file busy") => {
                    std::thread::sleep(std::time::Duration::from_millis(50))
                }
                r => return r,
            }
        }
        f()
    }

    /// A fake `curl` that records its args, writes `-o`'s target then runs `body`.
    #[cfg(unix)]
    pub(crate) fn fake_curl(dir: &Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("curl");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$(dirname \"$0\")/args\"\n\
                 [ /dev/fd/0 -ef /dev/null ] && echo null > \"$(dirname \"$0\")/stdin\"\n\
                 while [ $# -gt 1 ]; do [ \"$1\" = -o ] && out=$2; shift; done\n{body}\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script.to_str().unwrap().to_string()
    }

    #[cfg(unix)]
    #[test]
    fn download_writes_the_body_with_bounded_https_only_curl() {
        let dir = tempfile::tempdir().unwrap();
        let curl = fake_curl(dir.path(), "printf payload > \"$out\"");
        let out = dir.path().join("a.tar.gz");
        retry_busy(|| download_to(&curl, "https://example.com/a.tar.gz", &out)).unwrap();
        assert_eq!(fs::read_to_string(&out).unwrap(), "payload");
        let args = fs::read_to_string(dir.path().join("args")).unwrap();
        let args: Vec<_> = args.lines().collect();
        // `-q` only works as the very first argument.
        assert_eq!(args.first(), Some(&"-q"));
        for flag in [
            "--globoff",
            "-fsSL",
            "--proto",
            "--proto-redir",
            "--max-time",
            "--max-filesize",
        ] {
            assert!(args.contains(&flag), "missing {flag} in {args:?}");
        }
        assert_eq!(args.iter().filter(|a| **a == "=https").count(), 2);
        assert_eq!(args.last(), Some(&"https://example.com/a.tar.gz"));
        let stdin = fs::read_to_string(dir.path().join("stdin")).unwrap_or_default();
        assert_eq!(stdin, "null\n", "curl may read the terminal");
    }

    #[cfg(unix)]
    #[test]
    fn download_reports_http_errors() {
        let dir = tempfile::tempdir().unwrap();
        let curl = fake_curl(
            dir.path(),
            "echo 'curl: (22) The requested URL returned error: 404' >&2; exit 22",
        );
        let out = dir.path().join("a.tar.gz");
        let err = retry_busy(|| download_to(&curl, "https://example.com/a.tar.gz", &out))
            .unwrap_err()
            .to_string();
        assert!(err.contains("returned error: 404"), "{err}");
        assert!(err.contains("https://example.com/a.tar.gz"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn download_removes_a_truncated_file() {
        let dir = tempfile::tempdir().unwrap();
        let curl = fake_curl(
            dir.path(),
            "printf partial > \"$out\"; echo 'curl: (18) transfer closed' >&2; exit 18",
        );
        let out = dir.path().join("a.tar.gz");
        let err = retry_busy(|| download_to(&curl, "https://example.com/a.tar.gz", &out))
            .unwrap_err()
            .to_string();
        assert!(err.contains("transfer closed"), "{err}");
        assert!(!out.exists(), "truncated download left behind");
    }

    #[test]
    fn download_without_curl_says_to_install_it() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-curl");
        let err = download_to(
            missing.to_str().unwrap(),
            "https://example.com/a.tar.gz",
            &dir.path().join("a.tar.gz"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("not found in PATH"), "{err}");
        assert!(err.contains("install curl"), "{err}");
    }

    #[test]
    fn download_refuses_non_https_urls() {
        let dir = tempfile::tempdir().unwrap();
        // A program that would succeed if it were run: the refusal must come first.
        for url in ["http://example.com/a.tar.gz", "-o/etc/x https://e.com"] {
            let err = download_to("true", url, &dir.path().join("a.tar.gz"))
                .unwrap_err()
                .to_string();
            assert!(err.contains("must use an https:// URL"), "{url}: {err}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn download_accepts_an_uppercase_scheme() {
        let dir = tempfile::tempdir().unwrap();
        let curl = fake_curl(dir.path(), "printf payload > \"$out\"");
        let out = dir.path().join("a.tar.gz");
        retry_busy(|| download_to(&curl, "HTTPS://example.com/a.tar.gz", &out)).unwrap();
        assert_eq!(fs::read_to_string(&out).unwrap(), "payload");
    }

    #[cfg(unix)]
    #[test]
    fn download_masks_credentials_in_errors() {
        let dir = tempfile::tempdir().unwrap();
        let curl = fake_curl(dir.path(), "echo 'curl: (22) error: 401' >&2; exit 22");
        let out = dir.path().join("a.tar.gz");
        for url in [
            "https://user:s3cret@example.com/a.tar.gz",
            "http://user:s3cret@example.com/a.tar.gz",
        ] {
            let err = retry_busy(|| download_to(&curl, url, &out))
                .unwrap_err()
                .to_string();
            assert!(!err.contains("s3cret"), "{err}");
            assert!(err.contains("://***@example.com/a.tar.gz"), "{err}");
        }
    }

    #[test]
    fn git_failure_masks_credentials_in_argv_and_stderr() {
        let url = "https://u:s3cret@example.com/x.git";
        let msg = git_failure(
            &["clone", "-q", url, "/tmp/d"],
            "fatal: unable to access 'https://u:s3cret@example.com/x.git/'\n",
        );
        assert!(!msg.contains("s3cret"), "{msg}");
        assert!(
            msg.contains("git clone -q https://***@example.com/x.git /tmp/d failed"),
            "{msg}"
        );
        let url = "https://127.0.0.1:9/e.git?token=s3cretQ";
        let msg = git_failure(
            &["clone", url, "/tmp/d"],
            "fatal: unable to access 'https://127.0.0.1:9/e.git?token=s3cretQ/'",
        );
        assert!(!msg.contains("s3cretQ"), "{msg}");
    }

    #[test]
    fn archive_ext_ignores_the_query() {
        assert_eq!(archive_ext("https://e.com/a.zip?token=x.tar.gz"), "zip");
        assert_eq!(archive_ext("https://e.com/a.tgz#x"), "tgz");
    }

    #[test]
    fn sync_warning_masks_credentials_in_url_and_error() {
        let url = "https://u:s3cret@example.com/x.git?token=t0ken";
        let err = anyhow::anyhow!("git clone https://u:s3cret@example.com/x.git failed");
        let line = sync_warning(url, &err);
        assert!(!line.contains("s3cret"), "{line}");
        assert!(!line.contains("t0ken"), "{line}");
        assert!(
            line.contains("source https://***@example.com/x.git?***: git clone https://***@"),
            "{line}"
        );
    }

    /// A .tar.gz of what `build` writes into a fresh source dir.
    #[cfg(unix)]
    fn tar_gz(dir: &Path, name: &str, build: impl Fn(&Path)) -> std::path::PathBuf {
        let root = dir.join(format!("{name}-src"));
        fs::create_dir_all(&root).unwrap();
        build(&root);
        let archive = dir.join(format!("{name}.tar.gz"));
        let status = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&root)
            .arg(".")
            .status()
            .unwrap();
        assert!(status.success());
        archive
    }

    /// `fetch_archive` with a fake curl serving `archive`, into `<dir>/cache/src`
    /// pre-filled with an `old-pack`. Returns the result and that dest.
    #[cfg(unix)]
    fn fetch_over_old_cache(
        dir: &Path,
        archive: &Path,
    ) -> (anyhow::Result<()>, std::path::PathBuf) {
        let dest = dir.join("cache/src");
        write(&dest.join("old-pack/pack.yaml"), "name: old-pack\n");
        let curl = fake_curl(dir, &format!("cp '{}' \"$out\"", archive.display()));
        let tmp_root = dir.join("tmp");
        fs::create_dir_all(&tmp_root).unwrap();
        let res = retry_busy(|| fetch_archive(&curl, "https://e.com/a.tar.gz", &dest, &tmp_root));
        let cache: Vec<_> = fs::read_dir(dir.join("cache"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(cache, ["src"], "staging left in the cache root");
        assert_eq!(fs::read_dir(&tmp_root).unwrap().count(), 0);
        (res, dest)
    }

    #[cfg(unix)]
    #[test]
    fn fetch_archive_replaces_the_cache_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz(dir.path(), "ok", |root| {
            write(&root.join("demo/pack.yaml"), "name: demo\n");
            std::os::unix::fs::symlink("demo", root.join("inner")).unwrap();
        });
        let (res, dest) = fetch_over_old_cache(dir.path(), &archive);
        res.unwrap();
        assert!(dest.join("demo/pack.yaml").is_file());
        assert!(dest.join("inner/pack.yaml").is_file(), "inner link kept");
        assert!(!dest.join("old-pack").exists(), "old cache replaced");
    }

    #[cfg(unix)]
    #[test]
    fn fetch_archive_rejects_a_link_out_of_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        for (name, target) in [("abs", outside.clone()), ("rel", "../../..".into())] {
            let archive = tar_gz(dir.path(), name, |root| {
                write(&root.join("demo/pack.yaml"), "name: demo\n");
                // Nested, so the check must walk the whole tree.
                std::os::unix::fs::symlink(&target, root.join("demo/l")).unwrap();
            });
            let (res, dest) = fetch_over_old_cache(dir.path(), &archive);
            let err = res.unwrap_err().to_string();
            assert!(err.contains("'demo/l' points outside"), "{name}: {err}");
            assert!(err.contains("https://e.com/a.tar.gz"), "{name}: {err}");
            assert!(
                dest.join("old-pack/pack.yaml").is_file(),
                "{name}: old cache lost"
            );
            assert!(!dest.join("demo").exists(), "{name}: partial pack visible");
        }
    }

    #[cfg(unix)]
    #[test]
    fn fetch_archive_keeps_the_old_cache_when_extraction_stops_midway() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        // `demo/` extracts, then `l/pwned` would be written through the link `l`.
        let first = dir.path().join("first");
        write(&first.join("demo/pack.yaml"), "name: demo\n");
        std::os::unix::fs::symlink(&outside, first.join("l")).unwrap();
        let second = dir.path().join("second");
        write(&second.join("l/pwned"), "x");
        let archive = dir.path().join("evil.tar.gz");
        let status = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&first)
            .args(["demo", "l"])
            .arg("-C")
            .arg(&second)
            .arg("l/pwned")
            .status()
            .unwrap();
        assert!(status.success());
        let (res, dest) = fetch_over_old_cache(dir.path(), &archive);
        assert!(res.is_err());
        assert!(!outside.join("pwned").exists(), "wrote outside the archive");
        assert!(dest.join("old-pack/pack.yaml").is_file(), "old cache lost");
        assert!(!dest.join("demo").exists(), "partial pack visible");
    }

    #[cfg(unix)]
    #[test]
    fn fetch_archive_keeps_the_old_cache_on_a_truncated_archive() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz(dir.path(), "full", |root| {
            for i in 0..50 {
                write(&root.join(format!("demo/f{i}.md")), &"x".repeat(2000 + i));
            }
            write(&root.join("demo/pack.yaml"), "name: demo\n");
        });
        let bytes = fs::read(&archive).unwrap();
        let cut = dir.path().join("cut.tar.gz");
        fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let (res, dest) = fetch_over_old_cache(dir.path(), &cut);
        let err = res.unwrap_err().to_string();
        assert!(
            !err.contains("armadai-dl-"),
            "cites a deleted temp path: {err}"
        );
        assert!(dest.join("old-pack/pack.yaml").is_file(), "old cache lost");
        assert!(!dest.join("demo").exists(), "partial pack visible");
    }

    #[cfg(unix)]
    #[test]
    fn fetch_archive_leaves_nothing_in_the_temp_root() {
        let dir = tempfile::tempdir().unwrap();
        let tmp_root = dir.path().join("tmp");
        fs::create_dir(&tmp_root).unwrap();
        let curl = fake_curl(
            dir.path(),
            "ls -ld \"$(dirname \"$out\")\" > \"$(dirname \"$0\")/mode\"; printf 'not an archive' > \"$out\"",
        );
        let dest = dir.path().join("dest");
        let res = retry_busy(|| fetch_archive(&curl, "https://e.com/a.tar.gz", &dest, &tmp_root));
        assert!(res.is_err(), "garbage must not extract");
        let mode = fs::read_to_string(dir.path().join("mode")).unwrap();
        assert!(
            mode.starts_with("drwx------"),
            "download dir not private: {mode}"
        );
        let left: Vec<_> = fs::read_dir(&tmp_root).unwrap().collect();
        assert!(left.is_empty(), "left in temp root: {left:?}");
    }

    #[cfg(unix)]
    #[test]
    fn extract_archive_refuses_an_archive_too_big_once_extracted() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz(dir.path(), "big", |root| {
            write(&root.join("demo/big.md"), &"x".repeat(5000));
        });
        let mut archives = vec![archive];
        let zip_src = dir.path().join("big-src");
        let zip = dir.path().join("big.zip");
        let zipped = std::process::Command::new("zip")
            .current_dir(&zip_src)
            .arg("-qr")
            .arg(&zip)
            .arg(".")
            .status();
        if zipped.is_ok_and(|s| s.success()) {
            archives.push(zip);
        }
        for archive in archives {
            let dest = dir.path().join("dest");
            let limits = |bytes| ExtractLimits {
                bytes,
                entries: 10,
                ..EXTRACT_LIMITS
            };
            let err = extract_archive(&archive, &dest, "lbl", limits(4000))
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("exceed 4000 bytes"),
                "{}: {err}",
                archive.display()
            );
            assert!(!dest.join("demo").exists(), "extracted anyway");
            extract_archive(&archive, &dest, "lbl", limits(6000)).unwrap();
            assert!(dest.join("demo/big.md").is_file());
            fs::remove_dir_all(&dest).unwrap();
        }
    }

    #[test]
    fn swap_in_puts_the_old_copy_back_when_the_new_one_cannot_move() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("src");
        write(&dest.join("old-pack/pack.yaml"), "name: old-pack\n");
        let missing = dir.path().join("no-new-copy");
        let err = swap_in(&missing, &dest, &dir.path().join("old"))
            .unwrap_err()
            .to_string();
        assert!(err.contains(&dest.display().to_string()), "{err}");
        assert!(dest.join("old-pack/pack.yaml").is_file(), "old copy lost");
    }

    #[cfg(unix)]
    #[test]
    fn fetch_archive_sweeps_what_a_killed_sync_left() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        let tmp_root = dir.path().join("tmp");
        // Stale staging dirs (killed syncs): this source, another one whose cache is
        // still there, and one removed from the config whose cache is missing.
        let stale_stages = [
            cache.join(".armadai-staging-src+killed"),
            cache.join(".armadai-staging-src-2+killed"),
            cache.join(".armadai-staging-gone~ab12+killed"),
        ];
        for stage in &stale_stages {
            write(&stage.join("old/parked/pack.yaml"), "name: parked\n");
        }
        write(&cache.join("src-2/current/pack.yaml"), "name: current\n");
        let fresh_stage = cache.join(".armadai-staging-src+running");
        let stale_dl = tmp_root.join("armadai-dl-killed");
        let fresh_dl = tmp_root.join("armadai-dl-running");
        for d in [&fresh_stage, &stale_dl, &fresh_dl] {
            fs::create_dir_all(d).unwrap();
        }
        let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        for d in stale_stages.iter().chain([&stale_dl]) {
            fs::File::open(d)
                .unwrap()
                .set_modified(two_hours_ago)
                .unwrap();
        }
        let curl = fake_curl(dir.path(), "exit 6");
        let dest = cache.join("src");
        let res = retry_busy(|| fetch_archive(&curl, "https://e.com/a.tar.gz", &dest, &tmp_root));
        assert!(res.is_err());
        assert!(dest.join("parked/pack.yaml").is_file(), "src not restored");
        assert!(
            cache.join("gone~ab12/parked/pack.yaml").is_file(),
            "gone not restored"
        );
        assert!(
            cache.join("src-2/current/pack.yaml").is_file(),
            "src-2 overwritten"
        );
        assert!(!cache.join("src-2/parked").exists(), "src-2 overwritten");
        for stale in stale_stages.iter().chain([&stale_dl]) {
            assert!(!stale.exists(), "{} kept", stale.display());
        }
        assert!(
            fresh_stage.exists() && fresh_dl.exists(),
            "a running sync's dirs removed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fetch_archive_names_the_directory_it_cannot_write() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz(dir.path(), "ok", |root| {
            write(&root.join("demo/pack.yaml"), "name: demo\n");
        });
        let cache = dir.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o555)).unwrap();
        let curl = fake_curl(dir.path(), &format!("cp '{}' \"$out\"", archive.display()));
        let tmp_root = dir.path().join("tmp");
        fs::create_dir_all(&tmp_root).unwrap();
        let res = retry_busy(|| {
            fetch_archive(
                &curl,
                "https://e.com/a.tar.gz",
                &cache.join("src"),
                &tmp_root,
            )
        });
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o755)).unwrap();
        let err = res.unwrap_err().to_string();
        let ours = format!("cannot stage https://e.com/a.tar.gz in {}", cache.display());
        assert!(err.contains(&ours), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn fetch_archive_rejects_a_dangling_link() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz(dir.path(), "dangling", |root| {
            write(&root.join("demo/pack.yaml"), "name: demo\n");
            std::os::unix::fs::symlink("missing", root.join("demo/l")).unwrap();
        });
        let (res, dest) = fetch_over_old_cache(dir.path(), &archive);
        let err = res.unwrap_err().to_string();
        assert!(err.contains("'demo/l' is dangling"), "{err}");
        assert!(dest.join("old-pack/pack.yaml").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn extract_archive_refuses_too_many_entries() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz(dir.path(), "many", |root| {
            for i in 0..12 {
                write(&root.join(format!("f{i}")), "");
            }
        });
        let dest = dir.path().join("dest");
        let limits = |entries| ExtractLimits {
            bytes: 1 << 20,
            entries,
            ..EXTRACT_LIMITS
        };
        let err = extract_archive(&archive, &dest, "lbl", limits(10))
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "refusing archive lbl: more than the 10 entries allowed"
        );
        assert!(!dest.join("f0").exists(), "extracted anyway");
        extract_archive(&archive, &dest, "lbl", limits(20)).unwrap();
        assert!(dest.join("f0").exists());
    }

    /// `build`'s tree as a .tar.gz, and as a .zip when `zip` is installed.
    #[cfg(unix)]
    fn tar_gz_and_zip(dir: &Path, name: &str, build: impl Fn(&Path)) -> Vec<PathBuf> {
        let mut archives = vec![tar_gz(dir, name, build)];
        let zip = dir.join(format!("{name}.zip"));
        let zipped = std::process::Command::new("zip")
            .current_dir(dir.join(format!("{name}-src")))
            .arg("-qr")
            .arg(&zip)
            .arg(".")
            .status();
        if zipped.is_ok_and(|s| s.success()) {
            archives.push(zip);
        }
        archives
    }

    fn is_zip(archive: &Path) -> bool {
        archive.extension().is_some_and(|e| e == "zip")
    }

    /// Each bound accepts the archive at its exact value and refuses it one below,
    /// for tar.gz and zip alike.
    #[cfg(unix)]
    #[test]
    fn list_archive_entries_refuses_at_each_bound() {
        let dir = tempfile::tempdir().unwrap();
        let archives = tar_gz_and_zip(dir.path(), "names", |root| {
            write(&root.join("short"), "");
            write(&root.join("a-much-longer-entry-name.md"), "");
        });
        for archive in &archives {
            let list = |limits| list_archive_entries(archive, is_zip(archive), "lbl", limits);
            let names = list(EXTRACT_LIMITS).unwrap();
            assert!(names.iter().any(|n| n.ends_with("short")), "{names:?}");
            let longest = names.iter().map(String::len).max().unwrap();
            let total = names.iter().map(String::len).sum::<usize>();
            let exact = ExtractLimits {
                entries: names.len(),
                name_bytes: longest,
                names_bytes: total,
                ..EXTRACT_LIMITS
            };
            assert_eq!(list(exact).unwrap(), names, "{}", archive.display());
            for (limits, refusal) in [
                (
                    ExtractLimits {
                        entries: names.len() - 1,
                        ..exact
                    },
                    format!("more than the {} entries allowed", names.len() - 1),
                ),
                (
                    ExtractLimits {
                        name_bytes: longest - 1,
                        ..exact
                    },
                    format!("an entry name is longer than {} bytes", longest - 1),
                ),
                (
                    ExtractLimits {
                        names_bytes: total - 1,
                        ..exact
                    },
                    format!("its entry names exceed {} bytes in total", total - 1),
                ),
            ] {
                let err = list(limits).unwrap_err().to_string();
                assert_eq!(err, format!("refusing archive lbl: {refusal}"));
            }
        }
    }

    /// Past a bound the lister is killed: never waited on with its listing unread
    /// (it would block on the full pipe), never read to the end.
    #[cfg(unix)]
    #[test]
    fn list_archive_entries_kills_the_lister_past_a_bound() {
        let dir = tempfile::tempdir().unwrap();
        let archives = tar_gz_and_zip(dir.path(), "long", |root| {
            for i in 0..2000 {
                write(&root.join(format!("{i:04}-{}", "n".repeat(100))), "");
            }
        });
        for archive in archives {
            let (tx, rx) = std::sync::mpsc::channel();
            let zip = is_zip(&archive);
            std::thread::spawn(move || {
                let limits = ExtractLimits {
                    entries: 10,
                    ..EXTRACT_LIMITS
                };
                let _ = tx.send(list_archive_entries(&archive, zip, "lbl", limits));
            });
            let res = rx
                .recv_timeout(std::time::Duration::from_secs(60))
                .expect("lister not killed: still waiting on it");
            assert_eq!(
                res.unwrap_err().to_string(),
                "refusing archive lbl: more than the 10 entries allowed"
            );
        }
    }

    /// Run as `name` under a pseudo-terminal (`script`), so that a tool reading the
    /// terminal would block there; fails past 60 s instead of hanging. `false` when
    /// `script` is missing.
    #[cfg(unix)]
    fn run_under_a_terminal(name: &str) -> bool {
        let exe = std::env::current_exe().unwrap();
        let test = format!("starters_registry::archive_tests::{name}");
        let mut cmd = std::process::Command::new("script");
        if cfg!(target_os = "macos") {
            cmd.args(["-q", "/dev/null"])
                .arg(&exe)
                .args([&test, "--exact", "--nocapture"]);
        } else {
            let line = format!("'{}' '{test}' --exact --nocapture", exe.display());
            cmd.args(["-qec", &line, "/dev/null"]);
        }
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("log");
        let child = cmd
            .env("ARMADAI_TEST_UNDER_TERMINAL", "1")
            .stdin(std::process::Stdio::null())
            .stdout(fs::File::create(&out).unwrap())
            .stderr(std::process::Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            return false;
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{name} still running after 60 s: blocked on the terminal");
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        let log = fs::read_to_string(&out).unwrap_or_default();
        assert!(
            status.success() && log.contains("1 passed"),
            "{name} under a terminal: {status}\n{log}"
        );
        true
    }

    /// Under a terminal, an encrypted zip fails instead of asking for its password,
    /// and the tools do not get the terminal as stdin.
    #[cfg(unix)]
    #[test]
    fn extract_archive_refuses_an_encrypted_zip_without_prompting() {
        let name = "extract_archive_refuses_an_encrypted_zip_without_prompting";
        if std::env::var_os("ARMADAI_TEST_UNDER_TERMINAL").is_none() {
            if run_under_a_terminal(name) {
                return;
            }
            eprintln!("{name}: no `script`, run without a terminal");
        } else {
            let tty = |mut c: std::process::Command| {
                c.args(["-c", "test -t 0"]).status().unwrap().success()
            };
            assert!(
                tty(std::process::Command::new("sh")),
                "not under a terminal"
            );
            assert!(!tty(tool("sh")), "stdin of the tools is the terminal");
        }
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        write(&src.join("demo/pack.yaml"), "name: demo\n");
        let zip = dir.path().join("enc.zip");
        let zipped = std::process::Command::new("zip")
            .current_dir(&src)
            .args(["-qr", "-P", "secret"])
            .arg(&zip)
            .arg(".")
            .status();
        if !zipped.is_ok_and(|s| s.success()) {
            return; // no `zip` to build the fixture
        }
        let dest = dir.path().join("dest");
        assert!(extract_archive(&zip, &dest, "lbl", EXTRACT_LIMITS).is_err());
        assert!(!dest.join("demo/pack.yaml").exists());
    }
}
