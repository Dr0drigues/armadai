//! Claude Code adapter: subagents, skills and the root `CLAUDE.md` block.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use armadai_model::{Agent, AgentFront, Cap, Delegation, Flow, NativeValue, Project, Tier};
use serde_json::{Map, Value};

use super::{
    Artifact, ArtifactKind, BLOCK_BEGIN, BLOCK_END, Compiler, Plan, Report, Status, claude_settings,
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
        let strict = project.manifest.policy.delegation == Delegation::Strict;

        informational(&mut plan.report, project);
        for (id, agent) in &project.agents {
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
            let (files, backups) = super::read_tree_skipping(&dir, &root.join(".armadai"))?;
            for rel in backups {
                plan.report.push(
                    T,
                    format!("skill {id} {}", rel.display()),
                    Status::NotCompiled,
                    Some("stray backup ignored"),
                );
            }
            for (rel, content) in files {
                plan.artifacts.push(Artifact {
                    path: Path::new("skills").join(id).join(rel),
                    content,
                    kind: ArtifactKind::Skill,
                    produced_by: ProducedBy::skill(id),
                });
            }
            plan.report
                .push(T, format!("skill {id}"), Status::Compiled, None);
            for key in project.skills[id].front.extra.keys() {
                plan.report.push(
                    T,
                    format!("skill {id} key {key}"),
                    Status::Compiled,
                    Some("kept in SKILL.md as is, for Claude Code to read"),
                );
            }
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
            if a.description.is_some() {
                plan.report.push(
                    T,
                    format!("automation {id} description"),
                    Status::NotCompiled,
                    Some("Claude Code hooks have no description field"),
                );
            }
        }
        if !claude_settings::native_settings(project)?.is_empty() {
            plan.report.push(
                T,
                "project native",
                Status::Compiled,
                Some("merged into .claude/settings.json"),
            );
        }
        if strict {
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
                if [BLOCK_BEGIN, BLOCK_END]
                    .iter()
                    .any(|m| lead.body.contains(m))
                {
                    bail!(
                        "the prompt of coordinator `{c}` contains `{BLOCK_BEGIN}` or \
                         `{BLOCK_END}`, which delimit ArmadAI's block in CLAUDE.md"
                    );
                }
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
                        Some(if strict {
                            "rendered as guidance in CLAUDE.md; delegation topology enforced by the gate"
                        } else {
                            "rendered as guidance in CLAUDE.md, not enforced"
                        }),
                    );
                    if let Some(start) = &flow.start {
                        plan.report.push(
                            T,
                            format!("flow {fid} start"),
                            Status::Approximated,
                            Some(&format!(
                                "Start: `{start}`, rendered as guidance in CLAUDE.md"
                            )),
                        );
                    }
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
                    let (status, reason) = if strict {
                        (
                            Status::Approximated,
                            "delegation topology enforced by the gate; no coordinator, so no CLAUDE.md guidance",
                        )
                    } else {
                        (
                            Status::NotCompiled,
                            "no coordinator: no CLAUDE.md to render the flow into",
                        )
                    };
                    plan.report
                        .push(T, format!("flow {fid}"), status, Some(reason));
                    if let (true, Some(start)) = (strict, &project.flows[fid].start) {
                        plan.report.push(
                            T,
                            format!("flow {fid} start"),
                            Status::Compiled,
                            Some(&format!("the gate starts the main session at `{start}`")),
                        );
                    }
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

/// Lines for fields Claude Code subagents have no place for: one per field, naming
/// every agent that sets it (one line per agent was noise in every report).
fn informational(r: &mut Report, project: &Project) {
    type IsSet = fn(&AgentFront) -> bool;
    let fields: [(&str, &str, IsSet); 3] = [
        (
            "temperature",
            "no equivalent in Claude Code subagents",
            |f| f.temperature.is_some(),
        ),
        ("tags", "informational, no Claude Code field", |f| {
            !f.tags.is_empty()
        }),
        ("title", "Claude Code names a subagent by its id", |f| {
            f.title.is_some()
        }),
    ];
    for (field, why, set) in fields {
        let ids: Vec<&str> = (project.agents.iter())
            .filter(|(_, a)| set(&a.front))
            .map(|(id, _)| id.as_str())
            .collect();
        if ids.is_empty() {
            continue;
        }
        let n = ids.len();
        let reason = format!(
            "{why}; {n} agent{}: {}",
            if n == 1 { "" } else { "s" },
            ids.join(", ")
        );
        r.push(
            T,
            format!("agents {field}"),
            Status::NotCompiled,
            Some(&reason),
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
    if !f.native.is_empty() {
        fields.push("native");
    }
    if !f.skills.is_empty() {
        r.push(
            T,
            format!("agent {id} skills"),
            Status::Approximated,
            Some(
                "skills are discovered project-wide from .claude/skills; the main session \
                 cannot preload them",
            ),
        );
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
    let mut m = Map::new();
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
            // also for `delegate`: subagents may spawn subagents (up to three levels,
            // CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH), sub-agents doc checked 2026-10-12
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
        m.insert("skills".into(), Value::Array(list));
        r.push(
            T,
            format!("agent {id} skills"),
            Status::Compiled,
            Some("preloaded through the subagent `skills` field"),
        );
    }
    if let Some(native) = f.native.get(T) {
        let Some(native) = native.as_object() else {
            bail!("agent `{id}`: native.claude must be a mapping");
        };
        for (k, v) in native {
            // `model` is the documented override of the tier; any other generated key wins nothing
            if k != "model" && m.contains_key(k) {
                bail!("agent `{id}`: native.claude key `{k}` collides with a generated key");
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
    let yaml = armadai_model::yaml::to_string(&NativeValue(Value::Object(m)))
        .context("subagent frontmatter")?;
    let mut out = format!("---\n{yaml}---\n{}", agent.body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

pub(super) fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Coordinator prompt + `## Delegation`, generated from the agents and the flows.
fn claude_md(coordinator: &str, lead: &Agent, project: &Project) -> String {
    let mut out = lead.body.trim_end().to_string();
    out.push_str("\n\n## Delegation\n\nDelegate with the Agent tool, naming the subagent by its id.\n\n### Agents\n\n");
    // under a strict policy, say which agents the gate will not let the main
    // session reach rather than list them as if it would
    let strict = project.manifest.policy.delegation == Delegation::Strict;
    let allowed = crate::claude_adapter::policy_gate::allowed_targets(project, None);
    for (id, a) in project.agents.iter().filter(|(id, _)| *id != coordinator) {
        let note = if strict && !allowed.contains(id) {
            " (not delegable under `policy.delegation: strict`: in no flow and not a free agent)"
        } else {
            ""
        };
        out.push_str(&format!(
            "- `{id}`: {}{note}\n",
            one_line(&a.front.description)
        ));
    }
    for (fid, flow) in &project.flows {
        out.push_str(&flow_section(fid, flow));
    }
    out
}

pub(super) fn flow_section(fid: &str, flow: &Flow) -> String {
    let mut out = format!("\n### Flow `{fid}`\n\n");
    if let Some(d) = &flow.description {
        out.push_str(&format!("{}\n\n", one_line(d)));
    }
    if let Some(start) = &flow.start {
        out.push_str(&format!("Start: `{start}`\n\n"));
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

    fn front(plan: &Plan, path: &str) -> serde_json::Map<String, serde_json::Value> {
        let t = text(plan, path);
        let (yaml, _, _) = armadai_model::split_frontmatter(&t).unwrap();
        armadai_model::yaml::from_str(yaml).unwrap()
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
        assert_eq!(skills, serde_json::json!(["lint"]));
        assert_eq!(line(&plan, "agent dev skills").status, Status::Compiled);
        for e in ["agents temperature", "agents tags", "agents title"] {
            assert_eq!(line(&plan, e).status, Status::NotCompiled, "{e}");
        }
    }

    /// One line per informational field, for every agent that sets it.
    #[test]
    fn informational_fields_are_one_line_each() {
        let plan = compile(&[
            M,
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: d\ntemperature: 0.2\ntags: [x]\n---\nB\n",
            ),
            (
                ".armadai/agents/qa.md",
                b"---\ndescription: d\ntemperature: 0.3\n---\nB\n",
            ),
        ])
        .unwrap();
        let t = line(&plan, "agents temperature");
        assert_eq!(
            t.reason.as_deref(),
            Some("no equivalent in Claude Code subagents; 2 agents: dev, qa")
        );
        assert_eq!(
            line(&plan, "agents tags").reason.as_deref(),
            Some("informational, no Claude Code field; 1 agent: dev")
        );
        assert_eq!(
            plan.report
                .lines
                .iter()
                .filter(|l| l.element.ends_with("temperature"))
                .count(),
            1
        );
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

    /// The main session cannot preload skills, but Claude Code discovers the project's
    /// skills (`.claude/skills/`) by their description: approximated, not lost.
    #[test]
    fn coordinator_skills_are_approximated() {
        let mut files = team();
        files[1] = (
            ".armadai/agents/lead.md",
            b"---\ndescription: Leads\nskills: [lint]\n---\nYou lead the team.\n",
        );
        files.push((
            ".armadai/skills/lint/SKILL.md",
            b"---\nname: lint\ndescription: Lint\n---\nL\n",
        ));
        let plan = compile(&files).unwrap();
        let l = line(&plan, "agent lead skills");
        assert_eq!(l.status, Status::Approximated);
        assert_eq!(
            l.reason.as_deref(),
            Some(
                "skills are discovered project-wide from .claude/skills; the main session \
                 cannot preload them"
            )
        );
    }

    /// A backup left by an interrupted `new --force` is never compiled, and said so.
    #[test]
    fn a_stray_backup_in_a_skill_is_ignored_and_reported() {
        let plan = compile(&[
            M,
            (
                ".armadai/skills/lint/SKILL.md",
                b"---\nname: lint\ndescription: Lint\n---\nBody\n",
            ),
            (".armadai/skills/lint/SKILL.md.armadai-backup", b"old\n"),
        ])
        .unwrap();
        assert!(
            !plan
                .artifacts
                .iter()
                .any(|a| a.path.to_string_lossy().contains("armadai-backup"))
        );
        let l = line(&plan, "skill lint SKILL.md.armadai-backup");
        assert_eq!(l.status, Status::NotCompiled);
        assert_eq!(l.reason.as_deref(), Some("stray backup ignored"));
    }

    /// A declared `start` is said in CLAUDE.md and in the report.
    #[test]
    fn a_declared_start_is_rendered_and_reported() {
        let mut files = team();
        files.retain(|(p, _)| *p != FLOW.0);
        files.push((
            ".armadai/flows/ship.yaml",
            b"start: plan\nsteps:\n  plan: { agent: lead }\n  build: { agent: dev }\nlinks:\n  - { from: plan, to: build }\n  - { from: build, to: plan, when: failed }\n",
        ));
        let plan = compile(&files).unwrap();
        assert!(
            plan.blocks[0].1.contains("Start: `plan`\n"),
            "{}",
            plan.blocks[0].1
        );
        let l = line(&plan, "flow ship start");
        assert_eq!(l.status, Status::Approximated);
        assert!(l.reason.as_deref().unwrap().contains("`plan`"), "{l:?}");
        // without a coordinator, under strict, the gate applies it
        files[0] = (
            ".armadai/project.yaml",
            b"schema_version: 1\npolicy:\n  delegation: strict\n",
        );
        let plan = compile(&files).unwrap();
        assert_eq!(line(&plan, "flow ship start").status, Status::Compiled);
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
    fn under_a_strict_policy_flows_are_reported_as_enforced_topology() {
        let mut files = team();
        files[0] = (
            ".armadai/project.yaml",
            b"schema_version: 1\npolicy:\n  delegation: strict\n",
        );
        let plan = compile(&files).unwrap();
        let l = line(&plan, "flow ship");
        assert_eq!(
            l.status,
            Status::Approximated,
            "no coordinator, but the gate applies it"
        );
        assert!(
            l.reason
                .as_deref()
                .unwrap()
                .contains("enforced by the gate"),
            "{l:?}"
        );
        files[0] = (
            ".armadai/project.yaml",
            b"schema_version: 1\ncoordinator: lead\npolicy:\n  delegation: strict\n",
        );
        let plan = compile(&files).unwrap();
        let reason = line(&plan, "flow ship").reason.clone().unwrap();
        assert!(
            reason.contains("guidance") && reason.contains("enforced by the gate"),
            "{reason}"
        );
    }

    #[test]
    fn a_coordinator_prompt_holding_the_end_marker_is_an_error() {
        let mut files = team();
        files[1] = (
            ".armadai/agents/lead.md",
            b"---\ndescription: Leads\n---\nYou lead.\n<!-- armadai:end -->\n",
        );
        let err = compile(&files).unwrap_err().to_string();
        assert!(err.contains("lead") && err.contains("armadai:end"), "{err}");
    }

    #[test]
    fn skill_extra_keys_and_automation_descriptions_are_reported() {
        let plan = compile(&[
            M,
            (
                ".armadai/skills/lint/SKILL.md",
                b"---\nname: lint\ndescription: Lint\nallowed-tools: Bash\n---\nBody\n",
            ),
            (
                ".armadai/automations/fmt.yaml",
                b"event: on-stop\nrun: cargo fmt\ndescription: format\n",
            ),
        ])
        .unwrap();
        assert_eq!(
            line(&plan, "skill lint key allowed-tools").status,
            Status::Compiled
        );
        assert_eq!(
            line(&plan, "automation fmt description").status,
            Status::NotCompiled
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_skill_symlink_to_a_directory_or_a_fifo_is_refused() {
        for kind in ["dir", "fifo"] {
            let (dir, p) = project(&[
                M,
                (
                    ".armadai/skills/lint/SKILL.md",
                    b"---\nname: lint\ndescription: Lint\n---\nBody\n",
                ),
            ]);
            let skill = dir.path().join(".armadai/skills/lint");
            if kind == "dir" {
                std::fs::create_dir_all(skill.join("sub")).unwrap();
                std::os::unix::fs::symlink(skill.join("sub"), skill.join("sub/loop")).unwrap();
            } else {
                let status = std::process::Command::new("mkfifo")
                    .arg(skill.join("pipe"))
                    .status()
                    .unwrap();
                assert!(status.success());
            }
            let err = ClaudeCompiler
                .compile(&p, dir.path())
                .unwrap_err()
                .to_string();
            assert!(
                err.contains(if kind == "dir" { "loop" } else { "pipe" }),
                "{kind}: {err}"
            );
        }
    }

    #[test]
    fn under_strict_an_agent_the_gate_refuses_is_marked_in_claude_md() {
        let mut files = team();
        files[0] = (
            ".armadai/project.yaml",
            b"schema_version: 1\ncoordinator: lead\npolicy:\n  delegation: strict\n",
        );
        files.push((
            ".armadai/agents/doc.md",
            b"---\ndescription: Docs\n---\nB\n",
        ));
        let plan = compile(&files).unwrap();
        let block = &plan.blocks[0].1;
        assert!(block.contains("`doc`: Docs (not delegable"), "{block}");
        assert!(
            !block.contains("`dev`: Writes code (not delegable"),
            "{block}"
        );
    }

    #[test]
    fn no_artifact_is_empty() {
        let plan = compile(&team()).unwrap();
        assert!(!plan.artifacts.is_empty());
        assert!(plan.artifacts.iter().all(|a| !a.content.is_empty()));
    }
}
