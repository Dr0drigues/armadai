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
    let l = load_project(dir.path());
    assert_eq!(find(&l, "missing-frontmatter").line, Some(1));
}

#[test]
fn unclosed_frontmatter() {
    let dir = project(&[M, (".armadai/agents/dev.md", "---\ndescription: x\nbody\n")]);
    let l = load_project(dir.path());
    assert_eq!(find(&l, "unclosed-frontmatter").line, Some(1));
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

fn errors_for(files: &[(&str, &str)], code: &str) -> Vec<Diagnostic> {
    let dir = project(files);
    load_project(dir.path())
        .diagnostics
        .into_iter()
        .filter(|d| d.code == code)
        .collect()
}

#[test]
fn duplicate_keys_in_free_maps_are_rejected() {
    for (file, text, line) in [
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: x }\n  a: { agent: y }\n",
            3,
        ),
        (
            ".armadai/project.yaml",
            "schema_version: 1\noutputs:\n  claude: a\n  claude: b\n",
            4,
        ),
        (
            ".armadai/project.yaml",
            "schema_version: 1\nnative:\n  claude: {}\n  claude: {}\n",
            4,
        ),
    ] {
        // the position is the second (offending) occurrence
        let ds = errors_for(&[M, (file, text)], "invalid-yaml");
        assert_eq!(ds.len(), 1, "{file}: {ds:?}");
        assert_eq!(ds[0].line, Some(line), "{file}");
    }
}

#[test]
fn duplicate_metadata_key_in_skill_is_rejected() {
    let ds = errors_for(
        &[
            M,
            (
                ".armadai/skills/s/SKILL.md",
                "---\nname: s\nmetadata:\n  k: a\n  k: b\n---\n",
            ),
        ],
        "invalid-yaml",
    );
    // the position is the second (offending) occurrence
    assert_eq!(ds[0].line, Some(5));
}

#[test]
fn duplicate_sequence_step_id_is_an_error_not_an_overwrite() {
    let ds = errors_for(
        &[
            M,
            (
                ".armadai/flows/f.yaml",
                "sequence:\n  - { id: x, agent: a }\n  - { id: x, agent: b }\n",
            ),
        ],
        "duplicate-step",
    );
    assert_eq!((ds[0].line, ds[0].column), (Some(2), Some(11)));
}

#[test]
fn explicit_sequence_form_keeps_its_id() {
    let dir = project(&[
        M,
        (
            ".armadai/flows/f.yaml",
            "sequence:\n  - { id: first, agent: a }\n  - b\n",
        ),
    ]);
    let p = load_project(dir.path()).project.unwrap();
    assert_eq!(p.flows["f"].steps["first"].agent, "a");
    assert_eq!(p.flows["f"].links[0].from, "first");
    assert_eq!(p.flows["f"].links[0].to, "b");
}

#[test]
fn unknown_key_in_explicit_sequence_item_is_rejected() {
    let ds = errors_for(
        &[
            M,
            (
                ".armadai/flows/f.yaml",
                "sequence:\n  - { id: x, agent: a, bogus: 1 }\n",
            ),
        ],
        "invalid-yaml",
    );
    assert_eq!(ds.len(), 1);
}

#[test]
fn serde_position_suffix_is_not_in_the_message() {
    let dir = project(&[
        M,
        (
            ".armadai/agents/dev.md",
            "---\ndescription: ok\ntier: huge\n---\n",
        ),
    ]);
    let l = load_project(dir.path());
    let d = find(&l, "invalid-yaml");
    assert!(!d.message.contains(" at line "), "{}", d.message);
    assert_eq!(d.column, Some(7));
}

#[test]
fn flows_and_automations_are_filtered_and_sorted() {
    for dir_name in ["flows", "automations"] {
        let f = |n: &str| format!(".armadai/{dir_name}/{n}");
        let (z, a, m, t, y) = (
            f("zed.yaml"),
            f("alpha.yaml"),
            f("mid.yaml"),
            f("notes.txt"),
            f("old.yml"),
        );
        let dir = project(&[
            M,
            (&z, ""),
            (&a, ""),
            (&m, ""),
            (&t, "ignored"),
            (&y, "x: 1"),
        ]);
        let l = load_project(dir.path());
        let files: Vec<_> = l
            .diagnostics
            .iter()
            .map(|d| {
                (
                    d.code,
                    d.file.file_name().unwrap().to_string_lossy().into_owned(),
                )
            })
            .collect();
        assert_eq!(
            files,
            [
                ("invalid-yaml", "alpha.yaml".to_string()),
                ("invalid-yaml", "mid.yaml".to_string()),
                ("ignored-file", "old.yml".to_string()),
                ("invalid-yaml", "zed.yaml".to_string()),
            ],
            "{dir_name}"
        );
    }
}

#[test]
fn automations_load() {
    let dir = project(&[
        M,
        (
            ".armadai/automations/check.yaml",
            "event: before-tool\nrun: ./c.sh\n",
        ),
    ]);
    let p = load_project(dir.path()).project.unwrap();
    assert_eq!(p.automations["check"].event, Event::BeforeTool);
    assert_eq!(p.automations["check"].run, "./c.sh");
}

#[test]
fn skill_folder_without_skill_md_warns() {
    let dir = project(&[M, (".armadai/skills/empty/README.md", "x")]);
    let l = load_project(dir.path());
    let d = find(&l, "ignored-file");
    assert_eq!(d.severity, Severity::Warning);
    assert!(d.file.ends_with("skills/empty"));
}

#[test]
fn empty_frontmatter_falls_back_to_its_line() {
    let ds = errors_for(
        &[M, (".armadai/agents/dev.md", "---\n---\nBody\n")],
        "invalid-yaml",
    );
    assert_eq!(ds[0].line, Some(2));
}

#[test]
fn schema_version_position_survives_spacing_and_flow_style() {
    for (text, line) in [
        ("name: x\nschema_version : 2\n", 2),
        ("{schema_version: 2}\n", 1),
    ] {
        let ds = errors_for(
            &[(".armadai/project.yaml", text)],
            "unsupported-schema-version",
        );
        assert_eq!(ds[0].line, Some(line), "{text}");
    }
    let ds = errors_for(
        &[(".armadai/project.yaml", "schema_version: 2\n")],
        "unsupported-schema-version",
    );
    assert_eq!((ds[0].line, ds[0].column), (Some(1), Some(1)));
}

#[test]
fn bom_is_tolerated_and_four_dashes_is_not_a_frontmatter() {
    let dir = project(&[
        (".armadai/project.yaml", "\u{feff}schema_version: 1\n"),
        (
            ".armadai/agents/dev.md",
            "\u{feff}---\ndescription: ok\n---\n",
        ),
    ]);
    let l = load_project(dir.path());
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    let ds = errors_for(
        &[
            M,
            (".armadai/agents/dev.md", "----\ndescription: x\n----\n"),
        ],
        "missing-frontmatter",
    );
    assert_eq!(ds.len(), 1);
}

#[test]
fn undiscoverable_schema_version_key_falls_back_to_line_one() {
    let ds = errors_for(
        &[(".armadai/project.yaml", "? schema_version\n: 2\n")],
        "unsupported-schema-version",
    );
    assert_eq!((ds[0].line, ds[0].column), (Some(1), Some(1)));
}

#[test]
fn bom_does_not_shift_columns_and_split_tolerates_it() {
    let ds = errors_for(
        &[(".armadai/project.yaml", "\u{feff}schema_version: 2\n")],
        "unsupported-schema-version",
    );
    assert_eq!((ds[0].line, ds[0].column), (Some(1), Some(1)));
    assert_eq!(
        split_frontmatter("\u{feff}---\na: 1\n---\nB\n"),
        Some(("a: 1\n", "B\n", 2))
    );
}

#[test]
fn sequence_exclusivity_is_about_key_presence() {
    for body in [
        "steps: {}\nsequence: [dev]\n",
        "sequence: []\nsteps:\n  a: { agent: dev }\n",
        "sequence: [dev]\nlinks: []\n",
    ] {
        let ds = errors_for(&[M, (".armadai/flows/f.yaml", body)], "sequence-and-steps");
        assert_eq!(ds.len(), 1, "{body}");
    }
}
