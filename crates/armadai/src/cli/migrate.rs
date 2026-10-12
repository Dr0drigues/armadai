//! `armadai migrate`: convert a legacy project to the `.armadai/project.yaml` format.

use std::path::PathBuf;

pub fn execute(path: Option<PathBuf>, dry_run: bool, force: bool) -> anyhow::Result<()> {
    let root = match path {
        Some(p) => p,
        None => std::env::current_dir()?,
    };
    let report = crate::migrate::run(&root, dry_run, force)?;
    if dry_run {
        println!("Dry run: nothing was written.");
    }
    print!("{}", report.render());
    Ok(())
}
