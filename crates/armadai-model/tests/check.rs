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
const DEV: (&str, &str) = (
    ".armadai/agents/dev.md",
    "---\ndescription: dev\n---\nBody\n",
);
const REV: (&str, &str) = (
    ".armadai/agents/rev.md",
    "---\ndescription: rev\n---\nBody\n",
);

fn diags(files: &[(&str, &str)]) -> Vec<Diagnostic> {
    let dir = project(files);
    validate(dir.path())
}

/// The single diagnostic with `code`; asserts severity and position.
fn expect(ds: &[Diagnostic], code: &str, sev: Severity, pos: (usize, usize)) -> Diagnostic {
    let hits: Vec<_> = ds.iter().filter(|d| d.code == code).collect();
    assert_eq!(hits.len(), 1, "`{code}` in {ds:?}");
    let d = hits[0];
    assert_eq!(d.severity, sev, "{code}");
    assert_eq!(
        (d.line, d.column),
        (Some(pos.0), Some(pos.1)),
        "{code}: {d:?}"
    );
    d.clone()
}

fn err(ds: &[Diagnostic], code: &str, pos: (usize, usize)) -> Diagnostic {
    expect(ds, code, Severity::Error, pos)
}

fn only(ds: &[Diagnostic], code: &str) -> Diagnostic {
    let hits: Vec<_> = ds.iter().filter(|d| d.code == code).collect();
    assert_eq!(hits.len(), 1, "`{code}` in {ds:?}");
    hits[0].clone()
}

#[test]
fn targets_list_is_the_supported_one() {
    assert_eq!(TARGETS, ["claude", "codex", "copilot", "opencode"]);
}

#[test]
fn unknown_agent_in_a_step_is_positioned_on_its_token() {
    let ds = diags(&[
        M,
        (".armadai/flows/f.yaml", "steps:\n  a: { agent: ghost }\n"),
    ]);
    let d = err(&ds, "unknown-agent", (2, 15));
    assert!(d.message.contains("ghost"));
}

#[test]
fn unknown_coordinator_is_an_error_unknown_free_agent_a_warning() {
    let ds = diags(&[(
        ".armadai/project.yaml",
        "schema_version: 1\ncoordinator: boss\npolicy:\n  free_agents: [Explore]\n",
    )]);
    let d = err(&ds, "unknown-agent", (2, 14));
    assert!(d.message.contains("boss"));
    let w = expect(&ds, "unknown-free-agent", Severity::Warning, (4, 17));
    assert!(
        w.hint
            .as_deref()
            .unwrap()
            .contains("Explore, Plan, general-purpose"),
        "{w:?}"
    );
    assert!(
        !ds.iter()
            .any(|d| d.code == "unknown-agent" && d.message.contains("Explore"))
    );
}

#[test]
fn a_built_in_free_agent_does_not_refuse_the_project() {
    let dir = project(&[
        (
            ".armadai/project.yaml",
            "schema_version: 1\npolicy:\n  free_agents: [Explore, general-purpose]\n",
        ),
        DEV,
    ]);
    let (p, _) = load_checked(dir.path()).unwrap();
    assert_eq!(
        p.manifest.policy.free_agents,
        ["Explore", "general-purpose"]
    );
}

#[test]
fn a_free_agent_close_to_a_project_agent_gets_a_did_you_mean() {
    let ds = diags(&[
        (
            ".armadai/project.yaml",
            "schema_version: 1\npolicy:\n  free_agents: [dve]\n",
        ),
        DEV,
    ]);
    assert_eq!(
        only(&ds, "unknown-free-agent").hint.as_deref(),
        Some("did you mean `dev`?")
    );
}

#[test]
fn near_miss_gets_a_did_you_mean_hint() {
    let ds = diags(&[
        M,
        DEV,
        (".armadai/flows/f.yaml", "steps:\n  a: { agent: dve }\n"),
    ]);
    assert_eq!(
        only(&ds, "unknown-agent").hint.as_deref(),
        Some("did you mean `dev`?")
    );
}

#[test]
fn bad_id_is_an_error() {
    let ds = diags(&[
        M,
        (".armadai/agents/Dev_X.md", "---\ndescription: d\n---\n"),
    ]);
    let d = only(&ds, "bad-id");
    assert_eq!(d.severity, Severity::Error);
    assert!(d.file.ends_with("Dev_X.md"));
}

#[test]
fn unknown_target_in_manifest_outputs_and_native() {
    let ds = diags(&[
        (
            ".armadai/project.yaml",
            "schema_version: 1\ntargets: [claude, gemini]\noutputs:\n  cursor: x\nnative:\n  zed: {}\n",
        ),
        (
            ".armadai/agents/dev.md",
            "---\ndescription: d\nnative:\n  vim: {}\n---\n",
        ),
        (
            ".armadai/automations/a.yaml",
            "event: on-stop\nrun: x\nnative:\n  emacs: {}\n",
        ),
    ]);
    let got: Vec<_> = ds
        .iter()
        .filter(|d| d.code == "unknown-target")
        .map(|d| (d.line.unwrap(), d.column.unwrap(), d.message.clone()))
        .collect();
    assert_eq!(got.len(), 5, "{ds:?}");
    for (line, col, token) in [(2, 19, "gemini"), (4, 3, "cursor"), (6, 3, "zed")] {
        assert!(
            got.iter()
                .any(|g| (g.0, g.1) == (line, col) && g.2.contains(token)),
            "{token}: {got:?}"
        );
    }
    assert!(
        got.iter()
            .any(|g| (g.0, g.1) == (4, 3) && g.2.contains("vim"))
    );
    assert!(
        got.iter()
            .any(|g| (g.0, g.1) == (4, 3) && g.2.contains("emacs"))
    );
}

#[test]
fn unknown_skill_in_an_agent() {
    let ds = diags(&[
        M,
        (
            ".armadai/agents/dev.md",
            "---\ndescription: d\nskills: [nope]\n---\n",
        ),
    ]);
    err(&ds, "unknown-skill", (3, 10));
}

#[test]
fn unknown_step_in_links_and_on_failure() {
    let ds = diags(&[
        M,
        DEV,
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: dev, on_failure: zzz }\nlinks:\n  - { from: a, to: yyy }\n",
        ),
    ]);
    let hits: Vec<_> = ds.iter().filter(|d| d.code == "unknown-step").collect();
    assert_eq!(hits.len(), 2, "{ds:?}");
    assert_eq!((hits[0].line, hits[0].column), (Some(2), Some(32)));
    assert_eq!((hits[1].line, hits[1].column), (Some(4), Some(20)));
}

#[test]
fn unknown_state_outside_done_failed_and_declared() {
    let ds = diags(&[
        M,
        DEV,
        REV,
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: dev, states: [approved] }\n  b: { agent: rev }\nlinks:\n  - { from: a, to: b, when: bogus }\n",
        ),
    ]);
    err(&ds, "unknown-state", (5, 29));
}

#[test]
fn declared_state_and_failed_are_accepted() {
    let ds = diags(&[
        M,
        DEV,
        REV,
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: dev, states: [approved] }\n  b: { agent: rev }\n  c: { agent: rev }\nlinks:\n  - { from: a, to: b, when: approved }\n  - { from: a, to: c, when: failed }\n",
        ),
    ]);
    assert!(!ds.iter().any(|d| d.code == "unknown-state"), "{ds:?}");
}

#[test]
fn duplicate_failure_path() {
    let ds = diags(&[
        M,
        DEV,
        REV,
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: dev, on_failure: b }\n  b: { agent: rev }\nlinks:\n  - { from: a, to: b, when: failed }\n",
        ),
    ]);
    err(&ds, "duplicate-failure-path", (2, 3));
}

#[test]
fn self_link() {
    let ds = diags(&[
        M,
        DEV,
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: dev }\n  b: { agent: dev }\nlinks:\n  - { from: a, to: b }\n  - { from: b, to: b }\n",
        ),
    ]);
    err(&ds, "self-link", (3, 3)); // shortcut: first occurrence of the step id
}

#[test]
fn empty_flow() {
    let ds = diags(&[M, (".armadai/flows/f.yaml", "description: nothing\n")]);
    let d = only(&ds, "empty-flow");
    assert_eq!(d.severity, Severity::Error);
}

#[test]
fn empty_command() {
    let ds = diags(&[
        M,
        (
            ".armadai/automations/a.yaml",
            "event: on-stop\nrun: \"  \"\n",
        ),
    ]);
    err(&ds, "empty-command", (2, 1));
}

#[test]
fn skill_name_mismatch() {
    let ds = diags(&[
        M,
        (
            ".armadai/skills/s/SKILL.md",
            "---\nname: other\ndescription: d\n---\n",
        ),
    ]);
    err(&ds, "skill-name-mismatch", (2, 7));
}

#[test]
fn skill_missing_description() {
    let ds = diags(&[M, (".armadai/skills/s/SKILL.md", "---\nname: s\n---\n")]);
    assert_eq!(
        only(&ds, "skill-missing-description").severity,
        Severity::Error
    );
    let ds = diags(&[
        M,
        (
            ".armadai/skills/s/SKILL.md",
            "---\nname: s\ndescription: \"\"\n---\n",
        ),
    ]);
    err(&ds, "skill-missing-description", (3, 1));
}

#[test]
fn skill_description_too_long() {
    let long = "x".repeat(1025);
    let ok = "x".repeat(1024);
    let ds = diags(&[
        M,
        (
            ".armadai/skills/s/SKILL.md",
            &format!("---\nname: s\ndescription: {long}\n---\n"),
        ),
    ]);
    err(&ds, "skill-description-too-long", (3, 1));
    let ds = diags(&[
        M,
        (
            ".armadai/skills/s/SKILL.md",
            &format!("---\nname: s\ndescription: {ok}\n---\n"),
        ),
    ]);
    assert!(ds.is_empty(), "{ds:?}");
}

#[test]
fn empty_agent_description() {
    let ds = diags(&[
        M,
        (".armadai/agents/dev.md", "---\ndescription: \"\"\n---\n"),
    ]);
    err(&ds, "empty-description", (2, 1));
}

#[test]
fn non_finite_temperature() {
    for v in [".nan", ".inf"] {
        let body = format!("---\ndescription: d\ntemperature: {v}\n---\n");
        let ds = diags(&[M, (".armadai/agents/dev.md", &body)]);
        err(&ds, "invalid-temperature", (3, 1));
    }
    let ds = diags(&[
        M,
        (
            ".armadai/agents/dev.md",
            "---\ndescription: d\ntemperature: 0.4\n---\n",
        ),
    ]);
    assert!(ds.is_empty(), "{ds:?}");
}

#[test]
fn duplicate_step_state() {
    let ds = diags(&[
        M,
        DEV,
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: dev, states: [done] }\n",
        ),
    ]);
    err(&ds, "duplicate-step-state", (2, 29));
    let ds = diags(&[
        M,
        DEV,
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: dev, states: [x, x] }\n",
        ),
    ]);
    only(&ds, "duplicate-step-state");
}

fn flow(body: &str) -> Vec<Diagnostic> {
    diags(&[M, DEV, (".armadai/flows/f.yaml", body)])
}

#[test]
fn unreachable_step_is_a_warning() {
    let ds = flow(
        "steps:\n  a: { agent: dev }\n  b: { agent: dev }\n  c: { agent: dev }\n  d: { agent: dev }\nlinks:\n  - { from: a, to: b }\n  - { from: c, to: d }\n  - { from: d, to: c }\n",
    );
    let got: Vec<_> = ds
        .iter()
        .filter(|d| d.code == "unreachable-step")
        .map(|d| (d.severity, d.line, d.column))
        .collect();
    assert_eq!(
        got,
        [
            (Severity::Warning, Some(4), Some(3)),
            (Severity::Warning, Some(5), Some(3))
        ]
    );
}

#[test]
fn pure_loop_has_no_entry_step() {
    let ds = flow(
        "steps:\n  a: { agent: dev }\n  b: { agent: dev }\nlinks:\n  - { from: a, to: b }\n  - { from: b, to: a }\n",
    );
    assert_eq!(only(&ds, "no-entry-step").severity, Severity::Warning);
    assert!(!ds.iter().any(|d| d.code == "unreachable-step"));
}

/// An entry step has no incoming *unconditional* link (`when` absent or `done`):
/// a failure path back to it does not make it a non-entry.
#[test]
fn a_failure_path_back_keeps_the_entry_step() {
    let ds = flow(
        "steps:\n  a: { agent: dev }\n  b: { agent: dev, on_failure: a }\nlinks:\n  - { from: a, to: b }\n",
    );
    assert!(ds.is_empty(), "{ds:?}");
    let ds = flow(
        "steps:\n  a: { agent: dev }\n  b: { agent: dev }\nlinks:\n  - { from: a, to: b }\n  - { from: b, to: a, when: failed }\n",
    );
    assert!(ds.is_empty(), "{ds:?}");
    // ...and a step reached only through on_failure is reachable.
    let ds = flow("steps:\n  a: { agent: dev, on_failure: c }\n  c: { agent: dev }\n");
    assert!(ds.is_empty(), "{ds:?}");
}

/// The review-loop shape: the rejected path back to the author is conditional.
#[test]
fn a_review_loop_has_its_author_as_entry() {
    let body = "steps:\n  write: { agent: dev }\n  review: { agent: dev, states: [approved, rejected] }\n  ship: { agent: dev }\nlinks:\n  - { from: write, to: review }\n  - { from: review, to: write, when: rejected }\n  - { from: review, to: ship, when: approved }\n";
    assert!(flow(body).is_empty(), "{:?}", flow(body));
    let f = load_project(project(&[M, DEV, (".armadai/flows/f.yaml", body)]).path())
        .project
        .unwrap()
        .flows
        .remove("f")
        .unwrap();
    assert_eq!(entry_steps(&f), ["write"]);
}

/// Two steps linked to each other only conditionally: either may start the flow.
#[test]
fn mutually_conditional_steps_are_both_entries() {
    let body = "steps:\n  a: { agent: dev, states: [x] }\n  b: { agent: dev, states: [y] }\nlinks:\n  - { from: a, to: b, when: x }\n  - { from: b, to: a, when: y }\n";
    assert!(flow(body).is_empty(), "{:?}", flow(body));
    let f = load_project(project(&[M, DEV, (".armadai/flows/f.yaml", body)]).path())
        .project
        .unwrap()
        .flows
        .remove("f")
        .unwrap();
    assert_eq!(entry_steps(&f), ["a", "b"]);
}

/// A loop of unconditional links, even with `done` spelled out, never starts.
#[test]
fn a_done_loop_has_no_entry_step() {
    let ds = flow(
        "steps:\n  a: { agent: dev }\n  b: { agent: dev }\nlinks:\n  - { from: a, to: b, when: done }\n  - { from: b, to: a }\n",
    );
    only(&ds, "no-entry-step");
}

#[test]
fn loop_is_allowed() {
    let ds = flow(
        "steps:\n  a: { agent: dev }\n  r: { agent: dev, states: [ok, ko] }\n  c: { agent: dev }\nlinks:\n  - { from: a, to: r }\n  - { from: r, to: c, when: ko }\n  - { from: c, to: r }\n",
    );
    assert!(ds.is_empty(), "{ds:?}");
}

#[test]
fn locate_requires_whole_word() {
    assert_eq!(
        locate("a: ghostly\nb: ghost\n", "ghost"),
        (Some(2), Some(4))
    );
    assert_eq!(locate("x", "y"), (None, None));
    assert_eq!(locate("é: ghost", "ghost"), (Some(1), Some(4)));
}

#[test]
fn valid_project_has_no_diagnostic() {
    let ds = diags(&[
        (
            ".armadai/project.yaml",
            "schema_version: 1\nname: demo\ntargets: [claude, opencode]\noutputs: { claude: .claude }\ncoordinator: lead\npolicy:\n  delegation: strict\n  free_agents: [explorer]\nnative: { claude: {} }\n",
        ),
        (
            ".armadai/agents/lead.md",
            "---\ndescription: coordinates\ntier: max\n---\nLead\n",
        ),
        (
            ".armadai/agents/explorer.md",
            "---\ndescription: explores\ntier: fast\n---\nX\n",
        ),
        (
            ".armadai/agents/developpeur.md",
            "---\ndescription: Writes code\ntitle: Dev\ntier: pro\ntemperature: 0.4\ntags: [dev]\nskills: [conventions-rust]\ncapabilities: { read: allow, write: ask, shell: ask, web: deny, delegate: allow }\nnative: { claude: { model: claude-sonnet-4-5 } }\n---\nBody\n",
        ),
        (
            ".armadai/agents/relecteur.md",
            "---\ndescription: reviews\n---\nR\n",
        ),
        (
            ".armadai/agents/livreur.md",
            "---\ndescription: ships\n---\nL\n",
        ),
        (
            ".armadai/skills/conventions-rust/SKILL.md",
            "---\nname: conventions-rust\ndescription: Rust conventions\nlicense: MIT\n---\nBody\n",
        ),
        (
            ".armadai/flows/delivery.yaml",
            "description: Code to delivery\nsteps:\n  premier-jet: { agent: developpeur }\n  revue: { agent: relecteur, states: [approved, rejected], on_failure: corrections }\n  corrections: { agent: developpeur }\n  livraison: { agent: livreur }\nlinks:\n  - { from: premier-jet, to: revue }\n  - { from: revue, to: corrections, when: rejected }\n  - { from: corrections, to: revue }\n  - { from: revue, to: livraison, when: approved }\n",
        ),
        (
            ".armadai/automations/check.yaml",
            "event: before-tool\nrun: ./scripts/check.sh\ndescription: guard\nnative: {}\n",
        ),
    ]);
    assert!(ds.is_empty(), "{ds:?}");
}

#[test]
fn id_length_limit_is_64() {
    let ok = "a".repeat(64);
    let bad = "a".repeat(65);
    let body = "---\ndescription: d\n---\n";
    let ds = diags(&[M, (&format!(".armadai/agents/{ok}.md"), body)]);
    assert!(ds.is_empty(), "{ds:?}");
    let ds = diags(&[M, (&format!(".armadai/agents/{bad}.md"), body)]);
    only(&ds, "bad-id");
}

#[test]
fn on_failure_to_its_own_step_is_a_self_link() {
    let ds = flow("steps:\n  a: { agent: dev, on_failure: a }\n");
    err(&ds, "self-link", (2, 20));
}

#[test]
fn did_you_mean_picks_the_closest_within_a_proportional_bound() {
    let agent = |n: &str| {
        (
            format!(".armadai/agents/{n}.md"),
            "---\ndescription: d\n---\n".to_string(),
        )
    };
    let (a1, b1) = agent("reviewer");
    let (a2, b2) = agent("reviewers");
    let ds = diags(&[
        M,
        (&a1, &b1),
        (&a2, &b2),
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: reviewerz }\n",
        ),
    ]);
    // `reviewers` is at distance 1, `reviewer` at 1 too: tie -> alphabetical
    assert_eq!(
        only(&ds, "unknown-agent").hint.as_deref(),
        Some("did you mean `reviewer`?")
    );
    let ds = diags(&[
        M,
        (&a1, &b1),
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: reviewxx }\n",
        ),
    ]);
    // distance 2 > max(1, 8/3) = 2? equal: accepted
    assert_eq!(
        only(&ds, "unknown-agent").hint.as_deref(),
        Some("did you mean `reviewer`?")
    );
    let ds = diags(&[
        M,
        (&a1, &b1),
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: rexxxxxx }\n",
        ),
    ]);
    assert!(
        only(&ds, "unknown-agent")
            .hint
            .unwrap()
            .starts_with("known:")
    );
    // short token: bound is 1, so distance 2 is refused
    let (a3, b3) = agent("dev");
    let ds = diags(&[
        M,
        (&a3, &b3),
        (".armadai/flows/f.yaml", "steps:\n  a: { agent: xyv }\n"),
    ]);
    assert!(
        only(&ds, "unknown-agent")
            .hint
            .unwrap()
            .starts_with("known:")
    );
}

#[test]
fn a_diagnostic_without_origin_is_kept_without_position() {
    let loaded = load_project(
        project(&[
            M,
            DEV,
            (".armadai/flows/f.yaml", "steps:\n  a: { agent: ghost }\n"),
        ])
        .path(),
    );
    let ds = check(&loaded.project.unwrap(), &Origins::default());
    let d = only(&ds, "unknown-agent");
    assert_eq!((d.line, d.column), (None, None));
}

#[test]
fn identical_links_are_a_duplicate() {
    let ds = flow(
        "steps:\n  a: { agent: dev }\n  b: { agent: dev }\nlinks:\n  - { from: a, to: b }\n  - { from: a, to: b, when: done }\n",
    );
    only(&ds, "duplicate-link");
    let ds = flow(
        "steps:\n  a: { agent: dev }\n  b: { agent: dev }\nlinks:\n  - { from: a, to: b }\n  - { from: a, to: b, when: failed }\n",
    );
    assert!(!ds.iter().any(|d| d.code == "duplicate-link"), "{ds:?}");
}

#[test]
fn step_ids_follow_the_id_pattern() {
    let ds = flow("steps:\n  Bad_Step: { agent: dev }\n");
    let d = err(&ds, "bad-id", (2, 3));
    assert!(d.message.contains("Bad_Step"));
}

#[test]
fn did_you_mean_prefers_the_nearest_over_the_first() {
    let body = "---\ndescription: d\n---\n";
    let ds = diags(&[
        M,
        (".armadai/agents/pipe-a.md", body),
        (".armadai/agents/pipelinx.md", body),
        (
            ".armadai/flows/f.yaml",
            "steps:\n  a: { agent: pipeline }\n",
        ),
    ]);
    assert_eq!(
        only(&ds, "unknown-agent").hint.as_deref(),
        Some("did you mean `pipelinx`?")
    );
}

#[test]
fn load_checked_refuses_a_project_with_an_error() {
    // `ghost` is not a declared agent: a semantic (check) error, not a loader one
    let dir = project(&[
        (
            ".armadai/project.yaml",
            "schema_version: 1\ncoordinator: ghost\n",
        ),
        DEV,
    ]);
    let ds = load_checked(dir.path()).unwrap_err();
    assert!(has_errors(&ds), "{ds:?}");
}

#[test]
fn load_checked_refuses_a_missing_manifest() {
    let dir = project(&[DEV]);
    let ds = load_checked(dir.path()).unwrap_err();
    assert!(ds.iter().any(|d| d.code == "missing-manifest"), "{ds:?}");
}

#[test]
fn load_checked_accepts_a_project_with_warnings_only() {
    let dir = project(&[M, DEV, (".armadai/flows/x.yml", "sequence: [dev]\n")]);
    assert!(!validate(dir.path()).is_empty(), "the fixture must warn");
    let (p, origins) = load_checked(dir.path()).unwrap();
    assert!(p.agents.contains_key("dev"));
    assert!(origins.files.contains_key(&(Kind::Agent, "dev".into())));
}

#[test]
fn an_output_outside_the_project_or_into_its_sources_is_an_error() {
    for bad in [
        "/etc/claude",
        "../elsewhere",
        ".armadai/out",
        ".git/hooks",
        ".",
    ] {
        let ds = diags(&[(
            ".armadai/project.yaml",
            &format!("schema_version: 1\noutputs:\n  claude: '{bad}'\n"),
        )]);
        let d = err(&ds, "bad-output", (3, 12));
        assert!(d.message.contains(bad), "{d:?}");
    }
    let ds = diags(&[(
        ".armadai/project.yaml",
        "schema_version: 1\noutputs:\n  claude: tools/claude\n",
    )]);
    assert!(!ds.iter().any(|d| d.code == "bad-output"), "{ds:?}");
}
