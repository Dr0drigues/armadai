use std::path::Path;

use armadai_model::*;
use tempfile::TempDir;

fn project(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let path = dir.path().join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, c).unwrap();
    }
    dir
}

const M: (&str, &str) = (".armadai/project.yaml", "schema_version: 1\n");

fn find<'a>(l: &'a Loaded, code: &str) -> &'a Diagnostic {
    l.diagnostics
        .iter()
        .find(|d| d.code == code)
        .unwrap_or_else(|| panic!("no `{code}` in {:?}", l.diagnostics))
}

#[test]
fn invalid_yaml_in_agent_frontmatter_points_at_the_real_line() {
    let dir = project(&[
        M,
        (
            ".armadai/agents/dev.md",
            "---\ndescription: ok\ntier: huge\n---\nBody\n",
        ),
    ]);
    let l = load_project(dir.path());
    let d = find(&l, "invalid-yaml");
    assert_eq!((d.line, d.severity), (Some(3), Severity::Error));
    assert!(d.file.ends_with(".armadai/agents/dev.md"));
}

#[test]
fn missing_manifest() {
    let dir = project(&[]);
    let l = load_project(dir.path());
    assert!(l.project.is_none());
    find(&l, "missing-manifest");
}

#[test]
fn unsupported_schema_version() {
    let dir = project(&[(".armadai/project.yaml", "schema_version: 2\n")]);
    let l = load_project(dir.path());
    assert_eq!(find(&l, "unsupported-schema-version").line, Some(1));
    assert!(l.project.is_none());
}

#[test]
fn missing_frontmatter() {
    let dir = project(&[M, (".armadai/agents/dev.md", "just a body\n")]);
    find(&load_project(dir.path()), "missing-frontmatter");
}

#[test]
fn unclosed_frontmatter() {
    let dir = project(&[M, (".armadai/agents/dev.md", "---\ndescription: x\nbody\n")]);
    find(&load_project(dir.path()), "unclosed-frontmatter");
}

#[test]
fn crlf_frontmatter_loads() {
    let dir = project(&[
        M,
        (
            ".armadai/agents/dev.md",
            "---\r\ndescription: crlf ok\r\n---\r\nBody\r\n",
        ),
    ]);
    let l = load_project(dir.path());
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    assert_eq!(
        l.project.unwrap().agents["dev"].front.description,
        "crlf ok"
    );
}

#[test]
fn sequence_expands_to_graph() {
    let dir = project(&[
        M,
        (".armadai/flows/f.yaml", "sequence: [a, b, c]\n"),
        (".armadai/flows/g.yaml", "sequence: [a, b, a]\n"),
    ]);
    let p = load_project(dir.path()).project.unwrap();
    assert_eq!(p.flows["f"].steps.len(), 3);
    assert_eq!(p.flows["f"].links.len(), 2);
    assert_eq!(p.flows["f"].links[0].from, "a");
    assert_eq!(p.flows["f"].links[0].to, "b");
    let ids: Vec<_> = p.flows["g"].steps.keys().cloned().collect();
    assert_eq!(ids, ["a", "a-2", "b"]);
    assert_eq!(p.flows["g"].links[1].to, "a-2");
}

#[test]
fn sequence_and_steps_conflict() {
    let dir = project(&[
        M,
        (
            ".armadai/flows/f.yaml",
            "sequence: [a]\nsteps:\n  s: { agent: a }\n",
        ),
    ]);
    let l = load_project(dir.path());
    assert_eq!(find(&l, "sequence-and-steps").line, Some(1));
    assert!(l.project.unwrap().flows.is_empty());
}

#[test]
fn unknown_skill_key_warns() {
    let dir = project(&[
        M,
        (
            ".armadai/skills/s/SKILL.md",
            "---\nname: s\ndescription: d\nfoo: 1\n---\nBody\n",
        ),
    ]);
    let l = load_project(dir.path());
    let d = find(&l, "unknown-skill-key");
    assert_eq!((d.severity, d.line), (Severity::Warning, Some(4)));
    assert!(l.project.unwrap().skills.contains_key("s"));
}

#[test]
fn empty_file_is_an_error_not_a_panic() {
    let dir = project(&[(".armadai/project.yaml", ""), (".armadai/flows/f.yaml", "")]);
    let l = load_project(dir.path());
    assert!(l.project.is_none());
    let files: Vec<_> = l
        .diagnostics
        .iter()
        .filter(|d| d.code == "invalid-yaml")
        .map(|d| d.file.file_name().unwrap().to_owned())
        .collect();
    assert_eq!(files.len(), 2, "{files:?}");
}

#[test]
fn discovery_is_sorted_and_ignores_other_extensions() {
    // Broken files: diagnostics are emitted in discovery order (maps would hide it).
    let dir = project(&[
        M,
        (".armadai/agents/zed.md", "no frontmatter"),
        (".armadai/agents/alpha.md", "no frontmatter"),
        (".armadai/agents/mid.md", "no frontmatter"),
        (".armadai/agents/notes.txt", "ignored"),
    ]);
    let l = load_project(dir.path());
    let files: Vec<_> = l
        .diagnostics
        .iter()
        .map(|d| d.file.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, ["alpha.md", "mid.md", "zed.md"]);
}

#[test]
fn diagnostic_renders_position_and_hint() {
    let d = Diagnostic {
        severity: Severity::Error,
        code: "x",
        file: Path::new("a.yaml").into(),
        path: String::new(),
        line: Some(3),
        column: Some(4),
        message: "m".into(),
        hint: Some("h".into()),
    };
    assert_eq!(d.render(), "a.yaml:3:4: error[x]: m\n  hint: h");
    let d = Diagnostic {
        line: None,
        column: None,
        hint: None,
        ..d
    };
    assert_eq!(d.render(), "a.yaml: error[x]: m");
}
