//! `armadai migrate`: a legacy project converted to the canonical `.armadai/` model.

pub mod convert;
pub mod leftover;
pub mod read;
pub mod report;
pub mod write;

use std::path::Path;

use anyhow::{Context, bail};

use convert::SkillCopy;
use read::OldProject;
use report::Report;

type Convert = fn(&OldProject) -> (armadai_model::Project, Vec<SkillCopy>, Report);

/// Migrate the legacy project at `root`. With `dry_run`, nothing under `root` changes.
pub fn run(root: &Path, dry_run: bool, force: bool) -> anyhow::Result<Report> {
    run_with(root, dry_run, force, convert::convert, |_| {})
}

fn rendered_errors(dir: &Path) -> Option<String> {
    let ds = armadai_model::validate(dir);
    armadai_model::has_errors(&ds)
        .then(|| ds.iter().map(|d| d.render()).collect::<Vec<_>>().join("\n"))
}

/// `after_write` runs once the new files are written (a test seam; `run` passes a no-op).
fn run_with(
    root: &Path,
    dry_run: bool,
    force: bool,
    conv: Convert,
    after_write: fn(&Path),
) -> anyhow::Result<Report> {
    if root.join(".armadai/project.yaml").exists() && !force {
        bail!(
            "{} is already migrated (use --force to redo it)",
            root.display()
        );
    }
    let old = read::read_old(root)?;
    let root = old.root.clone();
    let (project, copies, mut report) = conv(&old);

    // Written and validated in a scratch directory first: a conversion that does not
    // validate fails here, before the project is touched.
    let scratch = tempfile::tempdir()?;
    report.written = write::write_all(&project, &copies, scratch.path(), &root, &root)?;
    if let Some(errors) = rendered_errors(scratch.path()) {
        bail!("the converted project does not validate; nothing was changed:\n{errors}");
    }
    let plan = write::plan(&old, &copies);
    report.warnings.extend(plan.warnings.iter().cloned());
    if dry_run {
        return Ok(report);
    }

    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = write::backup(&root, &plan, secs)?;
    report.backup = Some(backup.clone());
    let interrupted = || {
        format!(
            "migration interrupted; the previous project is in {}",
            backup.display()
        )
    };
    write::remove_legacy(&root, &plan).with_context(interrupted)?;
    report.written = write::write_all(&project, &copies, &root, &root, &root.join(&backup))
        .with_context(interrupted)?;
    after_write(&root);
    if let Some(errors) = rendered_errors(&root) {
        bail!(
            "the migrated project does not validate; the previous project is in {}:\n{errors}",
            backup.display()
        );
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const CASES: [&str; 6] = [
        "armadai-dev",
        "code-analysis-rust",
        "declarative",
        "demo-rust-team",
        "fullstack",
        "rust-dev",
    ];

    fn fixture(case: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/migrate")
            .join(case)
    }

    fn converted(case: &str) -> (armadai_model::Project, report::Report) {
        let _iso = armadai_core::test_support::IsolatedConfigDir::enter();
        let old = read::read_old(&fixture(case)).unwrap();
        let (p, _, r) = convert::convert(&old);
        (p, r)
    }

    fn tree(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let bytes = std::fs::read(&p).unwrap();
                    files.push((p, bytes));
                }
            }
        }
        files.sort();
        files
    }

    /// A conversion whose output does not validate: a step naming no agent.
    fn invalid(old: &OldProject) -> (armadai_model::Project, Vec<SkillCopy>, Report) {
        let (mut p, c, r) = convert::convert(old);
        let flow: armadai_model::FlowFile =
            serde_yaml_ng::from_str("steps:\n  a: { agent: ghost }\n").unwrap();
        p.flows.insert(
            "bad".into(),
            armadai_model::Flow {
                description: None,
                steps: flow.steps,
                links: Vec::new(),
            },
        );
        (p, c, r)
    }

    #[test]
    fn an_invalid_conversion_fails_before_touching_the_project() {
        let _iso = armadai_core::test_support::IsolatedConfigDir::enter();
        let dir = tempfile::tempdir().unwrap();
        let src = fixture("demo-rust-team").join(".armadai");
        std::fs::create_dir_all(dir.path().join(".armadai/agents")).unwrap();
        for f in [
            "config.yaml",
            "agents/rust-reviewer.md",
            "agents/doc-writer.md",
        ] {
            std::fs::copy(src.join(f), dir.path().join(".armadai").join(f)).unwrap();
        }
        let before = tree(dir.path());
        let e = run_with(dir.path(), false, false, invalid, |_| {})
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("unknown-agent") && e.contains("nothing was changed"),
            "{e}"
        );
        assert_eq!(tree(dir.path()), before);
    }

    #[test]
    fn an_output_that_does_not_validate_fails_and_names_the_backup() {
        let _iso = armadai_core::test_support::IsolatedConfigDir::enter();
        let dir = tempfile::tempdir().unwrap();
        let src = fixture("demo-rust-team").join(".armadai");
        std::fs::create_dir_all(dir.path().join(".armadai/agents")).unwrap();
        for f in ["config.yaml", "agents/rust-reviewer.md"] {
            std::fs::copy(src.join(f), dir.path().join(".armadai").join(f)).unwrap();
        }
        let corrupt = |root: &Path| {
            std::fs::create_dir_all(root.join(".armadai/flows")).unwrap();
            std::fs::write(
                root.join(".armadai/flows/bad.yaml"),
                "steps:\n  a: { agent: ghost }\n",
            )
            .unwrap();
        };
        let e = run_with(dir.path(), false, false, convert::convert, corrupt)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("does not validate")
                && e.contains("the previous project is in .armadai/.migrated/"),
            "{e}"
        );
        let migrated = dir.path().join(".armadai/.migrated");
        let backup = std::fs::read_dir(&migrated)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(backup.join(".armadai/config.yaml").is_file());
    }

    #[test]
    fn every_fixture_converts_to_a_valid_project() {
        for case in CASES {
            let (p, r) = converted(case);
            assert!(!p.agents.is_empty(), "{case}");
            assert!(
                !r.warnings.iter().any(|w| w.contains("not migrated")),
                "{case}: {:?}",
                r.warnings
            );
            for (id, a) in &p.agents {
                assert!(!a.front.description.is_empty(), "{case}/{id}");
                assert!(!a.body.trim().is_empty(), "{case}/{id}");
            }
            let dir = tempfile::tempdir().unwrap();
            armadai_model::write_project(&p, dir.path()).unwrap();
            let ds = armadai_model::validate(dir.path());
            assert!(
                !armadai_model::has_errors(&ds),
                "{case}: {}",
                ds.iter().map(|d| d.render()).collect::<Vec<_>>().join("\n")
            );
        }
    }

    #[test]
    fn fullstack_orchestration_omits_the_unknown_agent() {
        let (p, r) = converted("fullstack");
        let f = &p.flows["orchestration"];
        assert!(!f.steps.contains_key("ghost-agent"));
        assert!(f.steps.contains_key("doc-generator"));
        assert!(r.warnings.iter().any(|w| w.contains("`ghost-agent`")));
        assert_eq!(p.manifest.coordinator.as_deref(), Some("code-reviewer"));
        assert_eq!(p.manifest.policy.free_agents, ["doc-generator"]);
    }

    #[test]
    fn declarative_agents_keep_their_composed_prompt_and_description() {
        let (p, _) = converted("declarative");
        let core = &p.agents["core-specialist"];
        assert_eq!(core.front.description, "Owns the domain layer");
        assert_eq!(
            core.body,
            "You are core-specialist, a specialist of this Rust workspace.\n\n\
             You only change files of the `core` module."
        );
        assert_eq!(core.front.tier, Some(armadai_model::Tier::Pro));
        assert_eq!(core.front.tags, ["team"]);
        assert_eq!(p.agents["cli-specialist"].front.temperature, Some(0.3));
        assert!(p.skills.contains_key("review-checklist"));
    }
}
