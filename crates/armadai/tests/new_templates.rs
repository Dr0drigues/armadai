//! `armadai new` must work on a fresh machine: the agent templates are embedded
//! in the binary, and a `templates/` directory (project or user) overrides them.

use assert_cmd::Command;
use std::path::{Path, PathBuf};

/// Every template shipped in `crates/armadai/templates/`, read from the directory.
fn embedded_templates() -> Vec<String> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/templates");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .collect();
    names.sort();
    names
}

struct Scratch {
    _root: tempfile::TempDir,
    config: PathBuf,
    cwd: PathBuf,
}

fn scratch() -> Scratch {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let cwd = root.path().join("work");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(cwd.join(".armadai")).unwrap();
    // `new` writes into the canonical project of the current directory
    std::fs::write(cwd.join(".armadai/project.yaml"), "schema_version: 1\n").unwrap();
    Scratch {
        _root: root,
        config,
        cwd,
    }
}

fn armadai(s: &Scratch, args: &[&str]) -> assert_cmd::assert::Assert {
    Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(&s.cwd)
        .env("ARMADAI_CONFIG_DIR", &s.config)
        .env("HOME", &s.cwd)
        .env("XDG_CONFIG_HOME", s.cwd.join("xdg"))
        .args(args)
        .assert()
}

fn agent_path(s: &Scratch, name: &str) -> PathBuf {
    s.cwd.join(".armadai/agents").join(format!("{name}.md"))
}

#[test]
fn every_embedded_template_creates_a_parseable_agent_without_any_templates_dir() {
    let s = scratch();
    assert!(!s.cwd.join("templates").exists());
    assert!(!s.config.join("templates").exists());

    let templates = embedded_templates();
    assert!(
        templates.len() >= 11,
        "expected the shipped templates, saw {templates:?}"
    );
    for tpl in &templates {
        let name = format!("agent-{tpl}");
        armadai(&s, &["new", "agent", &name, "--template", tpl]).success();
        let path = agent_path(&s, &name);
        assert!(path.exists(), "{tpl}: {} not created", path.display());
        let ds = armadai_model::validate(&s.cwd);
        assert!(
            ds.is_empty(),
            "{tpl}: diagnostics: {}",
            ds.iter().map(|d| d.render()).collect::<Vec<_>>().join("\n")
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("{{"),
            "{tpl}: a placeholder is left:\n{text}"
        );
    }
}

/// The templates are canonical: a frontmatter, no `## Pipeline` (flows replace it),
/// no CLI relay template.
#[test]
fn the_template_files_are_canonical() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/templates");
    for tpl in embedded_templates() {
        let text = std::fs::read_to_string(format!("{dir}/{tpl}.md")).unwrap();
        assert!(text.starts_with("---\ndescription: "), "{tpl}");
        assert!(!text.contains("## Pipeline"), "{tpl}");
        assert!(!text.contains("## Metadata"), "{tpl}");
    }
    assert_eq!(embedded_templates().len(), 11);
    assert!(!embedded_templates().contains(&"cli-generic".to_string()));
}

#[test]
fn default_template_works_with_no_flag() {
    let s = scratch();
    armadai(&s, &["new", "agent", "my-agent"]).success();
    assert!(agent_path(&s, "my-agent").exists());
}

#[test]
fn project_templates_dir_overrides_the_embedded_template() {
    let s = scratch();
    let tpl_dir = s.cwd.join("templates");
    std::fs::create_dir_all(&tpl_dir).unwrap();
    std::fs::write(
        tpl_dir.join("basic.md"),
        "---\ndescription: Mine\n---\nPROJECT_OVERRIDE_MARKER\n",
    )
    .unwrap();

    armadai(&s, &["new", "agent", "over", "--template", "basic"]).success();
    let body = std::fs::read_to_string(agent_path(&s, "over")).unwrap();
    assert!(body.contains("PROJECT_OVERRIDE_MARKER"), "{body}");
}

#[test]
fn user_templates_dir_overrides_the_embedded_template() {
    let s = scratch();
    let tpl_dir = s.config.join("templates");
    std::fs::create_dir_all(&tpl_dir).unwrap();
    std::fs::write(
        tpl_dir.join("basic.md"),
        "---\ndescription: Mine\n---\nUSER_OVERRIDE_MARKER\n",
    )
    .unwrap();

    armadai(&s, &["new", "agent", "over", "--template", "basic"]).success();
    let body = std::fs::read_to_string(agent_path(&s, "over")).unwrap();
    assert!(body.contains("USER_OVERRIDE_MARKER"), "{body}");
}

#[test]
fn a_custom_template_is_offered_next_to_the_embedded_ones() {
    let s = scratch();
    let tpl_dir = s.cwd.join("templates");
    std::fs::create_dir_all(&tpl_dir).unwrap();
    std::fs::write(
        tpl_dir.join("mine.md"),
        "---\ndescription: Mine\n---\nMINE\n",
    )
    .unwrap();

    armadai(&s, &["new", "agent", "a", "--template", "mine"]).success();
    let body = std::fs::read_to_string(agent_path(&s, "a")).unwrap();
    assert!(body.contains("MINE"), "{body}");
    armadai(&s, &["new", "agent", "b", "--template", "dev-review"]).success();
}

#[test]
fn a_legacy_format_template_is_a_clear_error() {
    let s = scratch();
    let tpl_dir = s.cwd.join("templates");
    std::fs::create_dir_all(&tpl_dir).unwrap();
    std::fs::write(
        tpl_dir.join("basic.md"),
        "# {{name}}\n\n## Metadata\n- provider: anthropic\n\n## System Prompt\n\nOld\n",
    )
    .unwrap();
    let out = armadai(&s, &["new", "agent", "old", "--template", "basic"]).failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        err.contains("template basic uses the legacy format"),
        "{err}"
    );
    assert!(!agent_path(&s, "old").exists());
}

#[test]
fn new_outside_a_project_is_refused() {
    let s = scratch();
    std::fs::remove_dir_all(s.cwd.join(".armadai")).unwrap();
    let out = armadai(&s, &["new", "agent", "lonely"]).failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(err.contains("armadai init --project"), "{err}");
}

#[test]
fn an_invalid_id_or_an_existing_agent_is_refused() {
    let s = scratch();
    armadai(&s, &["new", "agent", "Bad_Name"]).failure();
    armadai(&s, &["new", "agent", "dup"]).success();
    armadai(&s, &["new", "agent", "dup"]).failure();
}

#[test]
fn unknown_template_is_rejected_and_lists_the_available_ones() {
    let s = scratch();
    let out = armadai(&s, &["new", "agent", "x", "--template", "nope"]).failure();
    let o = out.get_output();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    // Rejected by clap's possible-values list, which now includes the embedded names.
    assert!(text.contains("basic") && text.contains("tdd-red"), "{text}");
    assert!(!Path::new(&agent_path(&s, "x")).exists());
}
