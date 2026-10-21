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

    let artifact = format!("armadai-{platform}");
    let download_url =
        format!("https://github.com/{repo}/releases/download/v{latest_version}/{artifact}");

    let r = crate::cli::style::running();
    let a = crate::cli::style::accent();
    anstream::println!("{r}Downloading{r:#} {a}v{latest_version}{a:#} for {a}{platform}{a:#}...");

    // Download to temp file (removed by `download_to` on failure)
    let tmp_path = install_dir.join(".armadai-update-tmp");
    if let Err(e) = download_to("curl", &download_url, &tmp_path) {
        anyhow::bail!(
            "{e}\nCheck your network connection or try: VERSION=v{latest_version} install.sh"
        );
    }

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
        for flag in ["--globoff", "--proto", "--proto-redir", "--max-time"] {
            assert!(args.contains(&flag), "{flag} missing: {args:?}");
        }
    }
}
