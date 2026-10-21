use crate::starters_registry::download_to;

/// The GitHub repository releases are downloaded from.
const REPO: &str = "Dr0drigues/armadai";

/// Self-update armadai by downloading the latest release binary.
pub fn execute() -> anyhow::Result<()> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let platform = match (os, arch) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("macos", "aarch64") => "macos-aarch64",
        _ => anyhow::bail!("Unsupported platform: {os}/{arch}"),
    };

    let repo = REPO;
    let current_version = env!("CARGO_PKG_VERSION");
    let m = crate::cli::style::muted();
    anstream::println!("{m}Current version: v{current_version}{m:#}");
    let r = crate::cli::style::running();
    anstream::println!("{r}Checking for updates...{r:#}");

    let latest_version = latest_version("curl", repo)?;

    if latest_version == current_version {
        let m = crate::cli::style::muted();
        anstream::println!("{m}Already up to date (v{current_version}).{m:#}");
        return Ok(());
    }

    let w = crate::cli::style::warn();
    let a = crate::cli::style::accent();
    anstream::println!("{w}New version available:{w:#} {a}v{latest_version}{a:#}");

    // Find where the current binary is installed
    let current_exe = std::env::current_exe()?;
    let install_dir = current_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Could not determine install directory"))?;

    let r = crate::cli::style::running();
    let a = crate::cli::style::accent();
    anstream::println!("{r}Downloading{r:#} {a}v{latest_version}{a:#} for {a}{platform}{a:#}...");

    let tmp_path = download_release("curl", repo, &latest_version, platform, install_dir)?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&tmp_path, perms)?;
    }

    // Replace current binary
    let target_path = install_dir.join("armadai");
    std::fs::rename(&tmp_path, &target_path)?;

    let o = crate::cli::style::ok();
    let a = crate::cli::style::accent();
    anstream::println!("{o}Updated to{o:#} {a}v{latest_version}{a:#}{o}!{o:#}");
    let m = crate::cli::style::muted();
    anstream::println!("{m}Run 'armadai --version' to verify.{m:#}");

    super::setup::prompt_shell_setup();

    Ok(())
}

/// The latest release tag of `repo`, without its `v`, fetched with the hardened
/// [`download_to`] (`.curlrc` ignored, HTTPS only, bounded).
fn latest_version(curl: &str, repo: &str) -> anyhow::Result<String> {
    let dir = tempfile::tempdir()?;
    let json = dir.path().join("latest.json");
    download_to(
        curl,
        &format!("https://api.github.com/repos/{repo}/releases/latest"),
        &json,
    )
    .map_err(|e| anyhow::anyhow!("Failed to check for updates: {e}"))?;
    parse_tag(&std::fs::read_to_string(&json)?)
}

/// Downloads the release binary for `platform` into `dir/.armadai-update-tmp` with the
/// hardened [`download_to`], checks it against the release's `SHA256SUMS`, and returns
/// that path. On any failure the file is removed: an unverified binary is never kept.
fn download_release(
    curl: &str,
    repo: &str,
    version: &str,
    platform: &str,
    dir: &std::path::Path,
) -> anyhow::Result<std::path::PathBuf> {
    let base = format!("https://github.com/{repo}/releases/download/v{version}");
    let artifact = format!("armadai-{platform}");
    let tmp_path = dir.join(".armadai-update-tmp");
    download_to(curl, &format!("{base}/{artifact}"), &tmp_path).map_err(|e| {
        anyhow::anyhow!("{e}\nCheck your network connection or try: VERSION=v{version} install.sh")
    })?;
    if let Err(e) = verify_checksum(curl, &format!("{base}/SHA256SUMS"), &artifact, &tmp_path) {
        let _ = std::fs::remove_file(&tmp_path);
        anyhow::bail!("v{version} not installed: {e}");
    }
    Ok(tmp_path)
}

/// Checks `file` against the `artifact` line of the `SHA256SUMS` at `sums_url`
/// (`sha256sum` format). A release without that file, or without that line, is refused.
fn verify_checksum(
    curl: &str,
    sums_url: &str,
    artifact: &str,
    file: &std::path::Path,
) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir()?;
    let sums = dir.path().join("SHA256SUMS");
    download_to(curl, sums_url, &sums).map_err(|e| {
        anyhow::anyhow!("the release publishes no SHA256SUMS ({e}); refusing an unverified binary")
    })?;
    let sums = std::fs::read_to_string(&sums)?;
    let expected = sums
        .lines()
        .filter_map(|l| l.split_once(char::is_whitespace))
        .find(|(_, name)| name.trim().trim_start_matches('*') == artifact)
        .map(|(hash, _)| hash.to_ascii_lowercase())
        .ok_or_else(|| anyhow::anyhow!("SHA256SUMS has no line for {artifact}"))?;
    let actual: String = Sha256::digest(std::fs::read(file)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    anyhow::ensure!(
        actual == expected,
        "checksum mismatch for {artifact}: expected {expected}, got {actual}"
    );
    Ok(())
}

fn parse_tag(body: &str) -> anyhow::Result<String> {
    body.lines()
        .find(|l| l.contains("\"tag_name\""))
        .and_then(|l| {
            l.split('"')
                .nth(3)
                .map(|v| v.strip_prefix('v').unwrap_or(v).to_string())
        })
        .ok_or_else(|| anyhow::anyhow!("Could not parse latest version from GitHub API"))
}

#[cfg(test)]
mod tests {
    /// `update` downloads from the repository the package declares.
    #[test]
    fn update_uses_the_declared_repository() {
        assert_eq!(
            format!("https://github.com/{}", super::REPO),
            env!("CARGO_PKG_REPOSITORY")
        );
    }

    /// The version check goes through the hardened download: `.curlrc` ignored (`-q`
    /// first), HTTPS only, no URL globbing (#488).
    #[cfg(unix)]
    #[test]
    fn the_version_check_uses_the_hardened_curl() {
        use crate::starters_registry::archive_tests::{fake_curl, retry_busy};
        let dir = tempfile::tempdir().unwrap();
        let curl = fake_curl(
            dir.path(),
            "printf '{\\n  \"tag_name\": \"v9.9.9\",\\n}' > \"$out\"",
        );
        let v = retry_busy(|| super::latest_version(&curl, super::REPO)).unwrap();
        assert_eq!(v, "9.9.9");
        let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
        let args: Vec<_> = args.lines().collect();
        assert_eq!(args.first(), Some(&"-q"));
        assert_eq!(
            args.last(),
            Some(&"https://api.github.com/repos/Dr0drigues/armadai/releases/latest")
        );
        for flag in ["--globoff", "--proto", "--proto-redir", "--max-time"] {
            assert!(args.contains(&flag), "{flag} missing: {args:?}");
        }
    }

    /// SHA-256 of the fake binary `printf binary` writes.
    #[cfg(unix)]
    fn binary_sha() -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(b"binary")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// A fake `curl` serving the binary, and `SHA256SUMS` with `sums` (a 404 when
    /// `None`); each call's args are kept in `args-<last URL segment>`.
    #[cfg(unix)]
    fn release_curl(dir: &std::path::Path, sums: Option<&str>) -> String {
        let sums = match sums {
            Some(s) => format!("printf '%s\\n' '{s}' > \"$out\""),
            None => "echo 'curl: (22) 404' >&2; exit 22".to_string(),
        };
        crate::starters_registry::archive_tests::fake_curl(
            dir,
            &format!(
                "cp \"$(dirname \"$0\")/args\" \"$(dirname \"$0\")/args-$(basename \"$1\")\"\n\
                 case \"$1\" in *SHA256SUMS) {sums} ;; *) printf binary > \"$out\" ;; esac"
            ),
        )
    }

    #[cfg(unix)]
    fn release(curl: &str, dir: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
        crate::starters_registry::archive_tests::retry_busy(|| {
            super::download_release(curl, super::REPO, "9.9.9", "macos-aarch64", dir)
        })
    }

    /// The executable itself, the most sensitive download of #488, goes through the
    /// hardened `curl` call too, and is kept only when it matches `SHA256SUMS`.
    #[cfg(unix)]
    #[test]
    fn the_binary_download_uses_the_hardened_curl() {
        let dir = tempfile::tempdir().unwrap();
        let line = format!("{}  armadai-macos-aarch64", binary_sha());
        let curl = release_curl(dir.path(), Some(&line));
        let tmp = release(&curl, dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "binary");
        let base = "https://github.com/Dr0drigues/armadai/releases/download/v9.9.9";
        for (file, url) in [
            (
                "armadai-macos-aarch64",
                format!("{base}/armadai-macos-aarch64"),
            ),
            ("SHA256SUMS", format!("{base}/SHA256SUMS")),
        ] {
            let args = std::fs::read_to_string(dir.path().join(format!("args-{file}"))).unwrap();
            let args: Vec<_> = args.lines().collect();
            assert_eq!(args.first(), Some(&"-q"), "{file}");
            assert_eq!(args.last(), Some(&url.as_str()), "{file}");
            for flag in ["--globoff", "--proto", "--proto-redir", "--max-filesize"] {
                assert!(args.contains(&flag), "{file}: {flag} missing: {args:?}");
            }
        }
    }

    /// A release without `SHA256SUMS`, without a line for the platform, or whose
    /// binary does not match, is refused and the downloaded file removed.
    #[cfg(unix)]
    #[test]
    fn an_unverified_binary_is_refused_and_removed() {
        let wrong = format!("{}  armadai-macos-aarch64", "0".repeat(64));
        let other = format!("{}  armadai-linux-x86_64", binary_sha());
        for (sums, expected) in [
            (None, "the release publishes no SHA256SUMS"),
            (
                Some(wrong.as_str()),
                "checksum mismatch for armadai-macos-aarch64",
            ),
            (
                Some(other.as_str()),
                "SHA256SUMS has no line for armadai-macos-aarch64",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let curl = release_curl(dir.path(), sums);
            let err = release(&curl, dir.path()).unwrap_err().to_string();
            assert!(err.starts_with("v9.9.9 not installed: "), "{err}");
            assert!(err.contains(expected), "{expected}: {err}");
            assert!(
                !dir.path().join(".armadai-update-tmp").exists(),
                "{expected}: the unverified binary was kept"
            );
        }
    }
}
