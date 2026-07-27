//! CLI command for validating starter packs and project configs.

use std::env;
use std::path::PathBuf;

use crate::core::pack_validation::{Severity, validate_pack, validate_project_config};

pub async fn execute(path: Option<PathBuf>) -> anyhow::Result<()> {
    // Resolve path (default: current directory)
    let target_path = path.unwrap_or_else(|| env::current_dir().expect("Failed to get cwd"));

    // Auto-detect mode
    let pack_yaml = target_path.join("pack.yaml");
    let armadai_yaml = target_path.join("armadai.yaml");
    let armadai_config_yaml = target_path.join(".armadai/config.yaml");
    let armadai_yml = target_path.join("armadai.yml");

    let (mode, issues) = if pack_yaml.is_file() {
        // Pack mode
        let h = crate::cli::style::header();
        let a = crate::cli::style::accent();
        anstream::println!(
            "{h}Validating starter pack at:{h:#} {a}{}{a:#}",
            target_path.display()
        );
        println!();
        ("pack", validate_pack(&target_path))
    } else if armadai_config_yaml.is_file() || armadai_yaml.is_file() || armadai_yml.is_file() {
        // Project mode
        let h = crate::cli::style::header();
        let a = crate::cli::style::accent();
        anstream::println!(
            "{h}Validating project config at:{h:#} {a}{}{a:#}",
            target_path.display()
        );
        println!();
        ("project", validate_project_config(&target_path))
    } else {
        // No recognized config file
        anyhow::bail!(
            "No pack.yaml or armadai.yaml found at {}",
            target_path.display()
        );
    };

    // Display issues
    let mut error_count = 0;
    let mut warning_count = 0;

    for issue in &issues {
        let (style, prefix) = match issue.severity {
            Severity::Error => {
                error_count += 1;
                (crate::cli::style::err(), "ERROR")
            }
            Severity::Warning => {
                warning_count += 1;
                (crate::cli::style::warn(), "WARN ")
            }
        };

        anstream::println!(
            "{style}{}{style:#} {}: {}",
            prefix,
            issue.location,
            issue.message
        );
    }

    // Footer
    println!();
    let footer_style = if error_count > 0 {
        crate::cli::style::err()
    } else if warning_count > 0 {
        crate::cli::style::warn()
    } else {
        crate::cli::style::ok()
    };
    anstream::println!(
        "{footer_style}{} error(s), {} warning(s){footer_style:#}",
        error_count,
        warning_count
    );

    // Exit code logic
    if error_count > 0 {
        anyhow::bail!("validation failed: {} error(s)", error_count);
    }

    println!();
    let o = crate::cli::style::ok();
    anstream::println!(
        "{o}Validation passed for {} at {}{o:#}",
        mode,
        target_path.display()
    );

    Ok(())
}
