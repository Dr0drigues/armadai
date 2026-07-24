use std::process::Command as ProcessCommand;

/// Self-update armadai by downloading the latest release binary.
pub async fn execute() -> anyhow::Result<()> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let platform = match (os, arch) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("macos", "aarch64") => "macos-aarch64",
        _ => anyhow::bail!("Unsupported platform: {os}/{arch}"),
    };

    let repo = "Dr0drigues/swarm-festai";
    let current_version = env!("CARGO_PKG_VERSION");
    let m = crate::cli::style::muted();
    anstream::println!("{m}Current version: v{current_version}{m:#}");
    let r = crate::cli::style::running();
    anstream::println!("{r}Checking for updates...{r:#}");

    // Get latest release tag
    let output = ProcessCommand::new("curl")
        .args([
            "-fsSL",
            &format!("https://api.github.com/repos/{repo}/releases/latest"),
        ])
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to check for updates: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let body = String::from_utf8_lossy(&output.stdout);
    let latest_version = body
        .lines()
        .find(|l| l.contains("\"tag_name\""))
        .and_then(|l| {
            l.split('"')
                .nth(3)
                .map(|v| v.strip_prefix('v').unwrap_or(v).to_string())
        })
        .ok_or_else(|| anyhow::anyhow!("Could not parse latest version from GitHub API"))?;

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

    // Download to temp file
    let tmp_path = install_dir.join(".armadai-update-tmp");
    let status = ProcessCommand::new("curl")
        .args([
            "-fsSL",
            "-o",
            &tmp_path.display().to_string(),
            &download_url,
        ])
        .status()?;

    if !status.success() {
        // Clean up
        if let Err(e) = std::fs::remove_file(&tmp_path) {
            tracing::debug!("Failed to remove temporary download file: {:?}", e);
        }
        anyhow::bail!(
            "Download failed. Check your network connection or try: VERSION=v{latest_version} install.sh"
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
