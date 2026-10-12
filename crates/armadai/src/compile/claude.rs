//! Claude Code adapter: subagents, skills and the root `CLAUDE.md` block.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use armadai_model::{Agent, AgentFront, Cap, Delegation, Flow, Project, Tier};
use serde_yaml_ng::{Mapping, Value};

use super::{
    Artifact, ArtifactKind, BLOCK_BEGIN, Compiler, Plan, Report, Status, claude_settings, read_tree,
};
use crate::linker::manifest::ProducedBy;

const T: &str = "claude";

/// Capability → the Claude Code tools it governs (sub-agents doc, 2026-10-12).
const CAP_TOOLS: [(&str, &[&str]); 5] = [
    ("read", &["Read", "Grep", "Glob"]),
    ("write", &["Edit", "Write", "NotebookEdit"]),
    ("shell", &["Bash"]),
    ("web", &["WebFetch", "WebSearch"]),
    ("delegate", &["Agent"]),
];

const ASK: &str = "Claude Code asks per project/session permission, not per agent";
const MAIN: &str = "the coordinator runs as the main session (CLAUDE.md), not as a subagent";

pub struct ClaudeCompiler;

impl Compiler for ClaudeCompiler {
    fn compile(&self, project: &Project, root: &Path) -> anyhow::Result<Plan> {
        let mut plan = Plan::default();
        let coordinator = project.manifest.coordinator.as_deref();

        for (id, agent) in &project.agents {
            informational(&mut plan.report, id, &agent.front);
            if Some(id.as_str()) == coordinator {
                coordinator_lines(&mut plan.report, id, &agent.front);
                continue;
            }
            plan.artifacts.push(Artifact {
                path: PathBuf::from(format!("agents/{id}.md")),
                content: agent_file(id, agent, &mut plan.report)?.into_bytes(),
                kind: ArtifactKind::Agent,
                produced_by: ProducedBy::agent(id),
            });
            plan.report
                .push(T, format!("agent {id}"), Status::Compiled, None);
        }

        for id in project.skills.keys() {
            let dir = root.join(".armadai/skills").join(id);
            for (rel, content) in read_tree(&dir)? {
                plan.artifacts.push(Artifact {
                    path: Path::new("skills").join(id).join(rel),
                    content,
                    kind: ArtifactKind::Skill,
                    produced_by: ProducedBy::skill(id),
                });
            }
            plan.report
                .push(T, format!("skill {id}"), Status::Compiled, None);
        }
        plan.artifacts.sort_by(|a, b| a.path.cmp(&b.path));

        // computed here so a native collision fails the compilation, before any write
        claude_settings::hook_entries(project)?;
        for (id, a) in &project.automations {
            let reason = format!(
                "hook {} in .claude/settings.json",
                claude_settings::event_name(a.event)
            );
            plan.report.push(
                T,
                format!("automation {id}"),
                Status::Compiled,
                Some(&reason),
            );
        }
        if !claude_settings::native_settings(project)?.is_empty() {
            plan.report.push(
                T,
                "project native",
                Status::Compiled,
                Some("merged into .claude/settings.json"),
            );
        }
        if project.manifest.policy.delegation == Delegation::Strict {
            plan.report.push(
                T,
                "policy delegation=strict",
                Status::Compiled,
                Some("enforced: delegation topology, by the PreToolUse gate"),
            );
        }

        match coordinator {
            Some(c) => {
                let lead = project
                    .agents
                    .get(c)
                    .with_context(|| format!("coordinator `{c}` is not a declared agent"))?;
                plan.blocks
                    .push((PathBuf::from("CLAUDE.md"), claude_md(c, lead, project)));
                plan.report.push(
                    T,
                    format!("agent {c}"),
                    Status::Compiled,
                    Some("coordinator, rendered into CLAUDE.md"),
                );
                let existing = std::fs::read_to_string(root.join("CLAUDE.md")).ok();
                if existing.is_some_and(|t| !t.contains(BLOCK_BEGIN)) {
                    plan.report.push(
                        T,
                        "CLAUDE.md",
                        Status::Approximated,
                        Some("existing CLAUDE.md kept, ArmadAI block appended at its end"),
                    );
                }
                for (fid, flow) in &project.flows {
                    plan.report.push(
                        T,
                        format!("flow {fid}"),
                        Status::Approximated,
                        Some("rendered as guidance in CLAUDE.md, not enforced"),
                    );
                    for (sid, s) in &flow.steps {
                        if !s.states.is_empty() || s.on_failure.is_some() {
                            plan.report.push(
                                T,
                                format!("flow {fid} step {sid}"),
                                Status::Approximated,
                                Some("states and on_failure rendered as guidance"),
                            );
                        }
                    }
                }
            }
            None => {
                for fid in project.flows.keys() {
                    plan.report.push(
                        T,
                        format!("flow {fid}"),
                        Status::NotCompiled,
                        Some("no coordinator: no CLAUDE.md to render the flow into"),
                    );
                }
            }
        }
        Ok(plan)
    }
}

fn alias(tier: Tier) -> &'static str {
    match tier {
        Tier::Fast => "haiku",
        Tier::Pro => "sonnet",
        Tier::Max => "opus",
    }
}

fn caps(front: &AgentFront) -> [(&'static str, Option<Cap>); 5] {
    let c = &front.capabilities;
    // same order as CAP_TOOLS
    [
        ("read", c.read),
        ("write", c.write),
        ("shell", c.shell),
        ("web", c.web),
        ("delegate", c.delegate),
    ]
}

fn cap_str(c: Cap) -> &'static str {
    match c {
        Cap::Allow => "allow",
        Cap::Ask => "ask",
        Cap::Deny => "deny",
    }
}

/// Lines for fields Claude Code subagents have no place for.
fn informational(r: &mut Report, id: &str, f: &AgentFront) {
    if f.temperature.is_some() {
        r.push(
            T,
            format!("agent {id} temperature"),
            Status::NotCompiled,
            Some("no equivalent in Claude Code subagents"),
        );
    }
    if !f.tags.is_empty() {
        r.push(
            T,
            format!("agent {id} tags"),
            Status::NotCompiled,
            Some("informational, no Claude Code field"),
        );
    }
    if f.title.is_some() {
        r.push(
            T,
            format!("agent {id} title"),
            Status::NotCompiled,
            Some("Claude Code names a subagent by its id"),
        );
    }
}

fn coordinator_lines(r: &mut Report, id: &str, f: &AgentFront) {
    let mut fields = Vec::new();
    if caps(f).iter().any(|(_, c)| c.is_some()) {
        fields.push("capabilities");
    }
    if f.tier.is_some() {
        fields.push("tier");
    }
    if !f.skills.is_empty() {
        fields.push("skills");
    }
    if !f.native.is_empty() {
        fields.push("native");
    }
    for field in fields {
        r.push(
            T,
            format!("agent {id} {field}"),
            Status::NotCompiled,
            Some(MAIN),
        );
    }
}

fn agent_file(id: &str, agent: &Agent, r: &mut Report) -> anyhow::Result<String> {
    let f = &agent.front;
    let mut m = Mapping::new();
    m.insert("name".into(), id.into());
    m.insert("description".into(), f.description.as_str().into());
    if let Some(t) = f.tier {
        m.insert("model".into(), alias(t).into());
    }
    let mut denied = Vec::new();
    for ((name, cap), (_, tools)) in caps(f).into_iter().zip(CAP_TOOLS) {
        let Some(cap) = cap else { continue };
        let element = format!("agent {id} capability {name}={}", cap_str(cap));
        match cap {
            Cap::Allow => r.push(T, element, Status::Compiled, Some("inherited tools")),
            Cap::Ask => r.push(T, element, Status::Approximated, Some(ASK)),
            Cap::Deny => {
                denied.extend_from_slice(tools);
                r.push(T, element, Status::Compiled, Some("disallowedTools"));
            }
        }
    }
    if !denied.is_empty() {
        m.insert("disallowedTools".into(), denied.join(", ").into());
    }
    if !f.skills.is_empty() {
        let list: Vec<Value> = f.skills.iter().map(|s| s.as_str().into()).collect();
        m.insert("skills".into(), Value::Sequence(list));
        r.push(
            T,
            format!("agent {id} skills"),
            Status::Compiled,
            Some("preloaded through the subagent `skills` field"),
        );
    }
    if let Some(native) = f.native.get(T) {
        let Value::Mapping(native) = native else {
            bail!("agent `{id}`: native.claude must be a mapping");
        };
        for (k, v) in native {
            let key = k.as_str().unwrap_or_default();
            // `model` is the documented override of the tier; any other generated key wins nothing
            if key != "model" && m.contains_key(k) {
                bail!("agent `{id}`: native.claude key `{key}` collides with a generated key");
            }
            m.insert(k.clone(), v.clone());
        }
        r.push(
            T,
            format!("agent {id} native"),
            Status::Compiled,
            Some("merged into the subagent frontmatter"),
        );
    }
    let yaml = serde_yaml_ng::to_string(&m).context("subagent frontmatter")?;
    let mut out = format!("---\n{yaml}---\n{}", agent.body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Coordinator prompt + `## Delegation`, generated from the agents and the flows.
fn claude_md(coordinator: &str, lead: &Agent, project: &Project) -> String {
    let mut out = lead.body.trim_end().to_string();
    out.push_str("\n\n## Delegation\n\nDelegate with the Agent tool, naming the subagent by its id.\n\n### Agents\n\n");
    for (id, a) in project.agents.iter().filter(|(id, _)| *id != coordinator) {
        out.push_str(&format!("- `{id}`: {}\n", one_line(&a.front.description)));
    }
    for (fid, flow) in &project.flows {
        out.push_str(&flow_section(fid, flow));
    }
    out
}

fn flow_section(fid: &str, flow: &Flow) -> String {
    let mut out = format!("\n### Flow `{fid}`\n\n");
    if let Some(d) = &flow.description {
        out.push_str(&format!("{}\n\n", one_line(d)));
    }
    out.push_str("Steps:\n\n");
    for (sid, s) in &flow.steps {
        let mut detail = format!("agent `{}`", s.agent);
        if !s.states.is_empty() {
            detail.push_str(&format!(", states: {}", s.states.join(", ")));
        }
        if let Some(f) = &s.on_failure {
            detail.push_str(&format!(", on failure: `{f}`"));
        }
        out.push_str(&format!("- `{sid}` ({detail})\n"));
    }
    if !flow.links.is_empty() {
        out.push_str("\nTransitions:\n\n");
        for l in &flow.links {
            match &l.when {
                Some(w) => out.push_str(&format!("- `{}` -> `{}` when `{w}`\n", l.from, l.to)),
                None => out.push_str(&format!("- `{}` -> `{}`\n", l.from, l.to)),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::compile::{Artifact, ArtifactKind, Status};

    fn project(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let path = dir.path().join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        let p = armadai_model::load_checked(dir.path())
            .unwrap_or_else(|ds| panic!("{:?}", ds.iter().map(|d| d.render()).collect::<Vec<_>>()))
            .0;
        (dir, p)
    }

    fn compile(files: &[(&str, &[u8])]) -> anyhow::Result<Plan> {
        let (dir, p) = project(files);
        ClaudeCompiler.compile(&p, dir.path())
    }

    const M: (&str, &[u8]) = (".armadai/project.yaml", b"schema_version: 1\n");

    fn artifact<'a>(plan: &'a Plan, path: &str) -> &'a Artifact {
        plan.artifacts
            .iter()
            .find(|a| a.path == Path::new(path))
            .unwrap_or_else(|| {
                panic!(
                    "no {path} in {:?}",
                    plan.artifacts.iter().map(|a| &a.path).collect::<Vec<_>>()
                )
            })
    }

    fn text(plan: &Plan, path: &str) -> String {
        String::from_utf8(artifact(plan, path).content.clone()).unwrap()
    }

    fn front(plan: &Plan, path: &str) -> serde_yaml_ng::Mapping {
        let t = text(plan, path);
        let (yaml, _, _) = armadai_model::split_frontmatter(&t).unwrap();
        serde_yaml_ng::from_str(yaml).unwrap()
    }

    fn line<'a>(plan: &'a Plan, element: &str) -> &'a crate::compile::report::ReportLine {
        plan.report
            .lines
            .iter()
            .find(|l| l.element == element)
            .unwrap_or_else(|| panic!("no `{element}` in\n{}", plan.report.render_text()))
    }

    #[test]
    fn a_minimal_agent_compiles_to_a_subagent_file() {
        let plan = compile(&[
            M,
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: Writes code\n---\nYou write code.\n",
            ),
        ])
        .unwrap();
        assert_eq!(
            text(&plan, "agents/dev.md"),
            "---\nname: dev\ndescription: Writes code\n---\nYou write code.\n"
        );
        let a = artifact(&plan, "agents/dev.md");
        assert_eq!(a.kind, ArtifactKind::Agent);
        assert_eq!(
            a.produced_by,
            crate::linker::manifest::ProducedBy::agent("dev")
        );
        assert_eq!(line(&plan, "agent dev").status, Status::Compiled);
        assert!(plan.blocks.is_empty(), "no coordinator, no CLAUDE.md");
    }

    #[test]
    fn deny_becomes_disallowed_tools_and_ask_is_reported_as_approximated() {
        let plan = compile(&[
            M,
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: d\ncapabilities:\n  read: allow\n  write: deny\n  shell: ask\n  web: deny\n  delegate: deny\n---\nB\n",
            ),
        ])
        .unwrap();
        let f = front(&plan, "agents/dev.md");
        assert!(
            !f.contains_key("tools"),
            "ArmadAI never emits `tools`: {f:?}"
        );
        assert_eq!(
            f.get("disallowedTools").and_then(|v| v.as_str()),
            Some("Edit, Write, NotebookEdit, WebFetch, WebSearch, Agent")
        );
        let ask = line(&plan, "agent dev capability shell=ask");
        assert_eq!(ask.status, Status::Approximated);
        assert!(ask.reason.as_deref().unwrap().contains("not per agent"));
        assert_eq!(
            line(&plan, "agent dev capability write=deny").status,
            Status::Compiled
        );
        assert_eq!(
            line(&plan, "agent dev capability read=allow").status,
            Status::Compiled
        );
    }

    #[test]
    fn no_capability_means_no_tool_keys_at_all() {
        let plan = compile(&[
            M,
            (".armadai/agents/dev.md", b"---\ndescription: d\n---\nB\n"),
        ])
        .unwrap();
        let f = front(&plan, "agents/dev.md");
        assert!(!f.contains_key("tools") && !f.contains_key("disallowedTools"));
    }

    #[test]
    fn tier_maps_to_the_claude_alias_and_native_model_wins() {
        let plan = compile(&[
            M,
            (
                ".armadai/agents/a.md",
                b"---\ndescription: d\ntier: fast\n---\nB\n",
            ),
            (
                ".armadai/agents/b.md",
                b"---\ndescription: d\ntier: max\nnative:\n  claude:\n    model: claude-opus-5-5\n    color: blue\n---\nB\n",
            ),
            (".armadai/agents/c.md", b"---\ndescription: d\n---\nB\n"),
        ])
        .unwrap();
        let model = |p: &str| {
            front(&plan, p)
                .get("model")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        assert_eq!(model("agents/a.md").as_deref(), Some("haiku"));
        assert_eq!(model("agents/b.md").as_deref(), Some("claude-opus-5-5"));
        assert_eq!(model("agents/c.md"), None, "no tier: inherit");
        assert_eq!(
            front(&plan, "agents/b.md")
                .get("color")
                .and_then(|v| v.as_str()),
            Some("blue")
        );
    }

    #[test]
    fn a_native_key_colliding_with_a_generated_one_is_an_error() {
        let err = compile(&[
            M,
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: d\nnative:\n  claude:\n    description: other\n---\nB\n",
            ),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("`description`"), "{err}");
    }

    #[test]
    fn description_with_colon_and_quotes_round_trips() {
        let desc = "Reviews: \"risky\" code, it's #1";
        let plan = compile(&[
            M,
            (
                ".armadai/agents/dev.md",
                format!("---\ndescription: '{}'\n---\nB\n", desc.replace('\'', "''")).as_bytes(),
            ),
        ])
        .unwrap();
        assert_eq!(
            front(&plan, "agents/dev.md")
                .get("description")
                .and_then(|v| v.as_str()),
            Some(desc)
        );
    }

    #[test]
    fn agent_skills_and_informational_fields_are_reported() {
        let plan = compile(&[
            M,
            (
                ".armadai/skills/lint/SKILL.md",
                b"---\nname: lint\ndescription: Lint\n---\nBody\n",
            ),
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: d\ntitle: Dev\ntemperature: 0.2\ntags: [x]\nskills: [lint]\n---\nB\n",
            ),
        ])
        .unwrap();
        let skills = front(&plan, "agents/dev.md")
            .get("skills")
            .cloned()
            .unwrap();
        assert_eq!(
            skills,
            serde_yaml_ng::from_str::<serde_yaml_ng::Value>("[lint]").unwrap()
        );
        assert_eq!(line(&plan, "agent dev skills").status, Status::Compiled);
        for e in ["agent dev temperature", "agent dev tags", "agent dev title"] {
            assert_eq!(line(&plan, e).status, Status::NotCompiled, "{e}");
        }
    }

    #[test]
    fn a_skill_folder_is_copied_byte_for_byte() {
        let bin: &[u8] = &[0, 159, 146, 150, 255];
        let plan = compile(&[
            M,
            (
                ".armadai/skills/lint/SKILL.md",
                b"---\nname: lint\ndescription: Lint\n---\nBody\n",
            ),
            (".armadai/skills/lint/scripts/run.bin", bin),
        ])
        .unwrap();
        assert_eq!(artifact(&plan, "skills/lint/scripts/run.bin").content, bin);
        let skill = artifact(&plan, "skills/lint/SKILL.md");
        assert_eq!(
            skill.content,
            b"---\nname: lint\ndescription: Lint\n---\nBody\n"
        );
        assert_eq!(skill.kind, ArtifactKind::Skill);
        assert_eq!(line(&plan, "skill lint").status, Status::Compiled);
        let paths: Vec<&PathBuf> = plan.artifacts.iter().map(|a| &a.path).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "artifacts in sorted order");
    }

    #[cfg(unix)]
    #[test]
    fn a_skill_file_escaping_its_folder_is_refused() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "x").unwrap();
        let (dir, p) = project(&[
            M,
            (
                ".armadai/skills/lint/SKILL.md",
                b"---\nname: lint\ndescription: Lint\n---\nBody\n",
            ),
        ]);
        std::os::unix::fs::symlink(
            outside.path().join("secret"),
            dir.path().join(".armadai/skills/lint/leak"),
        )
        .unwrap();
        let err = ClaudeCompiler.compile(&p, dir.path()).unwrap_err();
        assert!(err.to_string().contains("leak"), "{err}");
    }

    const FLOW: (&str, &[u8]) = (
        ".armadai/flows/ship.yaml",
        b"description: Ship a change\nsteps:\n  plan: { agent: lead }\n  build: { agent: dev, on_failure: plan }\n  review: { agent: rev, states: [changes] }\nlinks:\n  - { from: plan, to: build }\n  - { from: build, to: review }\n  - { from: review, to: build, when: changes }\n",
    );

    fn team() -> Vec<(&'static str, &'static [u8])> {
        vec![
            (
                ".armadai/project.yaml",
                b"schema_version: 1\ncoordinator: lead\n",
            ),
            (
                ".armadai/agents/lead.md",
                b"---\ndescription: Leads\ncapabilities:\n  delegate: deny\n---\nYou lead the team.\n",
            ),
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: Writes code\n---\nB\n",
            ),
            (
                ".armadai/agents/rev.md",
                b"---\ndescription: Reviews code\n---\nB\n",
            ),
            FLOW,
        ]
    }

    #[test]
    fn the_coordinator_becomes_a_claude_md_block_with_the_delegation_guidance() {
        let plan = compile(&team()).unwrap();
        assert!(
            !plan
                .artifacts
                .iter()
                .any(|a| a.path == Path::new("agents/lead.md")),
            "the coordinator lives in CLAUDE.md"
        );
        assert_eq!(plan.blocks.len(), 1);
        let (path, block) = &plan.blocks[0];
        assert_eq!(path, Path::new("CLAUDE.md"));
        assert!(block.starts_with("You lead the team.\n"), "{block}");
        for needle in [
            "## Delegation",
            "`dev`: Writes code",
            "`rev`: Reviews code",
            "### Flow `ship`",
            "Ship a change",
            "`build` (agent `dev`, on failure: `plan`)",
            "`review` (agent `rev`, states: changes)",
            "`review` -> `build` when `changes`",
            "`plan` -> `build`\n",
        ] {
            assert!(block.contains(needle), "missing {needle:?} in\n{block}");
        }
        assert!(
            !block.contains("`lead`: Leads"),
            "the coordinator is not its own delegate"
        );
        assert_eq!(line(&plan, "flow ship").status, Status::Approximated);
        assert_eq!(
            line(&plan, "flow ship step build").status,
            Status::Approximated
        );
        assert_eq!(
            line(&plan, "agent lead capabilities").status,
            Status::NotCompiled
        );
    }

    #[test]
    fn without_a_coordinator_flows_are_not_compiled() {
        let mut files = team();
        files[0] = M;
        let plan = compile(&files).unwrap();
        assert!(plan.blocks.is_empty());
        assert_eq!(line(&plan, "flow ship").status, Status::NotCompiled);
    }

    #[test]
    fn an_existing_claude_md_without_markers_is_reported() {
        let mut files = team();
        files.push(("CLAUDE.md", b"# Mine\n"));
        let plan = compile(&files).unwrap();
        assert_eq!(line(&plan, "CLAUDE.md").status, Status::Approximated);
        let plan = compile(&team()).unwrap();
        assert!(!plan.report.lines.iter().any(|l| l.element == "CLAUDE.md"));
    }

    #[test]
    fn strict_policy_and_automations_are_reported() {
        let mut files = team();
        files[0] = (
            ".armadai/project.yaml",
            b"schema_version: 1\ncoordinator: lead\npolicy:\n  delegation: strict\nnative:\n  claude:\n    model: opus\n",
        );
        files.push((
            ".armadai/automations/fmt.yaml",
            b"event: after-tool\nrun: cargo fmt\n",
        ));
        let plan = compile(&files).unwrap();
        let gate = line(&plan, "policy delegation=strict");
        assert_eq!(gate.status, Status::Compiled);
        assert!(
            gate.reason
                .as_deref()
                .unwrap()
                .contains("enforced: delegation topology")
        );
        let auto = line(&plan, "automation fmt");
        assert_eq!(auto.status, Status::Compiled);
        assert!(auto.reason.as_deref().unwrap().contains("PostToolUse"));
        assert_eq!(line(&plan, "project native").status, Status::Compiled);
        assert!(
            !compile(&team())
                .unwrap()
                .report
                .lines
                .iter()
                .any(|l| l.element.starts_with("policy"))
        );
    }

    #[test]
    fn a_hook_native_collision_fails_the_compilation() {
        let mut files = team();
        files.push((
            ".armadai/automations/x.yaml",
            b"event: on-stop\nrun: x\nnative:\n  claude:\n    type: http\n",
        ));
        assert!(compile(&files).unwrap_err().to_string().contains("`type`"));
    }

    #[test]
    fn no_artifact_is_empty() {
        let plan = compile(&team()).unwrap();
        assert!(!plan.artifacts.is_empty());
        assert!(plan.artifacts.iter().all(|a| !a.content.is_empty()));
    }
}
