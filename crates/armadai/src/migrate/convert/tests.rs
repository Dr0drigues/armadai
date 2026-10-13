use std::collections::BTreeMap;
use std::path::PathBuf;

use armadai_core::agent::{Agent, AgentMetadata, PipelineConfig};
use armadai_core::orchestration::{OrchestrationConfig, OrchestrationPattern, TeamConfig};
use armadai_core::project::{LinkConfig, LinkOverride, ProjectConfig};
use armadai_core::prompt::Prompt;
use armadai_model::{Delegation, Tier};

use super::*;
use crate::migrate::read::{OldProject, OldSkill};

fn meta(provider: &str, model: Option<&str>) -> AgentMetadata {
    AgentMetadata {
        provider: provider.into(),
        model: model.map(str::to_string),
        command: None,
        args: None,
        temperature: 0.7,
        max_tokens: None,
        timeout: None,
        tags: Vec::new(),
        stacks: Vec::new(),
        scope: Vec::new(),
        model_fallback: Vec::new(),
        cost_limit: None,
        rate_limit: None,
        context_window: None,
        mode: None,
        orchestration: None,
        triggers: None,
        ring_config: None,
    }
}

fn agent(name: &str) -> Agent {
    Agent {
        name: name.into(),
        source: PathBuf::from(format!("/p/.armadai/agents/{}.md", slug(name))),
        metadata: meta("anthropic", None),
        system_prompt: format!("You are {name}.\nMore."),
        instructions: Some("Do it.".into()),
        output_format: None,
        pipeline: None,
        context: None,
    }
}

fn slug(n: &str) -> String {
    armadai_core::agent::slugify(n)
}

fn old(agents: Vec<Agent>) -> OldProject {
    OldProject {
        root: PathBuf::from("/p"),
        config: ProjectConfig::default(),
        config_file: PathBuf::from("/p/.armadai/config.yaml"),
        agents,
        prompts: Vec::new(),
        skills: Vec::new(),
        declared_descriptions: BTreeMap::new(),
        leftovers: BTreeMap::new(),
        fragment_sources: Vec::new(),
        load_warnings: Vec::new(),
    }
}

fn dropped(r: &Report, subject: &str) -> Vec<String> {
    let mut v: Vec<String> = (r.dropped.iter())
        .filter(|d| d.subject == subject)
        .map(|d| d.field.clone())
        .collect();
    v.sort();
    v
}

/// The converted project, written and validated by `armadai-model`.
fn assert_valid(p: &Project) {
    let dir = tempfile::tempdir().unwrap();
    armadai_model::write_project(p, dir.path()).unwrap();
    let ds = armadai_model::validate(dir.path());
    assert!(
        !armadai_model::has_errors(&ds),
        "{}",
        ds.iter().map(|d| d.render()).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn agent_body_title_and_derived_description() {
    let (p, _, r) = convert(&old(vec![agent("Dev Lead")]));
    let a = &p.agents["dev-lead"];
    assert_eq!(a.front.title.as_deref(), Some("Dev Lead"));
    assert_eq!(a.front.description, "You are Dev Lead.");
    assert_eq!(
        a.body,
        "You are Dev Lead.\nMore.\n\n## Instructions\n\nDo it."
    );
    assert!(r.warnings.iter().any(|w| w.contains("description derived")));
    assert_valid(&p);
}

#[test]
fn slug_collision_gets_a_suffix_and_a_warning() {
    let (p, _, r) = convert(&old(vec![agent("Dev Lead"), agent("dev_lead")]));
    assert_eq!(
        p.agents.keys().collect::<Vec<_>>(),
        ["dev-lead", "dev-lead-2"]
    );
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("`dev_lead` renamed to `dev-lead-2`")),
        "{:?}",
        r.warnings
    );
}

#[test]
fn empty_or_non_ascii_h1_gets_a_numbered_id() {
    let (p, _, r) = convert(&old(vec![agent("Développeur"), agent("日本"), agent("")]));
    assert_eq!(
        p.agents.keys().collect::<Vec<_>>(),
        ["agent-2", "agent-3", "developpeur"]
    );
    assert_eq!(
        r.warnings.iter().filter(|w| w.contains("renamed")).count(),
        3
    );
    assert_valid(&p);
}

#[test]
fn long_name_is_cut_to_a_valid_id() {
    let (p, _, _) = convert(&old(vec![agent(&"a".repeat(80))]));
    assert_eq!(p.agents.keys().next().unwrap().len(), 64);
    assert_valid(&p);
}

#[test]
fn default_temperature_is_not_written() {
    let mut cold = agent("Cold");
    cold.metadata.temperature = 0.2;
    let (p, _, _) = convert(&old(vec![agent("Warm"), cold]));
    assert_eq!(p.agents["warm"].front.temperature, None);
    assert_eq!(p.agents["cold"].front.temperature, Some(0.2));
}

#[test]
fn latest_tier_maps_to_tier() {
    let mut a = agent("A");
    a.metadata.model = Some("latest:fast".into());
    let mut b = agent("B");
    b.metadata.model = Some("latest:high".into());
    let (p, _, r) = convert(&old(vec![a, b]));
    assert_eq!(p.agents["a"].front.tier, Some(Tier::Fast));
    assert_eq!(p.agents["b"].front.tier, Some(Tier::Max));
    assert!(!dropped(&r, "a").contains(&"model".to_string()));
}

#[test]
fn latest_auto_is_dropped() {
    let mut a = agent("A");
    a.metadata.model = Some("latest:auto".into());
    let (p, _, r) = convert(&old(vec![a]));
    assert_eq!(p.agents["a"].front.tier, None);
    assert!(dropped(&r, "a").contains(&"model".to_string()));
}

#[test]
fn concrete_model_goes_native_by_provider() {
    let mut a = agent("A");
    a.metadata.model = Some("claude-sonnet-4-5".into());
    let mut b = agent("B");
    b.metadata = meta("openai", Some("gpt-5"));
    let (p, _, _) = convert(&old(vec![a, b]));
    let model = |id: &str, t: &str| {
        p.agents[id].front.native[t]
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    assert_eq!(model("a", "claude").as_deref(), Some("claude-sonnet-4-5"));
    assert_eq!(model("b", "codex").as_deref(), Some("gpt-5"));
    assert_valid(&p);
}

#[test]
fn unknown_provider_model_is_dropped() {
    let mut a = agent("A");
    a.metadata = meta("google", Some("gemini-2.5-pro"));
    let (p, _, r) = convert(&old(vec![a]));
    assert!(p.agents["a"].front.native.is_empty());
    assert!(dropped(&r, "a").contains(&"model".to_string()));
}

#[test]
fn every_dead_field_is_reported() {
    let mut a = agent("A");
    let m = &mut a.metadata;
    m.command = Some("claude".into());
    m.args = Some(vec!["-p".into()]);
    m.max_tokens = Some(10);
    m.timeout = Some(10);
    m.stacks = vec!["rust".into()];
    m.scope = vec!["src/".into()];
    m.model_fallback = vec!["x".into()];
    m.cost_limit = Some(1.0);
    m.rate_limit = Some("1/min".into());
    m.context_window = Some(1000);
    m.mode = Some(armadai_core::agent::AgentMode::Guided);
    m.orchestration = Some(OrchestrationPattern::Ring);
    m.triggers = Some(armadai_model::yaml::from_str("requires: [x]").unwrap());
    m.ring_config = Some(armadai_model::yaml::from_str("role: proposer").unwrap());
    a.pipeline = Some(PipelineConfig {
        next: vec!["b".into()],
    });
    let (_, _, r) = convert(&old(vec![a]));
    assert_eq!(
        dropped(&r, "a"),
        [
            "args",
            "command",
            "context_window",
            "cost_limit",
            "max_tokens",
            "mode",
            "model_fallback",
            "orchestration",
            "pipeline",
            "provider",
            "rate_limit",
            "ring_config",
            "scope",
            "stacks",
            "timeout",
            "triggers",
        ]
    );
    // a default agent reports only its provider
    let (_, _, r) = convert(&old(vec![agent("B")]));
    assert_eq!(dropped(&r, "b"), ["provider"]);
}

#[test]
fn tags_are_kept() {
    let mut a = agent("A");
    a.metadata.tags = vec!["dev".into()];
    let (p, _, _) = convert(&old(vec![a]));
    assert_eq!(p.agents["a"].front.tags, ["dev"]);
}

#[test]
fn declared_description_is_kept() {
    let mut o = old(vec![agent("Core Dev")]);
    o.declared_descriptions
        .insert("Core Dev".into(), "Writes the core.".into());
    let (p, _, r) = convert(&o);
    assert_eq!(p.agents["core-dev"].front.description, "Writes the core.");
    assert!(!r.warnings.iter().any(|w| w.contains("description derived")));
}

#[test]
fn empty_prompt_still_gets_a_description() {
    let mut a = agent("Mute");
    a.system_prompt = String::new();
    a.instructions = None;
    let (p, _, _) = convert(&old(vec![a]));
    assert_eq!(p.agents["mute"].front.description, "Mute");
    assert_valid(&p);
}

#[test]
fn prompt_becomes_a_skill() {
    let mut o = old(vec![]);
    o.prompts = vec![
        Prompt {
            name: "Rust Conventions".into(),
            description: Some("Rules.".into()),
            apply_to: vec!["dev".into()],
            body: "Use clippy.\n".into(),
            source: "/p/.armadai/prompts/rust-conventions.md".into(),
        },
        Prompt {
            name: "bare".into(),
            description: None,
            apply_to: vec![],
            body: "\n  First line.\n".into(),
            source: "/p/.armadai/prompts/bare.md".into(),
        },
    ];
    let (p, _, r) = convert(&o);
    let s = &p.skills["rust-conventions"];
    assert_eq!(s.front.name.as_deref(), Some("rust-conventions"));
    assert_eq!(s.front.description.as_deref(), Some("Rules."));
    assert_eq!(s.body, "Use clippy.\n");
    assert_eq!(
        p.skills["bare"].front.description.as_deref(),
        Some("First line.")
    );
    assert_eq!(dropped(&r, "rust-conventions"), ["apply_to"]);
    assert_valid(&p);
}

#[test]
fn legacy_skill_keeps_its_keys_and_drops_tools_and_version() {
    let mut o = old(vec![]);
    o.skills = vec![OldSkill {
        dir: "/p/.armadai/skills/review".into(),
        text: "---\nname: review\ndescription: Checks.\nversion: '1.0'\ntools: [Read]\nlicense: MIT\nallowed-tools: Bash\n---\nBody.\n".into(),
    }];
    let (p, copies, r) = convert(&o);
    let s = &p.skills["review"];
    assert_eq!(s.front.license.as_deref(), Some("MIT"));
    assert!(s.front.extra.contains_key("allowed-tools"));
    assert!(!s.front.extra.contains_key("tools") && !s.front.extra.contains_key("version"));
    assert_eq!(s.body, "Body.\n");
    assert_eq!(dropped(&r, "review"), ["tools", "version"]);
    assert_eq!(
        copies,
        [SkillCopy {
            id: "review".into(),
            from: "/p/.armadai/skills/review".into()
        }]
    );
    assert_valid(&p);
}

#[test]
fn skill_and_prompt_ids_do_not_collide() {
    let mut o = old(vec![]);
    o.prompts = vec![Prompt {
        name: "review".into(),
        description: Some("d".into()),
        apply_to: vec![],
        body: "b".into(),
        source: "/p/.armadai/prompts/review.md".into(),
    }];
    o.skills = vec![OldSkill {
        dir: "/p/.armadai/skills/review".into(),
        text: "---\ndescription: Checks.\n---\nBody.\n".into(),
    }];
    let (p, copies, _) = convert(&o);
    assert_eq!(p.skills.len(), 2);
    assert_eq!(copies[0].id, "review");
    assert_eq!(p.skills["review-2"].body, "b");
    assert_valid(&p);
}

fn with_link(link: LinkConfig) -> OldProject {
    let mut o = old(vec![agent("Dev Lead"), agent("Worker")]);
    o.config.link = Some(link);
    o
}

#[test]
fn gemini_target_is_dropped() {
    let o = with_link(LinkConfig {
        target: Some("gemini".into()),
        ..Default::default()
    });
    let (p, _, r) = convert(&o);
    assert!(p.manifest.targets.is_empty());
    assert_eq!(dropped(&r, "project"), ["link.target"]);
    assert!(r.warnings.iter().any(|w| w.contains("gemini")));
}

#[test]
fn link_target_outputs_and_coordinator() {
    let mut overrides = std::collections::HashMap::new();
    overrides.insert(
        "claude".to_string(),
        LinkOverride {
            output: Some(".claude".into()),
        },
    );
    overrides.insert(
        "gemini".to_string(),
        LinkOverride {
            output: Some(".gemini".into()),
        },
    );
    let o = with_link(LinkConfig {
        target: Some("claude".into()),
        coordinator: Some("Dev Lead".into()),
        overrides,
    });
    let (p, _, r) = convert(&o);
    assert_eq!(p.manifest.targets, ["claude"]);
    assert_eq!(p.manifest.outputs["claude"], ".claude");
    assert_eq!(p.manifest.outputs.len(), 1);
    assert_eq!(p.manifest.coordinator.as_deref(), Some("dev-lead"));
    assert_eq!(dropped(&r, "project"), ["link.overrides.gemini"]);
    assert_valid(&p);
}

#[test]
fn unresolved_link_coordinator_warns() {
    let o = with_link(LinkConfig {
        coordinator: Some("nobody".into()),
        ..Default::default()
    });
    let (p, _, r) = convert(&o);
    assert_eq!(p.manifest.coordinator, None);
    assert!(r.warnings.iter().any(|w| w.contains("nobody")));
}

fn with_orchestration(o: OrchestrationConfig) -> OldProject {
    let mut p = old(vec![
        agent("Dev Lead"),
        agent("Core Lead"),
        agent("Worker"),
        agent("Tester"),
    ]);
    p.config.orchestration = Some(Box::new(o));
    p
}

#[test]
fn orchestration_becomes_a_flow_and_policy() {
    let o = with_orchestration(OrchestrationConfig {
        enabled: true,
        pattern: OrchestrationPattern::Hierarchical,
        coordinator: Some("dev-lead".into()),
        teams: vec![
            TeamConfig {
                lead: Some("Core Lead".into()),
                agents: vec!["worker".into()],
                ..Default::default()
            },
            TeamConfig {
                agents: vec!["tester".into()],
                ..Default::default()
            },
        ],
        policy: armadai_core::orchestration::policy::PolicyMode::Strict,
        free_agents: vec![
            "tester".into(),
            "Core Lead".into(),
            "Explore".into(),
            "general-purpose".into(),
            "Some Helper".into(),
        ],
        max_depth: Some(3),
        ..Default::default()
    });
    let (p, _, r) = convert(&o);
    let f = &p.flows["orchestration"];
    assert_eq!(
        f.steps.keys().collect::<Vec<_>>(),
        ["core-lead", "dev-lead", "tester", "worker"]
    );
    assert_eq!(f.steps["core-lead"].agent, "core-lead");
    let links: Vec<(&str, &str)> = (f.links.iter())
        .map(|l| (l.from.as_str(), l.to.as_str()))
        .collect();
    assert_eq!(
        links,
        [
            ("dev-lead", "core-lead"),
            ("core-lead", "worker"),
            ("dev-lead", "tester")
        ]
    );
    assert_eq!(p.manifest.policy.delegation, Delegation::Strict);
    // `Explore` is Claude Code's own agent: kept, not dropped; a title is renamed
    assert_eq!(
        p.manifest.policy.free_agents,
        [
            "tester",
            "core-lead",
            "Explore",
            "general-purpose",
            "Some Helper"
        ]
    );
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("`Core Lead` renamed to `core-lead`")),
        "{:?}",
        r.warnings
    );
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("`Some Helper`") && w.contains("kept as is")),
        "{:?}",
        r.warnings
    );
    // Claude Code's own agents are known: no warning
    for quiet in ["`tester`", "`general-purpose`", "`Explore`"] {
        assert!(
            !r.warnings.iter().any(|w| w.contains(quiet)),
            "{:?}",
            r.warnings
        );
    }
    assert_eq!(
        dropped(&r, "orchestration"),
        ["enabled", "max_depth", "pattern"]
    );
    assert_valid(&p);
}

#[test]
fn unknown_agent_in_orchestration_warns_and_is_omitted() {
    let o = with_orchestration(OrchestrationConfig {
        coordinator: Some("ghost".into()),
        teams: vec![TeamConfig {
            lead: Some("phantom".into()),
            agents: vec!["worker".into(), "nobody".into()],
            ..Default::default()
        }],
        ..Default::default()
    });
    let (p, _, r) = convert(&o);
    let f = &p.flows["orchestration"];
    assert_eq!(f.steps.keys().collect::<Vec<_>>(), ["worker"]);
    assert!(f.links.is_empty());
    for name in ["ghost", "phantom", "nobody"] {
        assert!(
            r.warnings.iter().any(|w| w.contains(&format!("`{name}`"))),
            "{name}: {:?}",
            r.warnings
        );
    }
    assert_valid(&p);
}

#[test]
fn orchestration_without_any_known_agent_writes_no_flow() {
    let o = with_orchestration(OrchestrationConfig {
        coordinator: Some("ghost".into()),
        ..Default::default()
    });
    let (p, _, r) = convert(&o);
    assert!(p.flows.is_empty());
    assert!(r.warnings.iter().any(|w| w.contains("no flow")));
    assert_valid(&p);
}

#[test]
fn self_links_are_never_produced() {
    let o = with_orchestration(OrchestrationConfig {
        coordinator: Some("dev-lead".into()),
        teams: vec![TeamConfig {
            lead: Some("dev-lead".into()),
            agents: vec!["dev-lead".into(), "worker".into(), "worker".into()],
            ..Default::default()
        }],
        ..Default::default()
    });
    let (p, _, _) = convert(&o);
    let f = &p.flows["orchestration"];
    assert_eq!(f.links.len(), 1);
    assert_valid(&p);
}

#[test]
fn project_level_dead_sections_are_reported() {
    let mut o = old(vec![agent("A")]);
    o.config = armadai_model::yaml::from_str(
        "sources: [x]\ndefaults: { mode: guided }\nrouting: {}\nregistries: {}\n",
    )
    .unwrap();
    let (_, _, r) = convert(&o);
    assert_eq!(
        dropped(&r, "project"),
        ["defaults", "registries", "routing", "sources"]
    );
}

#[test]
fn generic_limits_are_always_stated() {
    let (_, _, r) = convert(&old(vec![]));
    assert!(r.warnings.iter().any(|w| w.contains("YAML comments")));
    assert!(r.warnings.iter().any(|w| w.contains("not detected")));
}

#[test]
fn load_warnings_and_library_agents_are_reported() {
    let mut lib = agent("Lib");
    lib.source = "/home/u/.config/armadai/agents/lib.md".into();
    let mut o = old(vec![lib]);
    o.load_warnings = vec!["agent not migrated: x".into()];
    let (_, _, r) = convert(&o);
    assert!(r.warnings.contains(&"agent not migrated: x".to_string()));
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("copied from the user library"))
    );
}

#[test]
fn orchestration_names_resolve_by_file_stem_too() {
    let mut lead = agent("Dev Lead");
    lead.source = "/p/.armadai/agents/lead.md".into();
    let mut o = old(vec![lead, agent("Worker")]);
    o.config.orchestration = Some(Box::new(OrchestrationConfig {
        coordinator: Some("lead".into()),
        teams: vec![TeamConfig {
            agents: vec!["worker".into()],
            ..Default::default()
        }],
        ..Default::default()
    }));
    let (p, _, _) = convert(&o);
    assert_eq!(p.flows["orchestration"].links[0].from, "dev-lead");
}

#[test]
fn leftover_text_is_appended_verbatim_and_reported() {
    let a = agent("Dev");
    let mut o = old(vec![a.clone()]);
    o.leftovers.insert(
        a.source.clone(),
        crate::migrate::leftover::Leftover {
            kept: vec![
                ("preamble".into(), "Intro.".into()),
                ("## Examples".into(), "## Examples\n\nEx.".into()),
            ],
            unknown_metadata: vec!["flavour".into()],
        },
    );
    let (p, _, r) = convert(&o);
    assert_eq!(
        p.agents["dev"].body,
        "You are Dev.\nMore.\n\n## Instructions\n\nDo it.\n\nIntro.\n\n## Examples\n\nEx."
    );
    for label in ["preamble", "## Examples"] {
        assert!(
            r.warnings
                .contains(&format!("agent `dev`: kept verbatim: {label}")),
            "{:?}",
            r.warnings
        );
    }
    assert!(dropped(&r, "dev").contains(&"metadata.flavour".to_string()));
}

#[test]
fn provider_and_derived_descriptions_are_grouped_in_the_report() {
    let names = ["A", "B", "C", "D"];
    let (_, _, r) = convert(&old(names.iter().map(|n| agent(n)).collect()));
    let out = r.render();
    assert!(out.contains("provider: dropped for 4 agents"), "{out}");
    assert!(!out.contains("a: provider"), "{out}");
    assert!(
        out.contains(
            "description derived from the first sentence of the prompt for 4 agents: a, b, c, d"
        ),
        "{out}"
    );
    let (_, _, r) = convert(&old(vec![agent("A")]));
    assert!(
        r.render()
            .contains("agent `a`: description derived from the first sentence of its prompt"),
        "{}",
        r.render()
    );
}

#[test]
fn common_accents_are_transliterated_and_the_rename_reported() {
    let (p, _, r) = convert(&old(vec![agent("Développeur Œuvre"), agent("Revue")]));
    assert!(p.agents.contains_key("developpeur-oeuvre"));
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("`Développeur Œuvre` renamed to `developpeur-oeuvre`")),
        "{:?}",
        r.warnings
    );
    assert!(!r.warnings.iter().any(|w| w.contains("`Revue` renamed")));
}

#[test]
fn kept_text_after_an_empty_section_is_separated_by_one_blank_line() {
    let mut a = agent("Dev");
    a.instructions = None;
    a.output_format = Some(String::new());
    let mut o = old(vec![a.clone()]);
    o.leftovers.insert(
        a.source.clone(),
        crate::migrate::leftover::Leftover {
            kept: vec![("## Report".into(), "## Report\n\nR.".into())],
            unknown_metadata: Vec::new(),
        },
    );
    let (p, _, _) = convert(&o);
    assert_eq!(
        p.agents["dev"].body,
        "You are Dev.\nMore.\n\n## Output Format\n\n## Report\n\nR."
    );
}

#[test]
fn derived_description_is_the_first_whole_sentence() {
    // Wrapped paragraph: sentence ends mid-line, later lines are ignored.
    assert_eq!(
        first_line("You are a reviewer. You focus on\nsafety.\n\nSecond.", 200).unwrap(),
        "You are a reviewer."
    );
    // No terminator: the paragraph is joined on one line.
    assert_eq!(first_line("  one\n two  \n\nnext", 200).unwrap(), "one two");
    assert_eq!(first_line("Really? Yes.", 200).unwrap(), "Really?");
    assert_eq!(
        first_line("v1.2 is out. Done", 200).unwrap(),
        "v1.2 is out."
    );
    assert_eq!(first_line("\n\n  \n", 200), None);
}

#[test]
fn derived_description_longer_than_the_cap_is_cut_at_a_word() {
    let d = first_line("alpha beta gamma delta", 12).unwrap();
    assert_eq!(d, "alpha beta…");
    assert_eq!(first_line("ab cd efgh", 5).unwrap(), "ab cd…"); // space exactly at the cap
    let d = first_line("abcdefghijklmnop", 5).unwrap(); // no space: hard cut
    assert_eq!(d, "abcde…");
    assert!(d.chars().count() <= 6);
}
