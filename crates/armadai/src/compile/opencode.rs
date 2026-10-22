//! OpenCode 2.x adapter: agents (`.opencode/agents/<id>.md`), commands, skills, models and
//! capabilities as per-agent permissions.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use armadai_core::config::{UserConfig, model_for_tier};
use armadai_model::{AgentFront, Cap, Command, Delegation, NativeValue, Project, agent_skills};
use serde_json::{Map, Value};

use super::json_keys::JsonKeys;
use super::{Artifact, ArtifactKind, Compiler, Plan, Report, Status};
use crate::claude_adapter::policy_gate::allowed_targets;
use crate::linker::manifest::ProducedBy;

const T: &str = "opencode";

/// Agents OpenCode ships (the last three hidden system agents): a project agent of
/// the same name would replace it.
pub const BUILTIN_AGENTS: [&str; 8] = [
    "build",
    "plan",
    "general",
    "explore",
    "scout",
    "compaction",
    "title",
    "summary",
];

/// Built-in agents a delegation can reach (the others are primary).
const BUILTIN_SUBAGENTS: [&str; 3] = ["general", "explore", "scout"];

/// Upper bound of `experimental.subagent_depth`.
const MAX_DEPTH: usize = 8;

/// Capability → the OpenCode 2.x permission keys it governs (v1 agent format, which
/// 2.x migrates: `task` becomes the `subagent` action).
const CAP_KEYS: [(&str, &[&str]); 5] = [
    ("read", &["read", "grep", "glob"]),
    ("write", &["edit"]),
    ("shell", &["shell"]),
    ("web", &["webfetch", "websearch"]),
    ("delegate", &["task"]),
];

pub struct OpencodeCompiler {
    /// The team file as `opencode.json` and the coordinator name it: in the output
    /// directory of this link, relative to the project root.
    team: String,
}

impl OpencodeCompiler {
    pub fn new(output: &Path) -> Self {
        let team = output.join("armadai-team.md");
        Self {
            team: team.to_string_lossy().replace('\\', "/"),
        }
    }
}

impl Compiler for OpencodeCompiler {
    fn compile(&self, project: &Project, root: &Path) -> anyhow::Result<Plan> {
        compile_with(
            project,
            root,
            &armadai_core::config::try_load_user_config()?,
            &self.team,
        )
    }
}

fn compile_with(
    project: &Project,
    root: &Path,
    config: &UserConfig,
    team: &str,
) -> anyhow::Result<Plan> {
    let mut plan = Plan::default();
    let coordinator = project.manifest.coordinator.as_deref();
    let strict = project.manifest.policy.delegation == Delegation::Strict;
    let has_flows = !project.flows.is_empty();
    informational(&mut plan.report, project);
    for id in project.agents.keys() {
        if BUILTIN_AGENTS.contains(&id.as_str()) {
            bail!(
                "agent `{id}` has the name of an OpenCode built-in agent; rename it \
                 (built-in: {})",
                BUILTIN_AGENTS.join(", ")
            );
        }
        let is_lead = Some(id.as_str()) == coordinator;
        let targets = strict.then(|| allowed_targets(project, Some(id)));
        plan.artifacts.push(Artifact {
            path: PathBuf::from(format!("agents/{id}.md")),
            content: agent_file(
                project,
                id,
                is_lead.then_some(team),
                has_flows,
                targets,
                config,
                &mut plan.report,
            )?
            .into_bytes(),
            kind: ArtifactKind::Agent,
            produced_by: ProducedBy::agent(id),
        });
        let note = is_lead.then_some("coordinator, primary agent, default_agent in opencode.json");
        plan.report
            .push(T, format!("agent {id}"), Status::Compiled, note);
    }

    let mut keys = JsonKeys::default();
    if let Some(c) = coordinator {
        keys.values.insert("default_agent".into(), c.into());
    }
    // without a coordinator nor a flow, OpenCode's own agent list says it all
    if coordinator.is_some() || !project.flows.is_empty() {
        plan.artifacts.push(Artifact {
            path: PathBuf::from("armadai-team.md"),
            content: team_file(project, strict).into_bytes(),
            kind: ArtifactKind::Instructions,
            produced_by: ProducedBy::coordinator(coordinator.unwrap_or(T)),
        });
        keys.array_entries
            .insert("instructions".into(), vec![team.into()]);
        let note = format!("guidance in {team}, listed in opencode.json instructions");
        plan.report
            .push(T, "team file", Status::Compiled, Some(&note));
    }
    if strict {
        // switching to `build` or `plan` (Tab) must not get around the topology
        let main = task_rule(&allowed_targets(project, None), "allow");
        for a in ["build", "plan"] {
            keys.values
                .insert(format!("agent.{a}.permission.task"), main.clone());
        }
        let depth = longest_chain(project, None, &mut Vec::new()).clamp(1, MAX_DEPTH);
        keys.values
            .insert("experimental.subagent_depth".into(), depth.into());
        plan.report.push(
            T,
            "agents build, plan",
            Status::Compiled,
            Some("permission.task set to the main-thread rule in opencode.json"),
        );
        let unknown: Vec<&str> = (project.manifest.policy.free_agents.iter())
            .map(String::as_str)
            .filter(|a| !BUILTIN_SUBAGENTS.contains(a) && !project.agents.contains_key(*a))
            .collect();
        if !unknown.is_empty() {
            let why = format!(
                "unknown to OpenCode (not an agent of .armadai/ nor a built-in subagent): \
                 {}; written as is",
                unknown.join(", ")
            );
            plan.report
                .push(T, "policy free_agents", Status::Approximated, Some(&why));
        }
    }
    if let Some(native) = project.manifest.native.get(T) {
        let Some(native) = native.as_object() else {
            bail!("native.opencode must be a mapping");
        };
        for k in native.keys() {
            if k.contains('.') {
                bail!(
                    "native.opencode key `{k}` contains a `.`: armadai cannot own it in opencode.json"
                );
            }
            let owned = (keys.values.keys())
                .chain(keys.array_entries.keys())
                .any(|p| p.split('.').next() == Some(k));
            if owned {
                bail!(
                    "native.opencode key `{k}` collides with a key armadai writes in opencode.json"
                );
            }
        }
        keys.values.extend(native.clone());
        plan.report.push(
            T,
            "native",
            Status::Compiled,
            Some("merged into opencode.json"),
        );
    }
    if !keys.values.is_empty() || !keys.array_entries.is_empty() {
        plan.json = Some((PathBuf::from("opencode.json"), keys));
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
    }
    for (id, c) in &project.commands {
        plan.artifacts.push(Artifact {
            path: PathBuf::from(format!("commands/{id}.md")),
            content: command_file(id, c, &mut plan.report)?.into_bytes(),
            kind: ArtifactKind::Command,
            produced_by: ProducedBy::command(id),
        });
        let (status, why) = match c.front.argument_hint {
            Some(_) => (
                Status::Approximated,
                Some("argument_hint has no OpenCode equivalent"),
            ),
            None => (Status::Compiled, None),
        };
        plan.report.push(T, format!("command {id}"), status, why);
    }
    plan.artifacts.sort_by(|a, b| a.path.cmp(&b.path));

    for id in project.automations.keys() {
        plan.report.push(
            T,
            format!("automation {id}"),
            Status::NotCompiled,
            Some("needs an OpenCode plugin"),
        );
    }
    for id in project.flows.keys() {
        let (status, why) = if strict {
            (
                Status::Compiled,
                "transitions enforced by OpenCode permissions, the rest as guidance",
            )
        } else {
            (Status::Approximated, "guidance only")
        };
        plan.report.push(T, format!("flow {id}"), status, Some(why));
    }
    if !project.flows.is_empty() {
        plan.report.push(
            T,
            "flow recording",
            Status::NotCompiled,
            Some(super::report::RECORDING_CLAUDE_ONLY),
        );
    }
    if strict {
        plan.report.push(
            T,
            "policy delegation=strict",
            Status::Compiled,
            Some(
                "enforced by OpenCode permissions; agents of your own outside .armadai/ not covered",
            ),
        );
    }
    Ok(plan)
}

/// `permission.task`: the catch-all deny first (OpenCode applies the last matching
/// rule), then `effect` for each target.
fn task_rule(targets: &BTreeSet<String>, effect: &str) -> Value {
    let mut m = Map::from_iter([("*".to_string(), Value::from("deny"))]);
    for t in targets {
        m.insert(t.clone(), effect.into());
    }
    Value::Object(m)
}

// shortcut: exponential in the worst case (longest simple path), cut at MAX_DEPTH;
// memoize per agent if a project ever has dozens of interlinked agents.
/// Delegations in the longest chain from `caller` (`None` = the main thread), each
/// agent at most once (cycles left out); the search stops at [`MAX_DEPTH`].
fn longest_chain(project: &Project, caller: Option<&str>, seen: &mut Vec<String>) -> usize {
    let mut best = 0;
    for t in allowed_targets(project, caller) {
        if best == MAX_DEPTH - seen.len() {
            break;
        }
        if seen.contains(&t) {
            continue;
        }
        seen.push(t.clone());
        best = best.max(1 + longest_chain(project, Some(&t), seen));
        seen.pop();
    }
    best
}

/// The team and its flows, described neutrally: every agent reads this file, so no
/// sentence gives a role (the coordinator's is in its own agent file).
fn team_file(project: &Project, strict: bool) -> String {
    let lead = project.manifest.coordinator.as_deref();
    let mut out = String::from(
        "# Team\n\nGenerated by `armadai link` from `.armadai/`: edit the sources, not this file.\n\n## Agents\n\n",
    );
    // ids only: a description may be written as a role, and OpenCode already shows
    // each one to the model through the subagent tool
    for id in project.agents.keys() {
        let role = if Some(id.as_str()) == lead {
            " (coordinator, the default primary agent)"
        } else {
            ""
        };
        out.push_str(&format!("- `{id}`{role}\n"));
    }
    let free = &project.manifest.policy.free_agents;
    if strict && !free.is_empty() {
        let names: Vec<String> = free.iter().map(|f| format!("`{f}`")).collect();
        out.push_str(&format!(
            "\nFree agents, delegable from any agent: {}.\n",
            names.join(", ")
        ));
    }
    if !project.flows.is_empty() {
        out.push_str("\n## Flows\n");
        if strict {
            out.push_str(
                "\nDelegation follows the transitions below, enforced by OpenCode permissions \
                 (`policy.delegation: strict`).\n",
            );
        }
        for (fid, flow) in &project.flows {
            out.push_str(&super::claude::flow_section(fid, flow));
        }
    }
    out
}

/// Fields OpenCode agents have no place for: one line per field, naming the agents.
fn informational(r: &mut Report, project: &Project) {
    type IsSet = fn(&AgentFront) -> bool;
    let fields: [(&str, &str, IsSet); 2] = [
        ("tags", "informational, no OpenCode field", |f| {
            !f.tags.is_empty()
        }),
        ("title", "OpenCode names an agent by its file name", |f| {
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

fn caps(front: &AgentFront) -> [Option<Cap>; 5] {
    let c = &front.capabilities;
    // same order as CAP_KEYS
    [c.read, c.write, c.shell, c.web, c.delegate]
}

fn cap_str(c: Cap) -> &'static str {
    match c {
        Cap::Allow => "allow",
        Cap::Ask => "ask",
        Cap::Deny => "deny",
    }
}

/// `native.opencode.model`, else the user's `model_tiers.opencode.<tier>`, with the
/// report line saying which; `None` when nothing (valid) is to be written.
fn model(id: &str, f: &AgentFront, config: &UserConfig, r: &mut Report) -> Option<String> {
    let element = format!("agent {id} model");
    let native = match (f.native.get(T)).and_then(|n| n.get("model")) {
        Some(Value::String(m)) => Some(m.clone()),
        Some(_) => {
            // the v2 object form, say: no silent fallback to the tier
            let why = "native.opencode.model is not a provider/model string, ignored";
            r.push(T, element, Status::Approximated, Some(why));
            return None;
        }
        None => None,
    };
    let (m, from) = match (native, f.tier) {
        (Some(m), _) => (m, "from native.opencode.model"),
        (None, Some(t)) => match model_for_tier(config, T, t) {
            Some(m) => (m, "from model_tiers in your config"),
            None => {
                r.push(
                    T,
                    element,
                    Status::Approximated,
                    Some("inherited from the OpenCode session"),
                );
                return None;
            }
        },
        (None, None) => return None,
    };
    if !m.contains('/') {
        let why = format!("`{m}` is not provider/model, ignored");
        r.push(T, element, Status::Approximated, Some(&why));
        return None;
    }
    r.push(T, element, Status::Compiled, Some(from));
    Some(m)
}

fn agent_file(
    project: &Project,
    id: &str,
    // the team file, for the coordinator only
    lead: Option<&str>,
    has_flows: bool,
    targets: Option<BTreeSet<String>>,
    config: &UserConfig,
    r: &mut Report,
) -> anyhow::Result<String> {
    let agent = &project.agents[id];
    let f = &agent.front;
    let mut m = Map::new();
    m.insert("description".into(), f.description.as_str().into());
    let mode = if lead.is_some() {
        "primary"
    } else {
        "subagent"
    };
    m.insert("mode".into(), mode.into());
    if let Some(model) = model(id, f, config, r) {
        m.insert("model".into(), model.into());
    }
    if let Some(t) = f.temperature {
        m.insert("temperature".into(), serde_json::to_value(t)?);
    }
    let mut permission = Map::new();
    for (cap, (name, keys)) in caps(f).into_iter().zip(CAP_KEYS) {
        let Some(cap) = cap else { continue };
        let element = format!("agent {id} capability {name}={}", cap_str(cap));
        if cap == Cap::Allow {
            r.push(T, element, Status::Compiled, Some("OpenCode default"));
            continue;
        }
        for k in keys {
            permission.insert((*k).into(), cap_str(cap).into());
        }
        let why = format!("permission {}", keys.join(", "));
        r.push(T, element, Status::Compiled, Some(&why));
    }
    if let Some(targets) = targets {
        // a `delegate: deny` capability stays a plain deny, `ask` asks per target
        match permission.get("task").and_then(Value::as_str) {
            Some("deny") => {}
            effect => {
                let rule = task_rule(&targets, effect.unwrap_or("allow"));
                permission.insert("task".into(), rule);
            }
        }
    }
    let mut body = agent.body.clone();
    if let Some(team) = lead {
        let (what, how) = if has_flows {
            ("the team and the flows", "Delegate each step to its agent")
        } else {
            ("the team", "Delegate")
        };
        body = format!(
            "{}\n\n## Delegation\n\nYou coordinate {what} described in `{team}`. \
             {how} with the `subagent` tool, naming the agent by its id.\n",
            body.trim_end()
        );
    }
    // only declared `skills:` restrict: an agent with none (slots or not) keeps
    // every skill of the project, as under Claude where a slot preloads only
    let skills = agent_skills(project, id);
    if !skills.is_empty() {
        let why = if f.skills.is_empty() {
            "loading prescribed in the prompt; no `skills:`, so every skill stays allowed"
        } else {
            // the catch-all first: OpenCode applies the last matching rule
            let mut skill = Map::from_iter([("*".to_string(), Value::from("deny"))]);
            for s in &skills {
                skill.insert(s.clone(), "allow".into());
            }
            permission.insert("skill".into(), Value::Object(skill));
            "allowed through permission.skill, loading prescribed in the prompt"
        };
        let list: Vec<String> = skills.iter().map(|s| format!("`{s}`")).collect();
        body = format!(
            "{}\n\n## Skills\n\nBefore you start, load these skills with the `skill` tool: {}.\n",
            body.trim_end(),
            list.join(", ")
        );
        r.push(T, format!("agent {id} skills"), Status::Compiled, Some(why));
    }
    super::slot_lines(r, T, project, id, f, None);
    if let Some(v) = super::verdict_instruction(project, id) {
        body = format!("{}{v}", body.trim_end());
    }
    if !permission.is_empty() {
        m.insert("permission".into(), Value::Object(permission));
    }
    // `model` was resolved above, as the documented override of the tier
    if super::merge_native(&mut m, &f.native, T, &format!("agent `{id}`"), &["model"])? {
        r.push(
            T,
            format!("agent {id} native"),
            Status::Compiled,
            Some("merged into the agent frontmatter"),
        );
    }
    let yaml = armadai_model::yaml::to_string(&NativeValue(Value::Object(m)))
        .context("agent frontmatter")?;
    let mut out = format!("---\n{yaml}---\n{body}");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// `.opencode/commands/<id>.md`: `description` and the body as the template (OpenCode
/// `dev` at 681156c8, `packages/core/src/config/plugin/command.ts`); no argument hint.
fn command_file(id: &str, c: &Command, r: &mut Report) -> anyhow::Result<String> {
    let mut m = Map::new();
    m.insert("description".into(), c.front.description.as_str().into());
    if super::merge_native(&mut m, &c.front.native, T, &format!("command `{id}`"), &[])? {
        r.push(
            T,
            format!("command {id} native"),
            Status::Compiled,
            Some("merged into the command frontmatter"),
        );
    }
    let yaml = armadai_model::yaml::to_string(&NativeValue(Value::Object(m)))
        .context("command frontmatter")?;
    let mut out = format!("---\n{yaml}---\n{}", c.body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;
    use crate::compile::{Artifact, ArtifactKind, Status};

    const M: (&str, &[u8]) = (".armadai/project.yaml", b"schema_version: 1\n");

    /// The frontmatter keys OpenCode 2.x decodes for an agent (v1 agent schema);
    /// any other key goes to the provider as a model option.
    const KNOWN: [&str; 10] = [
        "description",
        "mode",
        "model",
        "temperature",
        "top_p",
        "steps",
        "disable",
        "hidden",
        "color",
        "permission",
    ];

    fn compile_cfg(files: &[(&str, &[u8])], config: &UserConfig) -> anyhow::Result<Plan> {
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let path = dir.path().join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        let p = armadai_model::load_checked(dir.path())
            .unwrap_or_else(|ds| panic!("{:?}", ds.iter().map(|d| d.render()).collect::<Vec<_>>()))
            .0;
        compile_with(&p, dir.path(), config, ".opencode/armadai-team.md")
    }

    fn compile(files: &[(&str, &[u8])]) -> anyhow::Result<Plan> {
        compile_cfg(files, &UserConfig::default())
    }

    fn tiers(max: &str) -> UserConfig {
        UserConfig {
            model_tiers: BTreeMap::from([(
                "opencode".to_string(),
                BTreeMap::from([(armadai_model::Tier::Max, max.to_string())]),
            )]),
            ..Default::default()
        }
    }

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

    fn line(plan: &Plan, element: &str) -> (Status, Option<String>) {
        let l = plan
            .report
            .lines
            .iter()
            .find(|l| l.element == element)
            .unwrap_or_else(|| panic!("no `{element}` in\n{}", plan.report.render_text()));
        (l.status, l.reason.clone())
    }

    #[test]
    fn a_project_with_a_flow_says_recording_is_claude_only() {
        let agents: [(&str, &[u8]); 1] =
            [(".armadai/agents/dev.md", b"---\ndescription: d\n---\nB\n")];
        let flow: (&str, &[u8]) = (
            ".armadai/flows/ship.yaml",
            b"steps:\n  build: { agent: dev }\n",
        );
        let plan = compile(&[M, agents[0], flow]).unwrap();
        assert_eq!(
            line(&plan, "flow recording"),
            (Status::NotCompiled, Some("Claude Code only for now".into()))
        );
        let plan = compile(&[M, agents[0]]).unwrap();
        assert!(
            plan.report
                .lines
                .iter()
                .all(|l| l.element != "flow recording")
        );
    }

    #[test]
    fn a_minimal_agent_is_a_subagent_file() {
        let plan = compile(&[
            M,
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: Writes code\ntemperature: 0.2\n---\nYou write code.\n",
            ),
        ])
        .unwrap();
        assert_eq!(
            text(&plan, "agents/dev.md"),
            "---\ndescription: Writes code\nmode: subagent\ntemperature: 0.2\n---\nYou write code.\n"
        );
        assert_eq!(line(&plan, "agent dev"), (Status::Compiled, None));
        assert_eq!(plan.json, None);
    }

    #[test]
    fn the_agent_holding_a_step_with_states_is_asked_for_a_state_line() {
        let plan = compile(&[
            M,
            (".armadai/agents/dev.md", b"---\ndescription: d\n---\nYou build.\n"),
            (".armadai/agents/rev.md", b"---\ndescription: d\n---\nYou review.\n"),
            (
                ".armadai/flows/deliver.yaml",
                b"steps:\n  build: { agent: dev }\n  guard: { agent: rev, states: [rejected, approved] }\nlinks:\n  - { from: build, to: guard }\n  - { from: guard, to: build, when: rejected }\n",
            ),
        ])
        .unwrap();
        assert_eq!(
            text(&plan, "agents/rev.md"),
            "---\ndescription: d\nmode: subagent\n---\nYou review.\n\n## Verdict\n\n\
             End your reply with exactly one line `STATE: <state>`, where <state> is one of: \
             approved, rejected.\n"
        );
        assert_eq!(
            text(&plan, "agents/dev.md"),
            "---\ndescription: d\nmode: subagent\n---\nYou build.\n"
        );
    }

    #[test]
    fn agent_frontmatter_has_only_known_keys() {
        let plan = compile(&[
            M,
            (
                ".armadai/skills/lint/SKILL.md",
                b"---\nname: lint\ndescription: Lint\n---\nL\n",
            ),
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: d\ntitle: Dev\ntier: max\ntags: [x]\ntemperature: 0.3\nskills: [lint]\ncapabilities:\n  shell: deny\n  read: ask\n---\nB\n",
            ),
            (
                ".armadai/agents/qa.md",
                b"---\ndescription: d\nnative:\n  opencode:\n    color: '#ff0000'\n    reasoningEffort: high\n---\nB\n",
            ),
        ])
        .unwrap();
        for (id, native) in [("dev", vec![]), ("qa", vec!["color", "reasoningEffort"])] {
            let f = front(&plan, &format!("agents/{id}.md"));
            for k in f.keys() {
                assert!(
                    KNOWN.contains(&k.as_str()) || native.contains(&k.as_str()),
                    "agent {id}: unknown key `{k}` in {f:?}"
                );
            }
        }
        let dev = front(&plan, "agents/dev.md");
        let keys: Vec<&str> = dev.keys().map(String::as_str).collect();
        assert_eq!(keys, ["description", "mode", "temperature", "permission"]);
        assert_eq!(
            line(&plan, "agents title").0,
            Status::NotCompiled,
            "title is said, not written"
        );
        assert_eq!(line(&plan, "agents tags").0, Status::NotCompiled);
    }

    #[test]
    fn no_model_by_default() {
        let files: [(&str, &[u8]); 2] = [
            M,
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: d\ntier: max\n---\nB\n",
            ),
        ];
        let plan = compile(&files).unwrap();
        assert!(!front(&plan, "agents/dev.md").contains_key("model"));
        assert_eq!(
            line(&plan, "agent dev model"),
            (
                Status::Approximated,
                Some("inherited from the OpenCode session".into())
            )
        );

        let plan = compile_cfg(&files, &tiers("anthropic/x")).unwrap();
        assert_eq!(front(&plan, "agents/dev.md")["model"], "anthropic/x");
        assert_eq!(
            line(&plan, "agent dev model"),
            (
                Status::Compiled,
                Some("from model_tiers in your config".into())
            )
        );

        let native: [(&str, &[u8]); 2] = [
            M,
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: d\ntier: max\nnative:\n  opencode:\n    model: openai/y\n---\nB\n",
            ),
        ];
        let plan = compile_cfg(&native, &tiers("anthropic/x")).unwrap();
        assert_eq!(front(&plan, "agents/dev.md")["model"], "openai/y");
        assert_eq!(
            line(&plan, "agent dev model"),
            (Status::Compiled, Some("from native.opencode.model".into()))
        );

        // not provider/model: never written
        let plan = compile_cfg(&files, &tiers("sonnet")).unwrap();
        assert!(!front(&plan, "agents/dev.md").contains_key("model"));
        assert_eq!(
            line(&plan, "agent dev model"),
            (
                Status::Approximated,
                Some("`sonnet` is not provider/model, ignored".into())
            )
        );

        // no tier, no native model: nothing to say
        let plan = compile(&[
            M,
            (".armadai/agents/dev.md", b"---\ndescription: d\n---\nB\n"),
        ])
        .unwrap();
        assert!(
            !plan
                .report
                .lines
                .iter()
                .any(|l| l.element == "agent dev model")
        );
    }

    #[test]
    fn skills_are_allowed_per_agent() {
        let skill = |id: &str| {
            (
                format!(".armadai/skills/{id}/SKILL.md"),
                format!("---\nname: {id}\ndescription: S\n---\nBody {id}\n"),
            )
        };
        let (a, b) = (skill("a"), skill("b"));
        let plan = compile(&[
            M,
            (a.0.as_str(), a.1.as_bytes()),
            (b.0.as_str(), b.1.as_bytes()),
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: d\nskills: [a, b]\n---\nYou write code.\n",
            ),
            (".armadai/agents/qa.md", b"---\ndescription: d\n---\nB\n"),
        ])
        .unwrap();
        let dev = front(&plan, "agents/dev.md");
        assert_eq!(
            dev["permission"]["skill"],
            serde_json::json!({"*": "deny", "a": "allow", "b": "allow"})
        );
        let keys: Vec<&String> = dev["permission"]["skill"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        assert_eq!(
            keys,
            ["*", "a", "b"],
            "the catch-all first: last match wins"
        );
        let t = text(&plan, "agents/dev.md");
        let (_, body, _) = armadai_model::split_frontmatter(&t).unwrap();
        assert_eq!(
            body,
            "You write code.\n\n## Skills\n\nBefore you start, load these skills with the `skill` tool: `a`, `b`.\n"
        );
        assert_eq!(line(&plan, "agent dev skills").0, Status::Compiled);
        assert!(!front(&plan, "agents/qa.md").contains_key("permission"));
        for id in ["a", "b"] {
            assert_eq!(
                artifact(&plan, &format!("skills/{id}/SKILL.md")).content,
                skill(id).1.as_bytes()
            );
            assert_eq!(line(&plan, &format!("skill {id}")).0, Status::Compiled);
        }
    }

    #[test]
    fn capabilities_become_denies() {
        let plan = compile(&[
            M,
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: d\ncapabilities:\n  read: allow\n  write: deny\n  shell: deny\n  web: deny\n  delegate: deny\n---\nB\n",
            ),
            (
                ".armadai/agents/qa.md",
                b"---\ndescription: d\ncapabilities:\n  read: ask\n  shell: allow\n---\nB\n",
            ),
        ])
        .unwrap();
        assert_eq!(
            front(&plan, "agents/dev.md")["permission"],
            serde_json::json!({
                "edit": "deny", "shell": "deny", "webfetch": "deny",
                "websearch": "deny", "task": "deny"
            })
        );
        assert_eq!(
            front(&plan, "agents/qa.md")["permission"],
            serde_json::json!({"read": "ask", "grep": "ask", "glob": "ask"})
        );
        // grep returns file contents: reading is read, grep and glob in 2.x
        assert_eq!(
            line(&plan, "agent qa capability read=ask"),
            (Status::Compiled, Some("permission read, grep, glob".into()))
        );
        assert_eq!(
            line(&plan, "agent dev capability web=deny"),
            (
                Status::Compiled,
                Some("permission webfetch, websearch".into())
            )
        );
        assert_eq!(
            line(&plan, "agent qa capability shell=allow"),
            (Status::Compiled, Some("OpenCode default".into()))
        );
    }

    #[test]
    fn a_native_key_is_merged_after_and_a_clash_is_an_error() {
        let plan = compile(&[
            M,
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: d\nnative:\n  opencode:\n    steps: 5\n---\nB\n",
            ),
        ])
        .unwrap();
        let keys: Vec<String> = front(&plan, "agents/dev.md").keys().cloned().collect();
        assert_eq!(keys, ["description", "mode", "steps"]);
        assert_eq!(line(&plan, "agent dev native").0, Status::Compiled);

        let err = compile(&[
            M,
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: d\ncapabilities:\n  shell: deny\nnative:\n  opencode:\n    permission: {}\n---\nB\n",
            ),
        ])
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "agent `dev`: native.opencode key `permission` collides with a generated key"
        );
    }

    /// Review Focus 2: a project agent named like a built-in one would replace it.
    #[test]
    fn an_agent_named_like_a_builtin_is_an_error() {
        // literal, not the constant: dropping a name must fail here
        for id in [
            "build",
            "plan",
            "general",
            "explore",
            "scout",
            "compaction",
            "title",
            "summary",
        ] {
            let path = format!(".armadai/agents/{id}.md");
            let err = compile(&[M, (path.as_str(), b"---\ndescription: d\n---\nB\n")]).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!(
                    "agent `{id}` has the name of an OpenCode built-in agent; rename it \
                     (built-in: build, plan, general, explore, scout, compaction, title, summary)"
                )
            );
        }
    }

    /// Through the trait, `model_tiers` comes from the user config; a config that
    /// does not read fails the compilation with its existing message.
    #[test]
    fn the_user_config_is_read_and_a_broken_one_fails() {
        let iso = armadai_core::test_support::IsolatedConfigDir::enter();
        let cfg = iso.config_dir().join("config.yaml");
        std::fs::write(&cfg, "model_tiers:\n  opencode:\n    max: anthropic/x\n").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".armadai/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(dir.path().join(M.0), M.1).unwrap();
        std::fs::write(
            agents.join("dev.md"),
            "---\ndescription: d\ntier: max\n---\nB\n",
        )
        .unwrap();
        let (p, _) = armadai_model::load_checked(dir.path()).unwrap();
        let plan = OpencodeCompiler::new(Path::new(".opencode"))
            .compile(&p, dir.path())
            .unwrap();
        assert_eq!(front(&plan, "agents/dev.md")["model"], "anthropic/x");

        std::fs::write(&cfg, "model_tiers: [\n").unwrap();
        let err = OpencodeCompiler::new(Path::new(".opencode"))
            .compile(&p, dir.path())
            .unwrap_err()
            .to_string();
        let prefix = format!("{} does not read: ", cfg.display());
        assert!(err.starts_with(&prefix), "{err}");
        assert!(err.ends_with("; fix it or move it away"), "{err}");
    }

    const FLOW: (&str, &[u8]) = (
        ".armadai/flows/ship.yaml",
        b"description: Ship a change\nsteps:\n  plan: { agent: lead }\n  build: { agent: dev, on_failure: plan }\n  review: { agent: rev, states: [changes] }\nlinks:\n  - { from: plan, to: build }\n  - { from: build, to: review }\n  - { from: review, to: build, when: changes }\n",
    );

    fn team(manifest: &'static [u8]) -> Vec<(&'static str, &'static [u8])> {
        vec![
            (".armadai/project.yaml", manifest),
            (
                ".armadai/agents/lead.md",
                b"---\ndescription: Leads\n---\nYou lead the team.\n",
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

    fn json(plan: &Plan) -> &JsonKeys {
        let (path, keys) = plan.json.as_ref().expect("opencode.json keys");
        assert_eq!(path, Path::new("opencode.json"));
        keys
    }

    #[test]
    fn coordinator_is_the_default_primary_agent() {
        let plan = compile(&team(b"schema_version: 1\ncoordinator: lead\n")).unwrap();
        assert_eq!(
            text(&plan, "agents/lead.md"),
            "---\ndescription: Leads\nmode: primary\n---\nYou lead the team.\n\n## Delegation\n\n\
             You coordinate the team and the flows described in `.opencode/armadai-team.md`. \
             Delegate each step to its agent with the `subagent` tool, naming the agent by its id.\n"
        );
        // no flow: no flow to speak of
        let mut no_flow = team(b"schema_version: 1\ncoordinator: lead\n");
        no_flow.pop();
        let t = text(&compile(&no_flow).unwrap(), "agents/lead.md");
        assert_eq!(
            t.split_once("## Delegation\n\n").unwrap().1,
            "You coordinate the team described in `.opencode/armadai-team.md`. \
             Delegate with the `subagent` tool, naming the agent by its id.\n"
        );
        assert_eq!(front(&plan, "agents/dev.md")["mode"], "subagent");
        assert_eq!(
            json(&plan),
            &JsonKeys {
                values: BTreeMap::from([("default_agent".into(), "lead".into())]),
                array_entries: BTreeMap::from([(
                    "instructions".into(),
                    vec![".opencode/armadai-team.md".into()]
                )]),
            }
        );
        assert_eq!(
            line(&plan, "agent lead"),
            (
                Status::Compiled,
                Some("coordinator, primary agent, default_agent in opencode.json".into())
            )
        );
    }

    #[test]
    fn team_file_is_neutral() {
        let mut files = team(b"schema_version: 1\ncoordinator: lead\n");
        // a description written as a role must not reach every agent
        files[2] = (
            ".armadai/agents/dev.md",
            b"---\ndescription: You are the developer of the team\n---\nB\n",
        );
        let plan = compile(&files).unwrap();
        let a = artifact(&plan, "armadai-team.md");
        assert_eq!(a.kind, ArtifactKind::Instructions);
        let t = text(&plan, "armadai-team.md");
        assert_eq!(
            t,
            "# Team\n\nGenerated by `armadai link` from `.armadai/`: edit the sources, not this file.\n\n\
             ## Agents\n\n\
             - `dev`\n\
             - `lead` (coordinator, the default primary agent)\n\
             - `rev`\n\n\
             ## Flows\n\n\
             ### Flow `ship`\n\nShip a change\n\nSteps:\n\n\
             - `build` (agent `dev`, on failure: `plan`)\n\
             - `plan` (agent `lead`)\n\
             - `review` (agent `rev`, states: changes)\n\n\
             Transitions:\n\n\
             - `plan` -> `build`\n\
             - `build` -> `review`\n\
             - `review` -> `build` when `changes`\n"
        );
        for role in ["You are", "Tu es", "You coordinate", "You lead", "Leads"] {
            assert!(!t.contains(role), "role phrase {role:?} in\n{t}");
        }
        assert_eq!(
            line(&plan, "team file"),
            (
                Status::Compiled,
                Some(
                    "guidance in .opencode/armadai-team.md, listed in opencode.json instructions"
                        .into()
                )
            )
        );
    }

    const LOOP: [(&str, &[u8]); 4] = [
        (
            ".armadai/project.yaml",
            b"schema_version: 1\npolicy:\n  delegation: strict\n",
        ),
        (".armadai/agents/a.md", b"---\ndescription: A\n---\nB\n"),
        (".armadai/agents/b.md", b"---\ndescription: B\n---\nB\n"),
        (
            ".armadai/flows/f.yaml",
            b"start: sa\nsteps:\n  sa: { agent: a }\n  sb: { agent: b, states: [rejected] }\nlinks:\n  - { from: sa, to: sb }\n  - { from: sb, to: sa, when: rejected }\n",
        ),
    ];

    #[test]
    fn strict_restricts_every_delegation() {
        let plan = compile(&LOOP).unwrap();
        assert_eq!(
            front(&plan, "agents/a.md")["permission"]["task"],
            serde_json::json!({"*": "deny", "b": "allow"})
        );
        assert_eq!(
            front(&plan, "agents/b.md")["permission"]["task"],
            serde_json::json!({"*": "deny", "a": "allow"})
        );
        let main = serde_json::json!({"*": "deny", "a": "allow"});
        let keys = json(&plan);
        assert_eq!(
            keys.values,
            BTreeMap::from([
                ("agent.build.permission.task".into(), main.clone()),
                ("agent.plan.permission.task".into(), main),
                ("experimental.subagent_depth".into(), 2.into()),
            ])
        );
        assert_eq!(
            keys.array_entries,
            BTreeMap::from([(
                "instructions".into(),
                vec![".opencode/armadai-team.md".into()]
            )])
        );
        assert_eq!(
            line(&plan, "policy delegation=strict"),
            (
                Status::Compiled,
                Some(
                    "enforced by OpenCode permissions; agents of your own outside .armadai/ not covered"
                        .into()
                )
            )
        );
        assert_eq!(
            line(&plan, "agents build, plan"),
            (
                Status::Compiled,
                Some("permission.task set to the main-thread rule in opencode.json".into())
            )
        );
        assert_eq!(
            line(&plan, "flow f"),
            (
                Status::Compiled,
                Some("transitions enforced by OpenCode permissions, the rest as guidance".into())
            )
        );
        let t = text(&plan, "armadai-team.md");
        let flows = t.split_once("## Flows\n\n").unwrap().1;
        assert!(
            flows.starts_with(
                "Delegation follows the transitions below, enforced by OpenCode permissions \
                 (`policy.delegation: strict`).\n\n### Flow `f`\n"
            ),
            "{flows}"
        );
    }

    /// The depth OpenCode allows (`experimental.subagent_depth`, default 1) is the
    /// longest delegation chain from the main thread, cycles left out, within 1..=8.
    #[test]
    fn subagent_depth_is_the_longest_chain() {
        let chain = |n: usize| {
            let mut files: Vec<(String, String)> = vec![(
                ".armadai/project.yaml".into(),
                "schema_version: 1\npolicy:\n  delegation: strict\n".into(),
            )];
            let mut flow = String::from("steps:\n");
            for i in 0..n {
                files.push((
                    format!(".armadai/agents/a{i}.md"),
                    "---\ndescription: d\n---\nB\n".into(),
                ));
                flow.push_str(&format!("  s{i}: {{ agent: a{i} }}\n"));
            }
            flow.push_str("links:\n");
            for i in 1..n {
                flow.push_str(&format!("  - {{ from: s{}, to: s{i} }}\n", i - 1));
            }
            if n == 1 {
                flow.truncate(flow.len() - "links:\n".len());
            }
            files.push((".armadai/flows/f.yaml".into(), flow));
            let files: Vec<(&str, &[u8])> = (files.iter())
                .map(|(p, c)| (p.as_str(), c.as_bytes()))
                .collect();
            let plan = compile(&files).unwrap();
            json(&plan).values["experimental.subagent_depth"].clone()
        };
        assert_eq!(chain(1), 1);
        assert_eq!(chain(3), 3);
        assert_eq!(chain(10), 8, "bounded");
    }

    #[test]
    fn off_writes_no_task_rule() {
        let mut files = LOOP.to_vec();
        files[0] = M;
        let plan = compile(&files).unwrap();
        for id in ["a", "b"] {
            assert!(!front(&plan, &format!("agents/{id}.md")).contains_key("permission"));
        }
        let keys = json(&plan);
        assert!(keys.values.is_empty(), "{keys:?}");
        assert_eq!(
            keys.array_entries.keys().collect::<Vec<_>>(),
            ["instructions"]
        );
        assert_eq!(
            line(&plan, "flow f"),
            (Status::Approximated, Some("guidance only".into()))
        );
        assert!(
            !(plan.report.lines.iter())
                .any(|l| l.element.starts_with("policy") || l.element == "agents build, plan"),
            "{}",
            plan.report.render_text()
        );
    }

    /// A strict rule keeps a stricter `delegate` capability: `deny` stays a plain deny,
    /// `ask` asks for each allowed target.
    #[test]
    fn strict_keeps_the_delegate_capability() {
        let mut files = LOOP.to_vec();
        files[1] = (
            ".armadai/agents/a.md",
            b"---\ndescription: A\ncapabilities:\n  delegate: deny\n---\nB\n",
        );
        files[2] = (
            ".armadai/agents/b.md",
            b"---\ndescription: B\ncapabilities:\n  delegate: ask\n---\nB\n",
        );
        let plan = compile(&files).unwrap();
        assert_eq!(front(&plan, "agents/a.md")["permission"]["task"], "deny");
        assert_eq!(
            front(&plan, "agents/b.md")["permission"]["task"],
            serde_json::json!({"*": "deny", "a": "ask"})
        );
    }

    /// Review Focus 5: free agents apply as named; one OpenCode does not know is said.
    #[test]
    fn free_agents_apply_as_named() {
        let mut files = LOOP.to_vec();
        files[0] = (
            ".armadai/project.yaml",
            b"schema_version: 1\npolicy:\n  delegation: strict\n  free_agents: [Explore, general]\n",
        );
        let plan = compile(&files).unwrap();
        assert_eq!(
            front(&plan, "agents/a.md")["permission"]["task"],
            serde_json::json!({"*": "deny", "Explore": "allow", "b": "allow", "general": "allow"})
        );
        assert_eq!(
            line(&plan, "policy free_agents"),
            (
                Status::Approximated,
                Some(
                    "unknown to OpenCode (not an agent of .armadai/ nor a built-in subagent): \
                     Explore; written as is"
                        .into()
                )
            )
        );
        let t = text(&plan, "armadai-team.md");
        assert!(
            t.contains("\nFree agents, delegable from any agent: `Explore`, `general`.\n"),
            "{t}"
        );

        files[0] = (
            ".armadai/project.yaml",
            b"schema_version: 1\npolicy:\n  delegation: strict\n  free_agents: [general, a]\n",
        );
        let plan = compile(&files).unwrap();
        assert!(
            !(plan.report.lines.iter()).any(|l| l.element == "policy free_agents"),
            "{}",
            plan.report.render_text()
        );
    }

    #[test]
    fn native_opencode_of_the_project_is_merged() {
        let plan = compile(&[
            (
                ".armadai/project.yaml",
                b"schema_version: 1\nnative:\n  opencode:\n    share: disabled\n    experimental: { x: 1 }\n",
            ),
            (".armadai/agents/dev.md", b"---\ndescription: d\n---\nB\n"),
        ])
        .unwrap();
        assert_eq!(
            json(&plan),
            &JsonKeys {
                values: BTreeMap::from([
                    ("share".into(), "disabled".into()),
                    ("experimental".into(), serde_json::json!({"x": 1})),
                ]),
                array_entries: BTreeMap::new(),
            }
        );
        assert_eq!(
            line(&plan, "native"),
            (Status::Compiled, Some("merged into opencode.json".into()))
        );

        for (manifest, key) in [
            (
                &b"schema_version: 1\ncoordinator: dev\nnative:\n  opencode:\n    default_agent: x\n"[..],
                "default_agent",
            ),
            (
                b"schema_version: 1\ncoordinator: dev\nnative:\n  opencode:\n    instructions: [a.md]\n",
                "instructions",
            ),
            (
                b"schema_version: 1\npolicy:\n  delegation: strict\nnative:\n  opencode:\n    agent: {}\n",
                "agent",
            ),
            (
                b"schema_version: 1\npolicy:\n  delegation: strict\nnative:\n  opencode:\n    experimental: { x: 1 }\n",
                "experimental",
            ),
        ] {
            let err = compile(&[
                (".armadai/project.yaml", manifest),
                (".armadai/agents/dev.md", b"---\ndescription: d\n---\nB\n"),
            ])
            .unwrap_err();
            assert_eq!(
                err.to_string(),
                format!(
                    "native.opencode key `{key}` collides with a key armadai writes in opencode.json"
                )
            );
        }

        let err = compile(&[
            (
                ".armadai/project.yaml",
                b"schema_version: 1\nnative:\n  opencode:\n    a.b: 1\n",
            ),
            (".armadai/agents/dev.md", b"---\ndescription: d\n---\nB\n"),
        ])
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "native.opencode key `a.b` contains a `.`: armadai cannot own it in opencode.json"
        );
    }

    /// Ruling: a `native.opencode.model` that is not a string (the v2 object form) is
    /// said and not written, with no fallback to `model_tiers`.
    #[test]
    fn a_native_model_that_is_not_a_string_is_ignored() {
        let plan = compile_cfg(
            &[
                M,
                (
                    ".armadai/agents/dev.md",
                    b"---\ndescription: d\ntier: max\nnative:\n  opencode:\n    model: { providerID: a, modelID: b }\n---\nB\n",
                ),
            ],
            &tiers("anthropic/x"),
        )
        .unwrap();
        assert!(!front(&plan, "agents/dev.md").contains_key("model"));
        assert_eq!(
            line(&plan, "agent dev model"),
            (
                Status::Approximated,
                Some("native.opencode.model is not a provider/model string, ignored".into())
            )
        );
    }

    #[test]
    fn automations_are_not_compiled() {
        let plan = compile(&[
            M,
            (
                ".armadai/automations/fmt.yaml",
                b"event: after-tool\nrun: cargo fmt\n",
            ),
        ])
        .unwrap();
        assert_eq!(
            line(&plan, "automation fmt"),
            (Status::NotCompiled, Some("needs an OpenCode plugin".into()))
        );
    }

    const BRIEF: (&str, &[u8]) = (
        ".armadai/commands/brief.md",
        b"---\ndescription: Refine an idea\n---\nRefine: $ARGUMENTS\n",
    );

    #[test]
    fn a_command_compiles_to_a_command_file() {
        let plan = compile(&[M, BRIEF]).unwrap();
        assert_eq!(
            text(&plan, "commands/brief.md"),
            "---\ndescription: Refine an idea\n---\nRefine: $ARGUMENTS\n"
        );
        let a = artifact(&plan, "commands/brief.md");
        assert_eq!(a.kind, ArtifactKind::Command);
        assert_eq!(a.produced_by, ProducedBy::command("brief"));
        assert_eq!(line(&plan, "command brief"), (Status::Compiled, None));
    }

    #[test]
    fn an_argument_hint_is_approximated() {
        let plan = compile(&[
            M,
            (
                ".armadai/commands/brief.md",
                b"---\ndescription: Refine an idea\nargument_hint: <idea>\n---\nRefine: $ARGUMENTS\n",
            ),
        ])
        .unwrap();
        assert_eq!(
            text(&plan, "commands/brief.md"),
            "---\ndescription: Refine an idea\n---\nRefine: $ARGUMENTS\n"
        );
        assert_eq!(
            line(&plan, "command brief"),
            (
                Status::Approximated,
                Some("argument_hint has no OpenCode equivalent".into())
            )
        );
    }

    #[test]
    fn command_native_keys_are_merged_and_a_collision_is_an_error() {
        let plan = compile(&[
            M,
            (
                ".armadai/commands/brief.md",
                b"---\ndescription: Refine an idea\nnative:\n  opencode:\n    agent: dev\n---\nRefine: $ARGUMENTS\n",
            ),
        ])
        .unwrap();
        assert_eq!(
            text(&plan, "commands/brief.md"),
            "---\ndescription: Refine an idea\nagent: dev\n---\nRefine: $ARGUMENTS\n"
        );
        assert_eq!(
            line(&plan, "command brief native"),
            (
                Status::Compiled,
                Some("merged into the command frontmatter".into())
            )
        );
        let err = compile(&[
            M,
            (
                ".armadai/commands/brief.md",
                b"---\ndescription: Refine an idea\nnative:\n  opencode:\n    description: x\n---\nRefine: $ARGUMENTS\n",
            ),
        ])
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "command `brief`: native.opencode key `description` collides with a generated key"
        );
    }

    #[test]
    fn slot_skills_are_prescribed_without_restricting() {
        let plan = compile(&[
            M,
            (
                ".armadai/skills/b-std/SKILL.md",
                b"---\nname: b-std\ndescription: B\nmetadata:\n  armadai-slot: code-standards\n---\nB\n",
            ),
            (
                ".armadai/skills/a-std/SKILL.md",
                b"---\nname: a-std\ndescription: A\nmetadata:\n  armadai-slot: code-standards\n---\nA\n",
            ),
            (
                ".armadai/agents/dev.md",
                b"---\ndescription: d\nslots: [code-standards, security]\n---\nYou write code.\n",
            ),
        ])
        .unwrap();
        // no `skills:`: the slot prescribes, it does not restrict
        assert_eq!(front(&plan, "agents/dev.md").get("permission"), None);
        assert_eq!(
            line(&plan, "agent dev skills"),
            (
                Status::Compiled,
                Some(
                    "loading prescribed in the prompt; no `skills:`, so every skill stays allowed"
                        .into()
                )
            )
        );
        let t = text(&plan, "agents/dev.md");
        let (_, body, _) = armadai_model::split_frontmatter(&t).unwrap();
        assert_eq!(
            body,
            "You write code.\n\n## Skills\n\nBefore you start, load these skills with the `skill` tool: `a-std`, `b-std`.\n"
        );
        assert_eq!(
            line(&plan, "agent dev slot code-standards"),
            (Status::Compiled, Some("a-std, b-std".into()))
        );
        assert_eq!(
            line(&plan, "agent dev slot security"),
            (Status::NotCompiled, Some("no skill fills it".into()))
        );
    }
}
