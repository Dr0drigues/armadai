//! OpenCode 2.x adapter: agents (`.opencode/agents/<id>.md`), skills, models and
//! capabilities as per-agent permissions.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use armadai_core::config::{UserConfig, model_for_tier};
use armadai_model::{Agent, AgentFront, Cap, Delegation, NativeValue, Project};
use serde_json::{Map, Value};

use super::{Artifact, ArtifactKind, Compiler, Plan, Report, Status};
use crate::linker::manifest::ProducedBy;

const T: &str = "opencode";

/// Agents OpenCode ships: a project agent of the same name would replace it.
pub const BUILTIN_AGENTS: [&str; 5] = ["build", "plan", "general", "explore", "scout"];

/// Capability → the OpenCode 2.x permission keys it governs (v1 agent format, which
/// 2.x migrates: `task` becomes the `subagent` action).
const CAP_KEYS: [(&str, &[&str]); 5] = [
    ("read", &["read"]),
    ("write", &["edit"]),
    ("shell", &["shell"]),
    ("web", &["webfetch", "websearch"]),
    ("delegate", &["task"]),
];

const STEP4: &str = "not supported by this target yet (step 4)";

pub struct OpencodeCompiler;

impl Compiler for OpencodeCompiler {
    fn compile(&self, project: &Project, root: &Path) -> anyhow::Result<Plan> {
        compile_with(
            project,
            root,
            &armadai_core::config::try_load_user_config()?,
        )
    }
}

fn compile_with(project: &Project, root: &Path, config: &UserConfig) -> anyhow::Result<Plan> {
    let mut plan = Plan::default();
    let coordinator = project.manifest.coordinator.as_deref();
    informational(&mut plan.report, project);
    for (id, agent) in &project.agents {
        if BUILTIN_AGENTS.contains(&id.as_str()) {
            bail!(
                "agent `{id}` has the name of an OpenCode built-in agent; rename it \
                 (built-in: {})",
                BUILTIN_AGENTS.join(", ")
            );
        }
        let is_lead = Some(id.as_str()) == coordinator;
        plan.artifacts.push(Artifact {
            path: PathBuf::from(format!("agents/{id}.md")),
            content: agent_file(id, agent, is_lead, config, &mut plan.report)?.into_bytes(),
            kind: ArtifactKind::Agent,
            produced_by: ProducedBy::agent(id),
        });
        let note = is_lead.then_some("coordinator, primary agent");
        plan.report
            .push(T, format!("agent {id}"), Status::Compiled, note);
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
    plan.artifacts.sort_by(|a, b| a.path.cmp(&b.path));

    for id in project.automations.keys() {
        plan.report.push(
            T,
            format!("automation {id}"),
            Status::NotCompiled,
            Some("needs an OpenCode plugin"),
        );
    }
    // shortcut: flows and the strict policy are reported as before until task 4 compiles them
    for id in project.flows.keys() {
        plan.report
            .push(T, format!("flow {id}"), Status::NotCompiled, Some(STEP4));
    }
    if project.manifest.policy.delegation == Delegation::Strict {
        plan.report.push(
            T,
            "policy delegation=strict",
            Status::NotCompiled,
            Some("no delegation gate for this target (step 4)"),
        );
    }
    Ok(plan)
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
    let native = (f.native.get(T))
        .and_then(|n| n.get("model"))
        .and_then(|m| m.as_str())
        .map(str::to_string);
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
    id: &str,
    agent: &Agent,
    is_lead: bool,
    config: &UserConfig,
    r: &mut Report,
) -> anyhow::Result<String> {
    let f = &agent.front;
    let mut m = Map::new();
    m.insert("description".into(), f.description.as_str().into());
    let mode = if is_lead { "primary" } else { "subagent" };
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
    let mut body = agent.body.clone();
    if !f.skills.is_empty() {
        // the catch-all first: OpenCode applies the last matching rule
        let mut skill = Map::from_iter([("*".to_string(), Value::from("deny"))]);
        for s in &f.skills {
            skill.insert(s.clone(), "allow".into());
        }
        permission.insert("skill".into(), Value::Object(skill));
        let list: Vec<String> = f.skills.iter().map(|s| format!("`{s}`")).collect();
        body = format!(
            "{}\n\n## Skills\n\nBefore you start, load these skills with the `skill` tool: {}.\n",
            body.trim_end(),
            list.join(", ")
        );
        r.push(
            T,
            format!("agent {id} skills"),
            Status::Compiled,
            Some("allowed through permission.skill, loading prescribed in the prompt"),
        );
    }
    if !permission.is_empty() {
        m.insert("permission".into(), Value::Object(permission));
    }
    if let Some(native) = f.native.get(T) {
        let Some(native) = native.as_object() else {
            bail!("agent `{id}`: native.opencode must be a mapping");
        };
        // `model` was resolved above, as the documented override of the tier
        for (k, v) in native.iter().filter(|(k, _)| *k != "model") {
            if m.contains_key(k) {
                bail!("agent `{id}`: native.opencode key `{k}` collides with a generated key");
            }
            m.insert(k.clone(), v.clone());
        }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;
    use crate::compile::{Artifact, Status};

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
        compile_with(&p, dir.path(), config)
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
            serde_json::json!({"read": "ask"})
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
        for id in BUILTIN_AGENTS {
            let path = format!(".armadai/agents/{id}.md");
            let err = compile(&[M, (path.as_str(), b"---\ndescription: d\n---\nB\n")]).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!(
                    "agent `{id}` has the name of an OpenCode built-in agent; rename it \
                     (built-in: build, plan, general, explore, scout)"
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
        let plan = OpencodeCompiler.compile(&p, dir.path()).unwrap();
        assert_eq!(front(&plan, "agents/dev.md")["model"], "anthropic/x");

        std::fs::write(&cfg, "model_tiers: [\n").unwrap();
        let err = OpencodeCompiler
            .compile(&p, dir.path())
            .unwrap_err()
            .to_string();
        let prefix = format!("{} does not read: ", cfg.display());
        assert!(err.starts_with(&prefix), "{err}");
        assert!(err.ends_with("; fix it or move it away"), "{err}");
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
}
