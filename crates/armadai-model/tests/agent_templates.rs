use std::collections::BTreeMap;

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
const CHECKER: (&str, &str) = (
    ".armadai/templates/checker.md",
    "---\ndescription: \"Checks {{subject}}\"\ntier: pro\ncapabilities: { write: deny, shell: deny }\n---\nYou check {{subject}}.\n",
);
const GUARDIAN: (&str, &str) = (
    ".armadai/agents/guardian.md",
    "---\nfrom: checker\nvars: { subject: the frame }\nskills: [cadre]\n---\nExtra line.\n",
);

fn load(files: &[(&str, &str)]) -> Loaded {
    load_project(project(files).path())
}

/// (severity, code, file name, line, message, hint) of every diagnostic.
type Short = (
    Severity,
    &'static str,
    String,
    Option<usize>,
    String,
    Option<String>,
);

fn short(l: &Loaded) -> Vec<Short> {
    l.diagnostics
        .iter()
        .map(|d| {
            (
                d.severity,
                d.code,
                d.file.file_name().unwrap().to_string_lossy().into_owned(),
                d.line,
                d.message.clone(),
                d.hint.clone(),
            )
        })
        .collect()
}

fn agent<'a>(l: &'a Loaded, id: &str) -> &'a Agent {
    &l.project.as_ref().unwrap().agents[id]
}

fn instance(vars: &str, extra: &str) -> (&'static str, String) {
    (
        ".armadai/agents/guardian.md",
        format!("---\nfrom: checker\nvars: {vars}\n{extra}---\n"),
    )
}

fn load_with(template: &str, inst: &(&'static str, String)) -> Loaded {
    load(&[
        M,
        (".armadai/templates/checker.md", template),
        (inst.0, &inst.1),
    ])
}

#[test]
fn instance_is_rendered_into_an_ordinary_agent() {
    let l = load(&[M, CHECKER, GUARDIAN]);
    assert_eq!(short(&l), vec![]);
    let a = agent(&l, "guardian");
    assert_eq!(a.front.description, "Checks the frame");
    assert_eq!(a.front.tier, Some(Tier::Pro));
    assert_eq!(a.front.skills, vec!["cadre".to_string()]);
    assert_eq!(a.body, "You check the frame.\n\nExtra line.\n");
    assert_eq!(a.front.from, None);
    assert_eq!(a.front.vars, BTreeMap::new());
    let i = a.instance.as_ref().unwrap();
    assert_eq!(
        (i.from.as_str(), i.body.as_str()),
        ("checker", "Extra line.\n")
    );
    assert_eq!(
        i.vars,
        BTreeMap::from([("subject".to_string(), "the frame".to_string())])
    );
    assert_eq!(
        serde_json::Value::Object(i.front.clone()),
        serde_json::json!({ "skills": ["cadre"] })
    );
    // the template is loaded, unrendered
    let t = &l.project.as_ref().unwrap().templates["checker"];
    assert_eq!(t.front.description.as_deref(), Some("Checks {{subject}}"));
    assert_eq!(t.body, "You check {{subject}}.\n");
}

#[test]
fn hand_written_agent_has_no_instance() {
    let l = load(&[
        M,
        (".armadai/agents/dev.md", "---\ndescription: d\n---\nB\n"),
    ]);
    assert_eq!(agent(&l, "dev").instance, None);
}

#[test]
fn instance_capabilities_replace_the_whole_template_key() {
    let l = load_with(
        CHECKER.1,
        &instance("{ subject: x }", "capabilities: { write: allow }\n"),
    );
    assert_eq!(short(&l), vec![]);
    assert_eq!(
        agent(&l, "guardian").front.capabilities,
        Capabilities {
            write: Some(Cap::Allow),
            ..Default::default()
        }
    );
}

#[test]
fn a_value_cannot_inject_yaml_structure() {
    let l = load_with(CHECKER.1, &instance("{ subject: \"a: b\\nc\" }", ""));
    assert_eq!(short(&l), vec![]);
    let a = agent(&l, "guardian");
    assert_eq!(a.front.description, "Checks a: b\nc");
    assert_eq!(a.body, "You check a: b\nc.\n");
}

#[test]
fn unknown_template_suggests_the_closest() {
    let l = load(&[
        M,
        CHECKER,
        (
            ".armadai/agents/guardian.md",
            "---\nvars: { zzz: q }\nfrom: chekcer\n---\n",
        ),
    ]);
    assert_eq!(
        short(&l),
        vec![(
            Severity::Error,
            "unknown-template",
            "guardian.md".into(),
            Some(3),
            "unknown template `chekcer`".into(),
            Some("did you mean `checker`?".into()),
        )]
    );
    assert!(!l.project.unwrap().agents.contains_key("guardian"));
}

#[test]
fn missing_template_var_is_an_error() {
    let l = load_with(
        "---\ndescription: \"{{subject}}\"\n---\n{{level}}\n",
        &instance("{ subject: x }", ""),
    );
    assert_eq!(
        short(&l),
        vec![(
            Severity::Error,
            "missing-template-var",
            "guardian.md".into(),
            Some(2),
            "template `checker` uses `{{level}}`, which `vars` does not set".into(),
            None,
        )]
    );
    assert!(!l.project.unwrap().agents.contains_key("guardian"));
}

#[test]
fn unused_template_var_warns_and_the_agent_loads() {
    let l = load_with(CHECKER.1, &instance("{ subject: x, extra: y }", ""));
    assert_eq!(
        short(&l),
        vec![(
            Severity::Warning,
            "unused-template-var",
            "guardian.md".into(),
            Some(3),
            "variable `extra` is not used by template `checker`".into(),
            None,
        )]
    );
    assert_eq!(agent(&l, "guardian").front.description, "Checks x");
}

#[test]
fn vars_without_template_is_an_error() {
    let l = load(&[
        M,
        (
            ".armadai/agents/dev.md",
            "---\ndescription: d\nvars: { a: b }\n---\n",
        ),
    ]);
    assert_eq!(
        short(&l),
        vec![(
            Severity::Error,
            "vars-without-template",
            "dev.md".into(),
            Some(3),
            "`vars` is only allowed with `from: <template>`".into(),
            None,
        )]
    );
}

#[test]
fn nested_template_is_an_error() {
    let l = load(&[
        M,
        CHECKER,
        (
            ".armadai/templates/strict.md",
            "---\ndescription: s\nfrom: checker\n---\n",
        ),
    ]);
    assert_eq!(
        short(&l),
        vec![(
            Severity::Error,
            "nested-template",
            "strict.md".into(),
            Some(3),
            "template `strict` cannot use `from`: a template does not inherit".into(),
            None,
        )]
    );
    assert!(!l.project.unwrap().templates.contains_key("strict"));
}

#[test]
fn bad_template_id_and_bad_variable_name() {
    let l = load(&[
        M,
        (".armadai/templates/Bad.md", "---\ndescription: d\n---\n"),
    ]);
    assert_eq!(
        short(&l),
        vec![(
            Severity::Error,
            "bad-id",
            "Bad.md".into(),
            None,
            "`Bad` is not a valid id".into(),
            Some("ids match [a-z0-9][a-z0-9-]{0,63}".into()),
        )]
    );
    let l = load_with(CHECKER.1, &instance("{ subject: x, Bad_Name: y }", ""));
    assert_eq!(
        short(&l),
        vec![(
            Severity::Error,
            "bad-id",
            "guardian.md".into(),
            Some(3),
            "variable `Bad_Name` is not a valid name".into(),
            Some("ids match [a-z0-9][a-z0-9-]{0,63}".into()),
        )]
    );
}

#[test]
fn description_left_to_nobody_is_an_error_on_the_instance() {
    let l = load_with("---\ntier: pro\n---\n", &instance("{}", ""));
    assert_eq!(
        short(&l),
        vec![(
            Severity::Error,
            "invalid-yaml",
            "guardian.md".into(),
            Some(2),
            "agent rendered from template `checker`: missing field `description`".into(),
            None,
        )]
    );
}

#[test]
fn other_files_and_folders_in_templates_are_ignored() {
    let l = load(&[
        M,
        CHECKER,
        (".armadai/templates/x.txt", "not a template"),
        (
            ".armadai/templates/sub/inner.md",
            "---\ndescription: d\n---\n",
        ),
    ]);
    assert_eq!(short(&l), vec![]);
    assert_eq!(
        l.project.unwrap().templates.keys().collect::<Vec<_>>(),
        vec!["checker"]
    );
}

#[test]
fn the_instance_own_values_and_body_are_rendered_too() {
    let inst = (
        ".armadai/agents/guardian.md",
        "---\nfrom: checker\nvars: { subject: x, extra: y, more: z }\ntitle: \"T {{extra}}\"\n---\nAlso {{more}}.\n".to_string(),
    );
    let l = load_with(CHECKER.1, &inst);
    assert_eq!(short(&l), vec![]);
    let a = agent(&l, "guardian");
    assert_eq!(a.front.title.as_deref(), Some("T y"));
    assert_eq!(a.body, "You check x.\n\nAlso z.\n");
}

#[test]
fn a_flow_style_instance_without_description_loads() {
    let l = load(&[
        M,
        CHECKER,
        (
            ".armadai/agents/guardian.md",
            "---\n{from: checker, vars: {subject: x}}\n---\n",
        ),
    ]);
    assert_eq!(short(&l), vec![]);
    assert_eq!(agent(&l, "guardian").front.description, "Checks x");
}

#[test]
fn a_number_survives_the_merge() {
    let l = load_with(
        "---\ndescription: d\ntemperature: 0.3\n---\n",
        &instance("{}", ""),
    );
    assert_eq!(short(&l), vec![]);
    assert_eq!(agent(&l, "guardian").front.temperature, Some(0.3));
}

#[test]
fn unknown_key_of_the_instance_is_reported_on_the_instance() {
    let l = load_with(CHECKER.1, &instance("{ subject: x }", "bogus: 1\n"));
    assert_eq!(
        short(&l),
        vec![(
            Severity::Error,
            "invalid-yaml",
            "guardian.md".into(),
            Some(4),
            "agent rendered from template `checker`: unknown field `bogus`, expected one of `description`, `title`, `tier`, `temperature`, `tags`, `skills`, `capabilities`, `native`".into(),
            None,
        )]
    );
}

#[test]
fn unknown_key_of_the_template_is_reported_on_the_template() {
    let l = load(&[
        M,
        (
            ".armadai/templates/checker.md",
            "---\ndescription: d\nbogus: 1\n---\n",
        ),
        GUARDIAN,
    ]);
    assert_eq!(
        short(&l),
        vec![(
            Severity::Error,
            "invalid-yaml",
            "checker.md".into(),
            Some(3),
            "unknown field `bogus`, expected one of description, title, tier, temperature, tags, skills, capabilities, native".into(),
            None,
        )]
    );
}

#[test]
fn an_instance_the_generic_read_refuses_is_an_error_not_a_hand_written_agent() {
    let l = load_with(
        CHECKER.1,
        &instance("{ subject: x }", "description: d\ntemperature: .inf\n"),
    );
    assert_eq!(
        short(&l),
        vec![(
            Severity::Error,
            "invalid-yaml",
            "guardian.md".into(),
            Some(5),
            "non-finite float `.inf` rejected by the configured policy".into(),
            None,
        )]
    );
    assert!(!l.project.unwrap().agents.contains_key("guardian"));
}

#[test]
fn a_replaced_key_needs_none_of_its_variables() {
    let template = "---\ndescription: \"Checks {{subject}}\"\n---\nBody.\n";
    let l = load_with(template, &instance("{}", "description: Own\n"));
    assert_eq!(short(&l), vec![]);
    assert_eq!(agent(&l, "guardian").front.description, "Own");
    // and a variable only the replaced key used is unused
    let l = load_with(template, &instance("{ subject: x }", "description: Own\n"));
    assert_eq!(
        short(&l),
        vec![(
            Severity::Warning,
            "unused-template-var",
            "guardian.md".into(),
            Some(3),
            "variable `subject` is not used by template `checker`".into(),
            None,
        )]
    );
}

#[test]
fn an_unknown_template_reports_nothing_else() {
    let l = load(&[
        M,
        (
            ".armadai/agents/guardian.md",
            "---\nfrom: nope\nvars: { Bad_Name: y }\n---\n",
        ),
    ]);
    assert_eq!(
        short(&l),
        vec![(
            Severity::Error,
            "unknown-template",
            "guardian.md".into(),
            Some(2),
            "unknown template `nope`".into(),
            None,
        )]
    );
}

#[test]
fn a_non_string_var_is_reported_on_vars() {
    let l = load_with(CHECKER.1, &instance("{ subject: 1 }", ""));
    assert_eq!(
        short(&l),
        vec![(
            Severity::Error,
            "invalid-yaml",
            "guardian.md".into(),
            Some(3),
            "invalid type: number, expected a string".into(),
            None,
        )]
    );
}

#[test]
fn missing_variables_are_reported_sorted() {
    let l = load_with(
        "---\ndescription: d\n---\n{{zeta}} {{alpha}}\n",
        &instance("{}", ""),
    );
    let missing = |v: &str| {
        (
            Severity::Error,
            "missing-template-var",
            "guardian.md".to_string(),
            Some(2),
            format!("template `checker` uses `{{{{{v}}}}}`, which `vars` does not set"),
            None,
        )
    };
    assert_eq!(short(&l), vec![missing("alpha"), missing("zeta")]);
}

/// (severity, code, file under `.armadai/`, line, column, message, hint) of every
/// diagnostic of `validate` (loader and semantic layer).
type Full = (
    Severity,
    &'static str,
    String,
    Option<usize>,
    Option<usize>,
    String,
    Option<String>,
);

fn validated(files: &[(&str, &str)]) -> Vec<Full> {
    let dir = project(files);
    let base = dir.path().join(".armadai");
    validate(dir.path())
        .into_iter()
        .map(|d| {
            (
                d.severity,
                d.code,
                d.file
                    .strip_prefix(&base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                d.line,
                d.column,
                d.message,
                d.hint,
            )
        })
        .collect()
}

#[test]
fn a_template_and_an_agent_cannot_share_an_id() {
    let ds = validated(&[
        M,
        (
            ".armadai/templates/guardian.md",
            "---\ndescription: d\n---\n",
        ),
        (".armadai/agents/guardian.md", "---\ndescription: d\n---\n"),
    ]);
    assert_eq!(
        ds,
        vec![(
            Severity::Error,
            "template-agent-collision",
            "templates/guardian.md".into(),
            None,
            None,
            "template `guardian` has the same id as agent `guardian`".into(),
            Some("rename the template or the agent".into()),
        )]
    );
}

#[test]
fn a_template_is_not_an_agent_of_a_flow_nor_the_coordinator() {
    let ds = validated(&[
        (
            ".armadai/project.yaml",
            "schema_version: 1\ncoordinator: checker\n",
        ),
        CHECKER,
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a:\n    agent: checker\n",
        ),
    ]);
    let unknown = |file: &str, line| {
        (
            Severity::Error,
            "unknown-agent",
            file.to_string(),
            Some(line),
            Some(if line == 2 { 14 } else { 12 }),
            "unknown agent `checker`".to_string(),
            None,
        )
    };
    assert_eq!(
        ds,
        vec![unknown("project.yaml", 2), unknown("flows/f.yaml", 3)]
    );
}

#[test]
fn a_semantic_error_from_the_template_points_at_the_template() {
    let ds = validated(&[
        M,
        (
            ".armadai/templates/checker.md",
            "---\ndescription: \"\"\nskills: [nope]\nnative:\n  bogus: {}\n---\n",
        ),
        (".armadai/agents/guardian.md", "---\nfrom: checker\n---\n"),
    ]);
    let at = |code, line, column, message: &str, hint: Option<&str>| {
        (
            Severity::Error,
            code,
            "templates/checker.md".to_string(),
            Some(line),
            Some(column),
            message.to_string(),
            hint.map(String::from),
        )
    };
    assert_eq!(
        ds,
        vec![
            at(
                "empty-description",
                2,
                1,
                "agent description is empty",
                None
            ),
            at("unknown-skill", 3, 10, "unknown skill `nope`", None),
            at(
                "unknown-target",
                5,
                3,
                "unknown target `bogus`",
                Some("supported: claude, codex, copilot, opencode"),
            ),
        ]
    );
}

#[test]
fn a_semantic_error_from_the_instance_points_at_the_instance() {
    let ds = validated(&[
        M,
        (
            ".armadai/templates/checker.md",
            "---\ndescription: d\nskills: [cadre]\nnative:\n  claude: {}\n---\n",
        ),
        (
            ".armadai/agents/guardian.md",
            "---\nfrom: checker\ndescription: \"\"\nskills: [nope]\nnative:\n  bogus: {}\n---\n",
        ),
    ]);
    let at = |code, line, column, message: &str, hint: Option<&str>| {
        (
            Severity::Error,
            code,
            "agents/guardian.md".to_string(),
            Some(line),
            Some(column),
            message.to_string(),
            hint.map(String::from),
        )
    };
    assert_eq!(
        ds,
        vec![
            at(
                "empty-description",
                3,
                1,
                "agent description is empty",
                None
            ),
            at("unknown-skill", 4, 10, "unknown skill `nope`", None),
            at(
                "unknown-target",
                6,
                3,
                "unknown target `bogus`",
                Some("supported: claude, codex, copilot, opencode"),
            ),
        ]
    );
}

#[test]
fn a_non_finite_temperature_from_the_template_points_at_the_template() {
    // unreachable from files (the loader refuses `.nan`): set on the loaded project
    let dir = project(&[
        M,
        (
            ".armadai/templates/checker.md",
            "---\ndescription: d\ntemperature: 0.2\n---\n",
        ),
        (".armadai/agents/guardian.md", "---\nfrom: checker\n---\n"),
    ]);
    let l = load_project(dir.path());
    let mut p = l.project.unwrap();
    p.agents.get_mut("guardian").unwrap().front.temperature = Some(f32::NAN);
    let ds: Vec<_> = check(&p, &l.origins)
        .into_iter()
        .map(|d| {
            (
                d.code,
                d.file.strip_prefix(dir.path()).unwrap().to_path_buf(),
                d.line,
                d.column,
                d.message,
            )
        })
        .collect();
    assert_eq!(
        ds,
        vec![(
            "invalid-temperature",
            ".armadai/templates/checker.md".into(),
            Some(3),
            Some(1),
            "temperature must be a finite number, got NaN".to_string(),
        )]
    );
}
