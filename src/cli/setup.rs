use std::io::Write;
use std::path::{Path, PathBuf};

use clap::CommandFactory;

/// Prompt the user to set up shell integration (PATH symlink + completions).
///
/// Silently skips if stdin is not a terminal (non-interactive mode).
pub fn prompt_shell_setup() {
    if !is_interactive() {
        return;
    }

    let needs_path = !binary_in_path();
    let needs_completions = !completions_installed();

    if !needs_path && !needs_completions {
        return;
    }

    println!();
    let confirmed = dialoguer::Confirm::new()
        .with_prompt("Set up shell integration (PATH + completions)?")
        .default(true)
        .interact();

    match confirmed {
        Ok(true) => {
            if needs_path {
                setup_path();
            }
            if needs_completions {
                setup_completions();
            }
        }
        Ok(false) => {
            let m = crate::cli::style::muted();
            anstream::println!(
                "{m}Shell integration skipped. Run `armadai init` again to set it up later.{m:#}"
            );
        }
        Err(_) => {
            // Non-interactive or error — skip silently
        }
    }
}

/// Returns true if stdin is a terminal.
fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

/// Returns true if `armadai` resolves via PATH to the current executable.
fn binary_in_path() -> bool {
    let Ok(current_exe) = std::env::current_exe() else {
        return false;
    };
    // Canonicalize to resolve symlinks
    let Ok(current_canonical) = std::fs::canonicalize(&current_exe) else {
        return false;
    };

    // Walk PATH entries looking for an `armadai` binary that resolves to the same file
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("armadai");
            if candidate.exists()
                && let Ok(canonical) = std::fs::canonicalize(&candidate)
                && canonical == current_canonical
            {
                return true;
            }
        }
    }
    false
}

/// Returns true if shell completions are already installed for the detected shell.
fn completions_installed() -> bool {
    let Some(completion_path) = completion_file_path() else {
        return true; // Unknown shell — skip
    };
    completion_path.exists()
}

/// Returns the expected completion file path for the current shell, or None if unknown.
fn completion_file_path() -> Option<PathBuf> {
    let shell = detect_shell()?;
    let home = home_dir()?;
    let path = match shell.as_str() {
        "zsh" => home.join(".zfunc").join("_armadai"),
        "bash" => home
            .join(".local")
            .join("share")
            .join("bash-completion")
            .join("completions")
            .join("armadai"),
        "fish" => home
            .join(".config")
            .join("fish")
            .join("completions")
            .join("armadai.fish"),
        _ => return None,
    };
    Some(path)
}

/// Detect the current shell from the `$SHELL` environment variable.
fn detect_shell() -> Option<String> {
    let shell_path = std::env::var("SHELL").ok()?;
    let shell_name = std::path::Path::new(&shell_path)
        .file_name()?
        .to_str()?
        .to_string();
    Some(shell_name)
}

/// Return the user's home directory via `$HOME`.
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Create `~/.local/bin/armadai` symlink pointing to the current executable.
fn setup_path() {
    let Ok(current_exe) = std::env::current_exe() else {
        let e = crate::cli::style::err();
        anstream::eprintln!("{e}  [PATH] Could not determine current executable path.{e:#}");
        return;
    };
    let Some(home) = home_dir() else {
        let e = crate::cli::style::err();
        anstream::eprintln!("{e}  [PATH] Could not determine home directory.{e:#}");
        return;
    };

    let local_bin = home.join(".local").join("bin");
    if let Err(e) = std::fs::create_dir_all(&local_bin) {
        let err_style = crate::cli::style::err();
        anstream::eprintln!(
            "{err_style}  [PATH] Failed to create {}: {e}{err_style:#}",
            local_bin.display()
        );
        return;
    }

    let link_path = local_bin.join("armadai");

    // Remove existing symlink if present (but not a real file)
    if link_path.exists() || link_path.symlink_metadata().is_ok() {
        if link_path
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            if let Err(e) = std::fs::remove_file(&link_path) {
                tracing::debug!("Failed to remove existing symlink: {:?}", e);
            }
        } else {
            let w = crate::cli::style::warn();
            anstream::eprintln!(
                "{w}  [PATH] {} already exists and is not a symlink — skipping.{w:#}",
                link_path.display()
            );
            return;
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        match symlink(&current_exe, &link_path) {
            Ok(()) => {
                let o = crate::cli::style::ok();
                let m = crate::cli::style::muted();
                anstream::println!(
                    "{o}  [PATH] Symlink created:{o:#} {m}{} -> {}{m:#}",
                    link_path.display(),
                    current_exe.display()
                );
            }
            Err(e) => {
                let err_style = crate::cli::style::err();
                anstream::eprintln!(
                    "{err_style}  [PATH] Failed to create symlink: {e}{err_style:#}"
                );
                return;
            }
        }
    }

    #[cfg(not(unix))]
    {
        let w = crate::cli::style::warn();
        let m = crate::cli::style::muted();
        anstream::println!("{w}  [PATH] Symlink creation is not supported on this platform.{w:#}");
        anstream::println!("{m}         Add the following directory to your PATH manually:{m:#}");
        anstream::println!(
            "{m}         {}{m:#}",
            current_exe.parent().unwrap_or(&current_exe).display()
        );
        return;
    }

    // Check if ~/.local/bin is in PATH
    let local_bin_str = local_bin.to_string_lossy();
    let in_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.to_string_lossy() == local_bin_str))
        .unwrap_or(false);

    if !in_path {
        let rc_hint = shell_rc_file();
        let w = crate::cli::style::warn();
        let m = crate::cli::style::muted();
        anstream::println!("{w}  [PATH] ~/.local/bin is not in your PATH.{w:#}");
        anstream::println!("{m}         Add this line to {}:{m:#}", rc_hint);
        anstream::println!("{m}           export PATH=\"$HOME/.local/bin:$PATH\"{m:#}");
    }
}

/// Generate shell completions and write them to the appropriate file.
fn setup_completions() {
    let Some(shell_name) = detect_shell() else {
        let e = crate::cli::style::err();
        anstream::eprintln!("{e}  [Completions] Could not detect shell from $SHELL.{e:#}");
        return;
    };

    let clap_shell = match shell_name.as_str() {
        "zsh" => clap_complete::Shell::Zsh,
        "bash" => clap_complete::Shell::Bash,
        "fish" => clap_complete::Shell::Fish,
        other => {
            let w = crate::cli::style::warn();
            let m = crate::cli::style::muted();
            anstream::println!(
                "{w}  [Completions] Shell '{other}' is not supported for auto-install.{w:#}"
            );
            anstream::println!(
                "{m}               Run `armadai completion <shell>` to generate completions manually.{m:#}"
            );
            return;
        }
    };

    let Some(completion_path) = completion_file_path() else {
        return;
    };

    if let Some(parent) = completion_path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        let err_style = crate::cli::style::err();
        anstream::eprintln!(
            "{err_style}  [Completions] Failed to create directory {}: {e}{err_style:#}",
            parent.display()
        );
        return;
    }

    let mut buf = Vec::new();
    clap_complete::generate(clap_shell, &mut super::Cli::command(), "armadai", &mut buf);

    match std::fs::File::create(&completion_path).and_then(|mut f| f.write_all(&buf)) {
        Ok(()) => {
            let o = crate::cli::style::ok();
            let m = crate::cli::style::muted();
            anstream::println!(
                "{o}  [Completions] Written to:{o:#} {m}{}{m:#}",
                completion_path.display()
            );
            print_completion_hint(&shell_name, &completion_path);
        }
        Err(e) => {
            let err_style = crate::cli::style::err();
            anstream::eprintln!(
                "{err_style}  [Completions] Failed to write {}: {e}{err_style:#}",
                completion_path.display()
            );
        }
    }
}

/// Print shell-specific hints for activating completions.
fn print_completion_hint(shell: &str, path: &Path) {
    let h = crate::cli::style::header();
    let m = crate::cli::style::muted();
    let o = crate::cli::style::ok();
    match shell {
        "zsh" => {
            anstream::println!(
                "{h}  [Completions] To enable zsh completions, add to ~/.zshrc:{h:#}"
            );
            anstream::println!("{m}                  fpath=(~/.zfunc $fpath){m:#}");
            anstream::println!("{m}                  autoload -Uz compinit && compinit{m:#}");
        }
        "bash" => {
            anstream::println!(
                "{o}  [Completions] Completions will be auto-loaded on next bash session.{o:#}"
            );
            anstream::println!(
                "{m}               If not, source {} manually.{m:#}",
                path.display()
            );
        }
        "fish" => {
            anstream::println!(
                "{o}  [Completions] Fish completions are active immediately in new sessions.{o:#}"
            );
        }
        _ => {}
    }
}

/// Return a human-friendly RC file name for the detected shell.
fn shell_rc_file() -> String {
    match detect_shell().as_deref() {
        Some("zsh") => "~/.zshrc".to_string(),
        Some("bash") => "~/.bashrc (or ~/.bash_profile on macOS)".to_string(),
        Some("fish") => "~/.config/fish/config.fish".to_string(),
        _ => "your shell RC file".to_string(),
    }
}
