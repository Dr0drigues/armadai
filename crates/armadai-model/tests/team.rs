//! Team graph: delegation edges from `allowed_targets`, exact renderings.

use std::collections::BTreeSet;
use std::path::Path;

use armadai_model::*;

const LOT: &str = "start: spec
steps:
  frame: { agent: framer }
  plan: { agent: architect }
  review: { agent: reviewer, states: [approved, rejected] }
  spec: { agent: analyst }
  validate: { agent: validator, states: [approved, rejected] }
links:
  - { from: spec, to: frame }
  - { from: frame, to: plan }
  - { from: plan, to: review }
  - { from: review, to: validate, when: approved }
  - { from: review, to: plan, when: rejected }
  - { from: validate, to: plan, when: rejected }
";

const TASK: &str = "start: build
steps:
  build: { agent: lead }
  build-adapter: { agent: adapter-specialist }
  build-cli: { agent: cli-specialist }
  build-schema: { agent: schema-specialist }
  guard: { agent: guardian, states: [approved, rejected] }
links:
  - { from: build, to: build-schema }
  - { from: build, to: build-adapter }
  - { from: build, to: build-cli }
  - { from: build-schema, to: guard }
  - { from: build-adapter, to: guard }
  - { from: build-cli, to: guard }
  - { from: guard, to: build-schema, when: rejected }
  - { from: guard, to: build-adapter, when: rejected }
  - { from: guard, to: build-cli, when: rejected }
";

const REPO: &str = "schema_version: 1
coordinator: lead
policy:
  delegation: strict
  free_agents: [Explore, Plan, general-purpose, claude-code-guide, claude]
";

const AGENTS: [&str; 10] = [
    "lead",
    "analyst",
    "framer",
    "architect",
    "schema-specialist",
    "adapter-specialist",
    "cli-specialist",
    "guardian",
    "reviewer",
    "validator",
];

fn project(manifest: &str, agents: &[&str], flows: &[(&str, &str)]) -> Project {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join(".armadai");
    std::fs::create_dir_all(base.join("agents")).unwrap();
    std::fs::create_dir_all(base.join("flows")).unwrap();
    std::fs::write(base.join("project.yaml"), manifest).unwrap();
    for a in agents {
        std::fs::write(
            base.join(format!("agents/{a}.md")),
            "---\ndescription: d\n---\nB\n",
        )
        .unwrap();
    }
    for (id, text) in flows {
        std::fs::write(base.join(format!("flows/{id}.yaml")), text).unwrap();
    }
    load(dir.path())
}

fn load(root: &Path) -> Project {
    let l = load_project(root);
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    l.project.unwrap()
}

fn repo() -> Project {
    project(REPO, &AGENTS, &[("lot", LOT), ("task", TASK)])
}

fn edge(from: Option<&str>, to: &str) -> TeamEdge {
    TeamEdge {
        from: from.map(Into::into),
        to: to.into(),
    }
}

fn targets(g: &TeamGraph, from: Option<&str>) -> Vec<String> {
    (g.edges.iter())
        .filter(|e| e.from.as_deref() == from)
        .map(|e| e.to.clone())
        .collect()
}

#[test]
fn the_repository_team() {
    let g = team_graph(&repo());
    assert_eq!(
        (g.delegation.as_str(), g.coordinator.as_deref()),
        ("strict", Some("lead"))
    );
    let mut expected = vec![];
    for t in [
        "adapter-specialist",
        "analyst",
        "architect",
        "cli-specialist",
        "framer",
        "guardian",
        "reviewer",
        "schema-specialist",
        "validator",
    ] {
        expected.push(edge(None, t));
    }
    // each agent, by id, reaches what its steps lead to
    expected.extend([
        edge(Some("adapter-specialist"), "guardian"),
        edge(Some("analyst"), "framer"),
        edge(Some("architect"), "reviewer"),
        edge(Some("cli-specialist"), "guardian"),
        edge(Some("framer"), "architect"),
        edge(Some("guardian"), "adapter-specialist"),
        edge(Some("guardian"), "cli-specialist"),
        edge(Some("guardian"), "schema-specialist"),
        edge(Some("reviewer"), "architect"),
        edge(Some("reviewer"), "validator"),
        edge(Some("schema-specialist"), "guardian"),
        edge(Some("validator"), "architect"),
    ]);
    assert_eq!(g.edges, expected);
    // criterion 7
    assert!(g.edges.iter().all(|e| e.to != "lead"));
    for b in ["adapter-specialist", "cli-specialist", "schema-specialist"] {
        assert!(targets(&g, Some(b)).contains(&"guardian".to_string()));
        assert!(targets(&g, Some("guardian")).contains(&b.to_string()));
    }
    assert_eq!(
        g.free_agents,
        [
            "Explore",
            "Plan",
            "claude",
            "claude-code-guide",
            "general-purpose"
        ]
    );
    let guardian = g.agents.iter().find(|a| a.id == "guardian").unwrap();
    assert_eq!(guardian.flows, ["task"]);
    assert!(g.agents.iter().all(|a| a.id != "lead"));
}

#[test]
fn without_a_coordinator_the_main_thread_reaches_entry_steps_only() {
    let p = project(
        "schema_version: 1\n",
        &["dev", "qa"],
        &[(
            "ship",
            "steps:\n  build: { agent: dev }\n  test: { agent: qa }\nlinks:\n  - { from: build, to: test }\n",
        )],
    );
    let g = team_graph(&p);
    assert_eq!(g.delegation, "off");
    assert_eq!(g.edges, [edge(None, "dev"), edge(Some("dev"), "qa")]);
}

/// Every caller's targets are `allowed_targets` less the free agents, and the
/// coordinator, who is also the agent of a step here, is only the main thread.
#[test]
fn team_edges_match_allowed_targets() {
    let p = repo();
    let g = team_graph(&p);
    let free: BTreeSet<String> = g.free_agents.iter().cloned().collect();
    let mut callers: Vec<Option<&str>> = vec![None];
    callers.extend(g.agents.iter().map(|a| Some(a.id.as_str())));
    for c in callers {
        let want: BTreeSet<String> = allowed_targets(&p, c).difference(&free).cloned().collect();
        let got: BTreeSet<String> = targets(&g, c).into_iter().collect();
        assert_eq!(got, want, "caller {c:?}");
    }
    assert!(g.edges.iter().all(|e| e.from.as_deref() != Some("lead")));
    // free agents are allowed to everyone yet never an edge
    assert!(allowed_targets(&p, None).contains("Explore"));
    assert!(g.edges.iter().all(|e| !free.contains(&e.to)));
}

#[test]
fn free_agents_are_a_group_not_edges() {
    let g = team_graph(&repo());
    assert!(g.edges.iter().all(|e| !g.free_agents.contains(&e.to)));
    let m = team_to_mermaid(&g);
    assert!(
        m.contains("subgraph free[\"free agents, callable from anywhere\"]"),
        "{m}"
    );
    assert!(!m.contains("--> n_Explore"), "{m}");
}

#[test]
fn policy_off_keeps_the_edges() {
    let off = REPO.replace("strict", "off");
    let g = team_graph(&project(&off, &AGENTS, &[("lot", LOT), ("task", TASK)]));
    assert_eq!(g.delegation, "off");
    assert_eq!(g.edges, team_graph(&repo()).edges);
    assert!(team_to_mermaid(&g).starts_with("%% team (delegation: off, not enforced)\n"));
    assert!(team_to_dot(&g).starts_with("// team (delegation: off, not enforced)\n"));
}

#[test]
fn team_to_mermaid_golden() {
    let g = team_graph(&repo());
    assert_eq!(
        team_to_mermaid(&g),
        r#"%% team (delegation: strict, enforced by the gate)
flowchart TD
  main(["main thread (lead)"])
  n_adapter_specialist["adapter-specialist · task"]
  n_analyst["analyst · lot"]
  n_architect["architect · lot"]
  n_cli_specialist["cli-specialist · task"]
  n_framer["framer · lot"]
  n_guardian["guardian · task"]
  n_reviewer["reviewer · lot"]
  n_schema_specialist["schema-specialist · task"]
  n_validator["validator · lot"]
  subgraph free["free agents, callable from anywhere"]
    n_Explore["Explore"]
    n_Plan["Plan"]
    n_claude["claude"]
    n_claude_code_guide["claude-code-guide"]
    n_general_purpose["general-purpose"]
  end
  main --> n_adapter_specialist
  main --> n_analyst
  main --> n_architect
  main --> n_cli_specialist
  main --> n_framer
  main --> n_guardian
  main --> n_reviewer
  main --> n_schema_specialist
  main --> n_validator
  n_adapter_specialist --> n_guardian
  n_analyst --> n_framer
  n_architect --> n_reviewer
  n_cli_specialist --> n_guardian
  n_framer --> n_architect
  n_guardian --> n_adapter_specialist
  n_guardian --> n_cli_specialist
  n_guardian --> n_schema_specialist
  n_reviewer --> n_architect
  n_reviewer --> n_validator
  n_schema_specialist --> n_guardian
  n_validator --> n_architect
"#
    );
}

#[test]
fn team_to_dot_golden() {
    let g = team_graph(&repo());
    assert_eq!(
        team_to_dot(&g),
        r#"// team (delegation: strict, enforced by the gate)
digraph "team" {
  "main thread" [label="main thread (lead)", shape=box, style="rounded,bold"];
  "adapter-specialist" [label="adapter-specialist · task", shape=box];
  "analyst" [label="analyst · lot", shape=box];
  "architect" [label="architect · lot", shape=box];
  "cli-specialist" [label="cli-specialist · task", shape=box];
  "framer" [label="framer · lot", shape=box];
  "guardian" [label="guardian · task", shape=box];
  "reviewer" [label="reviewer · lot", shape=box];
  "schema-specialist" [label="schema-specialist · task", shape=box];
  "validator" [label="validator · lot", shape=box];
  subgraph "cluster_free" {
    label="free agents, callable from anywhere";
    "Explore" [shape=box];
    "Plan" [shape=box];
    "claude" [shape=box];
    "claude-code-guide" [shape=box];
    "general-purpose" [shape=box];
  }
  "main thread" -> "adapter-specialist";
  "main thread" -> "analyst";
  "main thread" -> "architect";
  "main thread" -> "cli-specialist";
  "main thread" -> "framer";
  "main thread" -> "guardian";
  "main thread" -> "reviewer";
  "main thread" -> "schema-specialist";
  "main thread" -> "validator";
  "adapter-specialist" -> "guardian";
  "analyst" -> "framer";
  "architect" -> "reviewer";
  "cli-specialist" -> "guardian";
  "framer" -> "architect";
  "guardian" -> "adapter-specialist";
  "guardian" -> "cli-specialist";
  "guardian" -> "schema-specialist";
  "reviewer" -> "architect";
  "reviewer" -> "validator";
  "schema-specialist" -> "guardian";
  "validator" -> "architect";
}
"#
    );
}

#[test]
fn team_json_golden() {
    let p = project(
        "schema_version: 1\ncoordinator: lead\npolicy:\n  free_agents: [Explore]\n",
        &["lead", "dev"],
        &[("ship", "steps:\n  build: { agent: dev }\n")],
    );
    assert_eq!(
        serde_json::to_string(&team_graph(&p)).unwrap(),
        r#"{"delegation":"off","coordinator":"lead","agents":[{"id":"dev","flows":["ship"]}],"edges":[{"from":null,"to":"dev"}],"free_agents":["Explore"]}"#
    );
}
