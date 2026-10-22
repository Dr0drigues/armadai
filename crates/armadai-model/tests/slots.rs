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

fn skill(id: &str, slot: Option<&str>) -> (String, String) {
    let meta = slot.map_or(String::new(), |s| {
        format!("metadata:\n  armadai-slot: \"{s}\"\n")
    });
    (
        format!(".armadai/skills/{id}/SKILL.md"),
        format!("---\nname: {id}\ndescription: d\n{meta}---\nB\n"),
    )
}

fn files(extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut fs: Vec<(String, String)> = vec![
        skill("b-std", Some("code-standards")),
        skill("a-std", Some("code-standards")),
        skill("x", Some("code-standards, security")),
        skill("z", None),
    ];
    fs.extend(extra.iter().map(|(p, c)| (p.to_string(), c.to_string())));
    fs.push((M.0.into(), M.1.into()));
    fs
}

fn loaded(extra: &[(&str, &str)]) -> (TempDir, Project) {
    let fs = files(extra);
    let refs: Vec<(&str, &str)> = fs.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let dir = project(&refs);
    let (p, _) = load_checked(dir.path()).unwrap();
    (dir, p)
}

#[test]
fn slot_fills_are_the_skills_declaring_the_slot_sorted() {
    let (_d, p) = loaded(&[]);
    assert_eq!(slot_fills(&p, "code-standards"), ["a-std", "b-std", "x"]);
    assert_eq!(slot_fills(&p, "security"), ["x"]);
    assert_eq!(slot_fills(&p, "nothing"), Vec::<String>::new());
}

#[test]
fn agent_skills_are_its_skills_then_its_slot_fills_without_duplicate() {
    let (_d, p) = loaded(&[(
        ".armadai/agents/dev.md",
        "---\ndescription: d\nskills: [z, a-std]\nslots: [code-standards, security]\n---\nB\n",
    )]);
    assert_eq!(agent_skills(&p, "dev"), ["z", "a-std", "b-std", "x"]);
}

#[test]
fn slot_fills_are_sorted_by_id_across_slots() {
    let (_d, p) = loaded(&[
        (
            ".armadai/skills/sec/SKILL.md",
            "---\nname: sec\ndescription: d\nmetadata:\n  armadai-slot: security\n---\nB\n",
        ),
        (
            ".armadai/agents/dev.md",
            "---\ndescription: d\nslots: [security, code-standards]\n---\nB\n",
        ),
    ]);
    assert_eq!(agent_skills(&p, "dev"), ["a-std", "b-std", "sec", "x"]);
}

#[test]
fn unused_slot_is_not_a_diagnostic() {
    let fs = files(&[]);
    let refs: Vec<(&str, &str)> = fs.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let dir = project(&refs);
    assert_eq!(validate(dir.path()), []);
}

#[test]
fn an_empty_slot_is_not_a_diagnostic() {
    let fs = files(&[(
        ".armadai/agents/dev.md",
        "---\ndescription: d\nslots: [infra-docs]\n---\nB\n",
    )]);
    let refs: Vec<(&str, &str)> = fs.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let dir = project(&refs);
    assert_eq!(validate(dir.path()), []);
    let (p, _) = load_checked(dir.path()).unwrap();
    assert_eq!(agent_skills(&p, "dev"), Vec::<String>::new());
}

#[test]
fn a_slot_of_an_agent_rendered_from_a_template_resolves() {
    let (_d, p) = loaded(&[
        (
            ".armadai/templates/checker.md",
            "---\ndescription: \"Checks {{subject}}\"\nslots: [security]\n---\nYou check.\n",
        ),
        (
            ".armadai/agents/guardian.md",
            "---\nfrom: checker\nvars: { subject: the frame }\nskills: [z]\n---\n",
        ),
    ]);
    assert_eq!(agent_skills(&p, "guardian"), ["z", "x"]);
}

#[test]
fn a_bad_slot_name_from_a_template_points_at_the_template() {
    let dir = project(&[
        M,
        (
            ".armadai/templates/checker.md",
            "---\ndescription: d\nslots: [Bad_Slot]\n---\n",
        ),
        (".armadai/agents/guardian.md", "---\nfrom: checker\n---\n"),
        (".armadai/agents/keeper.md", "---\nfrom: checker\n---\n"),
    ]);
    let got: Vec<_> = validate(dir.path())
        .into_iter()
        .map(|d| {
            let file = d.file.strip_prefix(dir.path()).unwrap().to_path_buf();
            (d.severity, d.code, file, d.line, d.column, d.message)
        })
        .collect();
    assert_eq!(
        got,
        [(
            Severity::Error,
            "bad-id",
            std::path::PathBuf::from(".armadai/templates/checker.md"),
            Some(3),
            Some(9),
            "slot name `Bad_Slot` is not a valid id".to_string()
        )]
    );
}

#[test]
fn an_instance_setting_its_own_slots_is_reported_on_the_instance() {
    let dir = project(&[
        M,
        (
            ".armadai/templates/checker.md",
            "---\ndescription: d\n---\n",
        ),
        (
            ".armadai/agents/guardian.md",
            "---\nfrom: checker\nslots: [Bad_Slot]\n---\n",
        ),
    ]);
    let got: Vec<_> = validate(dir.path())
        .into_iter()
        .map(|d| (d.code, d.file.file_name().unwrap().to_owned(), d.line))
        .collect();
    assert_eq!(got, [("bad-id", "guardian.md".into(), Some(3))]);
}
