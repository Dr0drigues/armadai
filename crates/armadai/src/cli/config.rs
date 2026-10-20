use clap::Subcommand;

use armadai_core::config as app_config;

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Manage custom starter pack directories
    #[command(name = "starters-dir")]
    StartersDir {
        #[command(subcommand)]
        action: StartersDirAction,
    },
}

#[derive(Subcommand)]
pub enum StartersDirAction {
    /// List all starter directories and their sources
    List,
    /// Add a custom starter directory to config.yaml
    Add {
        /// Path to the directory containing starter packs
        path: String,
    },
    /// Remove a custom starter directory from config.yaml
    Remove {
        /// Path to remove
        path: String,
    },
}

pub fn execute(action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::StartersDir { action } => match action {
            StartersDirAction::List => starters_dir_list(),
            StartersDirAction::Add { path } => starters_dir_add(&path),
            StartersDirAction::Remove { path } => starters_dir_remove(&path),
        },
    }
}

/// List all starter directories with their source type.
fn starters_dir_list() -> anyhow::Result<()> {
    use armadai_core::starter::{all_starters_dirs, builtin_starters_dir};

    let builtin = builtin_starters_dir();
    let user = app_config::user_starters_dir();

    let config = app_config::load_user_config();
    let config_dirs: Vec<std::path::PathBuf> = config
        .starters_dirs
        .iter()
        .map(std::path::PathBuf::from)
        .collect();

    let env_dirs: Vec<std::path::PathBuf> = std::env::var("ARMADAI_STARTERS_DIRS")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.trim().is_empty())
        .map(|s| std::path::PathBuf::from(s.trim()))
        .collect();

    let project_starters = std::path::Path::new(".armadai/starters");

    for dir in all_starters_dirs() {
        let source = if dir == builtin {
            "built-in"
        } else if dir == project_starters.to_path_buf() {
            "project"
        } else if dir == user {
            "user"
        } else if env_dirs.contains(&dir) {
            "env"
        } else if config_dirs.contains(&dir) {
            "config"
        } else {
            "unknown"
        };
        let m = crate::cli::style::muted();
        anstream::println!("{m}  [{source}]{m:#} {}", dir.display());
    }
    for (name, path) in armadai_core::starter::user_packs_masking_embedded() {
        let w = crate::cli::style::warn();
        anstream::println!(
            "{w}  note: {} is your own folder: it masks the embedded pack {name} \
             (ArmadAI never writes to it){w:#}",
            path.display()
        );
    }

    Ok(())
}

/// Add a custom starter directory to config.yaml.
fn starters_dir_add(path: &str) -> anyhow::Result<()> {
    let mut config = app_config::try_load_user_config()?;

    if config.starters_dirs.contains(&path.to_string()) {
        let m = crate::cli::style::muted();
        anstream::println!("{m}Directory already in config: {path}{m:#}");
        return Ok(());
    }

    config.starters_dirs.push(path.to_string());
    app_config::save_user_config(&config)?;
    let o = crate::cli::style::ok();
    anstream::println!("{o}Added starter directory: {path}{o:#}");
    let m = crate::cli::style::muted();
    anstream::println!(
        "{m}  Saved to {}{m:#}",
        app_config::config_file_path().display()
    );

    Ok(())
}

/// Remove a custom starter directory from config.yaml.
fn starters_dir_remove(path: &str) -> anyhow::Result<()> {
    let mut config = app_config::try_load_user_config()?;

    let before = config.starters_dirs.len();
    config.starters_dirs.retain(|d| d != path);

    if config.starters_dirs.len() == before {
        let m = crate::cli::style::muted();
        anstream::println!("{m}Directory not found in config: {path}{m:#}");
        return Ok(());
    }

    app_config::save_user_config(&config)?;
    let o = crate::cli::style::ok();
    anstream::println!("{o}Removed starter directory: {path}{o:#}");
    let m = crate::cli::style::muted();
    anstream::println!(
        "{m}  Saved to {}{m:#}",
        app_config::config_file_path().display()
    );

    Ok(())
}
