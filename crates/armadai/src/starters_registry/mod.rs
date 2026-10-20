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
use armadai_core::registries::{RegistrySource, SourceKind, cache_key, load_user_registries};
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
        anyhow::bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
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

/// List entry paths inside an archive without extracting it, so they can be
/// validated by [`archive_entry_is_safe`] before anything is written to disk.
fn list_archive_entries(archive: &Path, is_zip: bool) -> anyhow::Result<Vec<String>> {
    let out = if is_zip {
        std::process::Command::new("unzip")
            .args(["-Z1", archive.to_str().unwrap_or("")])
            .output()?
    } else {
        std::process::Command::new("tar")
            .args(["-tzf", archive.to_str().unwrap_or("")])
            .output()?
    };
    if !out.status.success() {
        anyhow::bail!(
            "failed to list archive entries for {}: {}",
            archive.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// Extract a downloaded archive (.tar.gz/.tgz or .zip) into `dest` by shelling
/// out to system `tar`/`unzip` (no extra crate). `dest` is created.
///
/// Before extracting, every entry is listed and validated with
/// [`archive_entry_is_safe`]: a malicious archive with a `../../etc/x` or
/// absolute-path entry (zip-slip) is rejected up front rather than allowed to
/// write outside `dest`.
fn extract_archive(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    let name = archive.to_string_lossy().to_lowercase();
    let is_zip = name.ends_with(".zip");

    let entries = list_archive_entries(archive, is_zip)?;
    if let Some(unsafe_entry) = entries.iter().find(|e| !archive_entry_is_safe(e)) {
        anyhow::bail!(
            "refusing to extract archive {}: unsafe entry path '{unsafe_entry}' (zip-slip guard)",
            archive.display()
        );
    }

    let status = if is_zip {
        std::process::Command::new("unzip")
            .args([
                "-q",
                "-o",
                archive.to_str().unwrap_or(""),
                "-d",
                dest.to_str().unwrap_or(""),
            ])
            .status()?
    } else {
        // .tar.gz / .tgz
        std::process::Command::new("tar")
            .args([
                "-xzf",
                archive.to_str().unwrap_or(""),
                "-C",
                dest.to_str().unwrap_or(""),
            ])
            .status()?
    };
    if !status.success() {
        anyhow::bail!("failed to extract archive {}", archive.display());
    }
    Ok(())
}

/// Archive-backed fetcher: download via `curl`, then extract via `tar`/`unzip`.
pub struct ArchiveFetcher;

impl StarterFetcher for ArchiveFetcher {
    fn fetch(&self, url: &str, dest: &Path) -> anyhow::Result<()> {
        fetch_archive("curl", url, dest, &std::env::temp_dir())
    }
}

/// Download into a private (0700), uniquely named dir under `tmp_root`, removed on return.
fn fetch_archive(curl: &str, url: &str, dest: &Path, tmp_root: &Path) -> anyhow::Result<()> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("armadai-dl-");
    #[cfg(unix)]
    builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
    let tmp = builder.tempdir_in(tmp_root)?;
    // The extension makes extract_archive pick tar vs unzip.
    let src = tmp.path().join(format!("archive.{}", archive_ext(url)));
    download_to(curl, url, &src)?;
    let _ = std::fs::remove_dir_all(dest); // fresh extract
    extract_archive(&src, dest)
}

fn archive_ext(url: &str) -> &'static str {
    let u = url.to_lowercase();
    if u.ends_with(".zip") {
        "zip"
    } else if u.ends_with(".tgz") {
        "tgz"
    } else {
        "tar.gz"
    }
}

/// `url` with any `user:pass@` userinfo masked, for messages.
fn redact(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    match authority.rfind('@') {
        Some(at) => format!("{scheme}://***@{}", &rest[at + 1..]),
        None => url.to_string(),
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
fn download_to(curl: &str, url: &str, out: &Path) -> anyhow::Result<()> {
    let shown = redact(url);
    if !url
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
    {
        anyhow::bail!("download {shown} refused: archive sources must use an https:// URL");
    }
    let result = std::process::Command::new(curl)
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

/// Sync all sources; warn+continue on failure. Returns the dirs that synced OK.
pub fn sync_starters(sources: &[RegistrySource]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for s in sources {
        match fetch_starter_source(s) {
            Ok(d) => dirs.push(d),
            Err(e) => eprintln!(
                "  warn: failed to sync starter source {}: {e}",
                redact(&s.url)
            ),
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
mod archive_tests {
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
        extract_archive(&archive, &dest).unwrap();
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
    fn retry_busy<T>(f: impl Fn() -> anyhow::Result<T>) -> anyhow::Result<T> {
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
    fn fake_curl(dir: &Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("curl");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$(dirname \"$0\")/args\"\n\
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
    fn redact_leaves_urls_without_userinfo_alone() {
        for url in [
            "https://example.com/a@b.tar.gz",
            "https://example.com/a.tar.gz?x=a@b",
            "git@github.com:org/repo.git",
        ] {
            assert_eq!(redact(url), url);
        }
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
}
