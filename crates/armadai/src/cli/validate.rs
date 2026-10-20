//! CLI command for validating starter packs and project configs.

use std::env;
use std::path::PathBuf;

use armadai_core::pack_validation::{Severity, validate_pack};

/// New format: `.armadai/project.yaml` present. Diagnostics come from `armadai-model`.
fn execute_new(target_path: &std::path::Path, json: bool) -> anyhow::Result<()> {
    report(armadai_model::validate(target_path), json)
}

/// Print `ds` (text or JSON); an error fails the command.
fn report(ds: Vec<armadai_model::Diagnostic>, json: bool) -> anyhow::Result<()> {
    use armadai_model::{Severity as S, has_errors};

    let errors = ds.iter().filter(|d| d.severity == S::Error).count();
    let warnings = ds.len() - errors;
    if json {
        let v: Vec<_> = ds
            .iter()
            .map(|d| {
                serde_json::json!({
                    "severity": d.severity.as_str(),
                    "code": d.code,
                    "file": d.file.display().to_string(),
                    "path": d.path,
                    "line": d.line,
                    "column": d.column,
                    "message": d.message,
                    "hint": d.hint,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        for d in &ds {
            println!("{}", d.render());
        }
        let style = if errors > 0 {
            crate::cli::style::err()
        } else if warnings > 0 {
            crate::cli::style::warn()
        } else {
            crate::cli::style::ok()
        };
        anstream::println!("{style}{errors} error(s), {warnings} warning(s){style:#}");
    }
    if has_errors(&ds) {
        anyhow::bail!("validation failed: {errors} error(s)");
    }
    Ok(())
}

pub fn execute(path: Option<PathBuf>, json: bool) -> anyhow::Result<()> {
    // Resolve path (default: current directory)
    let target_path = path.unwrap_or_else(|| env::current_dir().expect("Failed to get cwd"));

    if target_path.join(".armadai/project.yaml").is_file() {
        return execute_new(&target_path, json);
    }
    if let Ok(pack) = armadai_core::starter::StarterPack::load(&target_path)
        && pack.format.is_some()
    {
        if pack.format != Some(1) {
            anyhow::bail!("pack '{}' uses an unknown format", pack.name);
        }
        let pack = super::native_pack::files(&target_path)?;
        for f in &pack.ignored {
            eprintln!("warning: ignored: {} (not part of a pack)", f.display());
        }
        let (_tmp, ds) = super::native_pack::stage(&target_path, &pack.files)?;
        return report(ds, json);
    }
    if json {
        eprintln!("warning: --json only applies to new-format projects; ignored");
    }

    if !target_path.join("pack.yaml").is_file() {
        if (crate::compile::LEGACY_CONFIGS.iter()).any(|c| target_path.join(c).is_file()) {
            anyhow::bail!(
                "{} uses the legacy format, run `armadai migrate`",
                target_path.display()
            );
        }
        anyhow::bail!(
            "No .armadai/project.yaml or pack.yaml found at {}",
            target_path.display()
        );
    }
    let h = crate::cli::style::header();
    let a = crate::cli::style::accent();
    anstream::println!(
        "{h}Validating starter pack at:{h:#} {a}{}{a:#}",
        target_path.display()
    );
    println!();
    let issues = validate_pack(&target_path);

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
            // A multi-line message (R7's `link.coordinator` report) must
            // still read as one entry rather than as three unrelated
            // lines.
            crate::cli::style::indent_continuation(&issue.message, "  ")
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

    if error_count > 0 {
        anyhow::bail!("validation failed: {} error(s)", error_count);
    }

    println!();
    let o = crate::cli::style::ok();
    anstream::println!(
        "{o}Validation passed for pack at {}{o:#}",
        target_path.display()
    );

    Ok(())
}
