//! Legacy project -> canonical model, in memory (spec §6.1).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use armadai_core::agent::{
    Agent as OldAgent, default_temperature, name_matches_reference, slugify,
};
use armadai_core::model_resolution::{ModelTier, parse_latest_placeholder};
use armadai_core::orchestration::OrchestrationPattern;
use armadai_core::orchestration::policy::PolicyMode;
use armadai_model::{
    Agent, AgentFront, Delegation, Flow, Link, Manifest, Policy, Project, Skill, SkillFront, Step,
    TARGETS, Tier,
};
use serde_yaml_ng::{Mapping, Value};

use super::read::OldProject;
use super::report::Report;

/// A legacy skill directory whose files other than `SKILL.md` are copied as they are.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillCopy {
    pub id: String,
    pub from: PathBuf,
}

const MAX_DERIVED_DESCRIPTION: usize = 200;
const MAX_SKILL_DESCRIPTION: usize = 1024;
const ORCHESTRATION_FLOW: &str = "orchestration";
const NO_ENGINE: &str = "the execution engine was removed";
const NO_READER: &str = "no reader in the new format";
pub const PROVIDER_REASON: &str = "the target tool picks its own provider";

pub fn convert(old: &OldProject) -> (Project, Vec<SkillCopy>, Report) {
    let mut r = Report::default();
    r.warnings.extend(old.load_warnings.iter().cloned());
    r.warn("YAML comments of the legacy files are not preserved; review the backup");
    r.warn(
        "unknown keys of the legacy config file are not detected (the legacy loader ignores them); \
         review the backup",
    );

    // agent ids, in roster order: the first holder of a slug keeps it
    let mut taken = BTreeSet::new();
    let mut agents = BTreeMap::new();
    let mut ids: Vec<(String, &OldAgent)> = Vec::new();
    let mut derived = Vec::new();
    for (i, a) in old.agents.iter().enumerate() {
        let id = unique_id(&a.name, &format!("agent-{}", i + 1), &mut taken, &mut r);
        if !a.source.starts_with(&old.root) {
            r.warn(format!(
                "agent `{id}` copied from the user library ({})",
                a.source.display()
            ));
        }
        let (agent, was_derived) = convert_agent(&id, a, old, &mut r);
        if was_derived {
            derived.push(id.clone());
        }
        agents.insert(id.clone(), agent);
        ids.push((id, a));
    }
    if derived.len() <= 3 {
        for id in &derived {
            r.warn(format!(
                "agent `{id}`: description derived from the first sentence of its prompt"
            ));
        }
    } else {
        r.warn(format!(
            "description derived from the first sentence of the prompt for {} agents: {}",
            derived.len(),
            derived.join(", ")
        ));
    }
    let resolve = |reference: &str| -> Option<String> {
        ids.iter()
            .find(|(_, a)| {
                name_matches_reference(&a.name, reference)
                    || (a.source.extension().is_some_and(|e| e == "md")
                        && a.source.file_stem().is_some_and(|s| s == reference))
            })
            .map(|(id, _)| id.clone())
    };

    let mut manifest = Manifest {
        schema_version: armadai_model::SCHEMA_VERSION,
        name: None,
        targets: Vec::new(),
        outputs: BTreeMap::new(),
        coordinator: None,
        policy: Policy::default(),
        native: Default::default(),
    };
    convert_link(old, &resolve, &mut manifest, &mut r);
    let mut flows = BTreeMap::new();
    if let Some(o) = &old.config.orchestration
        && let Some(flow) = convert_orchestration(o, &resolve, &mut manifest.policy, &mut r)
    {
        flows.insert(ORCHESTRATION_FLOW.to_string(), flow);
    }
    project_dead_sections(old, &mut r);

    let mut skill_ids = BTreeSet::new();
    let mut skills = BTreeMap::new();
    // legacy skills first: one already in `.armadai/skills/<id>/` keeps its directory
    let mut copies = Vec::new();
    for (i, s) in old.skills.iter().enumerate() {
        let dir_name = s.dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let id = unique_id(
            dir_name,
            &format!("skill-{}", i + 1),
            &mut skill_ids,
            &mut r,
        );
        let (front, body) = convert_skill(&id, &s.text, &s.dir, &mut r);
        skills.insert(id.clone(), Skill { front, body });
        copies.push(SkillCopy {
            id,
            from: s.dir.clone(),
        });
    }

    for (i, p) in old.prompts.iter().enumerate() {
        let id = unique_id(&p.name, &format!("skill-{}", i + 1), &mut skill_ids, &mut r);
        if !p.apply_to.is_empty() {
            r.drop(
                &id,
                "apply_to",
                "a skill is loaded on demand by the target, not attached to agents",
            );
        }
        let description = skill_description(&id, p.description.as_deref(), &p.body, &mut r);
        let front = SkillFront {
            name: Some(id.clone()),
            description: Some(description),
            ..Default::default()
        };
        let body = p.body.clone();
        skills.insert(id, Skill { front, body });
    }
    let project = Project {
        manifest,
        agents,
        skills,
        flows,
        automations: BTreeMap::new(),
    };
    (project, copies, r)
}

/// `[a-z0-9][a-z0-9-]{0,63}` from `name`, unique within `taken`; a rename is reported.
fn unique_id(name: &str, fallback: &str, taken: &mut BTreeSet<String>, r: &mut Report) -> String {
    let mut base: String = slugify(&transliterate(name)).chars().take(64).collect();
    base = base.trim_end_matches('-').to_string();
    if base.is_empty() {
        base = fallback.to_string();
    }
    let mut id = base.clone();
    let mut n = 1;
    while taken.contains(&id) {
        n += 1;
        let suffix = format!("-{n}");
        let stem: String = base.chars().take(64 - suffix.len()).collect();
        id = format!("{}{suffix}", stem.trim_end_matches('-'));
    }
    if id != slugify(name) {
        r.warn(format!("`{name}` renamed to `{id}`"));
    }
    taken.insert(id.clone());
    id
}

/// Common Latin accents folded to ASCII, so `slugify` keeps the letter instead of dropping it.
fn transliterate(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.to_lowercase().chars() {
        match c {
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'à' | 'â' | 'ä' => out.push('a'),
            'î' | 'ï' => out.push('i'),
            'ô' | 'ö' => out.push('o'),
            'ù' | 'û' | 'ü' => out.push('u'),
            'ç' => out.push('c'),
            'œ' => out.push_str("oe"),
            c => out.push(c),
        }
    }
    out
}

/// First sentence of the first paragraph of `text`, on one line. Longer than
/// `max` characters: cut at the last space and ended with `…`.
fn first_line(text: &str, max: usize) -> Option<String> {
    let para = text.trim_start().split("\n\n").next()?;
    let one_line = para.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return None;
    }
    let bytes = one_line.as_bytes();
    let end = (0..bytes.len())
        .find(|&i| {
            matches!(bytes[i], b'.' | b'?' | b'!') && bytes.get(i + 1).is_none_or(|b| *b == b' ')
        })
        .map_or(one_line.len(), |i| i + 1);
    let sentence = &one_line[..end];
    if sentence.chars().count() <= max {
        return Some(sentence.to_string());
    }
    let head: String = sentence.chars().take(max + 1).collect();
    let cut = head.rfind(' ').map_or_else(
        || sentence.chars().take(max).collect(),
        |i| head[..i].to_string(),
    );
    Some(format!("{cut}…"))
}

/// The agent, and whether its description was derived.
fn convert_agent(id: &str, a: &OldAgent, old: &OldProject, r: &mut Report) -> (Agent, bool) {
    let mut body = a.composed_prompt();
    if let Some(l) = old.leftovers.get(&a.source) {
        for (label, text) in &l.kept {
            body.truncate(body.trim_end().len());
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            body.push_str(text);
            r.warn(format!("agent `{id}`: kept verbatim: {label}"));
        }
        for key in &l.unknown_metadata {
            r.drop(
                id,
                &format!("metadata.{key}"),
                "ignored by the legacy loader",
            );
        }
    }
    let mut derived = false;
    let description = match old.declared_descriptions.get(&a.name) {
        Some(d) if !d.trim().is_empty() => d.clone(),
        _ => {
            derived = true;
            first_line(&body, MAX_DERIVED_DESCRIPTION).unwrap_or_else(|| a.name.clone())
        }
    };
    let description = if description.trim().is_empty() {
        id.to_string()
    } else {
        description
    };
    let m = &a.metadata;
    let temperature =
        ((m.temperature - default_temperature()).abs() > f32::EPSILON).then_some(m.temperature);
    let mut front = AgentFront {
        description,
        title: (!a.name.is_empty()).then(|| a.name.clone()),
        tier: None,
        temperature,
        tags: m.tags.clone(),
        skills: Vec::new(),
        capabilities: Default::default(),
        native: Default::default(),
    };
    convert_model(id, &m.provider, m.model.as_deref(), &mut front, r);

    r.drop(id, "provider", PROVIDER_REASON);
    let present = [
        ("command", m.command.is_some(), NO_READER),
        ("args", m.args.is_some(), NO_READER),
        ("max_tokens", m.max_tokens.is_some(), NO_READER),
        ("timeout", m.timeout.is_some(), NO_READER),
        ("stacks", !m.stacks.is_empty(), NO_READER),
        ("scope", !m.scope.is_empty(), NO_READER),
        ("model_fallback", !m.model_fallback.is_empty(), NO_READER),
        ("cost_limit", m.cost_limit.is_some(), NO_ENGINE),
        ("rate_limit", m.rate_limit.is_some(), NO_ENGINE),
        ("context_window", m.context_window.is_some(), NO_READER),
        ("mode", m.mode.is_some(), NO_ENGINE),
        ("orchestration", m.orchestration.is_some(), NO_ENGINE),
        ("triggers", m.triggers.is_some(), NO_ENGINE),
        ("ring_config", m.ring_config.is_some(), NO_ENGINE),
        ("pipeline", a.pipeline.is_some(), NO_ENGINE),
    ];
    for (field, is_present, reason) in present {
        if is_present {
            r.drop(id, field, reason);
        }
    }
    (Agent { front, body }, derived)
}

fn convert_model(
    id: &str,
    provider: &str,
    model: Option<&str>,
    f: &mut AgentFront,
    r: &mut Report,
) {
    let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) else {
        return;
    };
    if let Some(tier) = parse_latest_placeholder(model) {
        f.tier = Some(match tier {
            ModelTier::Fast => Tier::Fast,
            ModelTier::Pro => Tier::Pro,
            ModelTier::Max => Tier::Max,
        });
        return;
    }
    if model.starts_with("latest") {
        r.drop(id, "model", format!("`{model}` has no portable equivalent"));
        return;
    }
    let target = match provider {
        "anthropic" | "claude" => "claude",
        "openai" | "codex" => "codex",
        _ => {
            r.drop(
                id,
                "model",
                format!("`{model}`: provider `{provider}` maps to no supported target"),
            );
            return;
        }
    };
    let mut block = Mapping::new();
    block.insert("model".into(), model.into());
    f.native.insert(target.to_string(), Value::Mapping(block));
}

fn convert_link(
    old: &OldProject,
    resolve: &dyn Fn(&str) -> Option<String>,
    m: &mut Manifest,
    r: &mut Report,
) {
    let Some(link) = &old.config.link else {
        return;
    };
    if let Some(t) = &link.target {
        if TARGETS.contains(&t.as_str()) {
            m.targets.push(t.clone());
        } else {
            r.drop(
                "project",
                "link.target",
                format!("target `{t}` is not supported"),
            );
            r.warn(format!(
                "link target `{t}` is not supported: no default target written"
            ));
        }
    }
    for (t, o) in &link.overrides {
        let Some(out) = &o.output else { continue };
        if TARGETS.contains(&t.as_str()) {
            m.outputs.insert(t.clone(), out.clone());
        } else {
            r.drop(
                "project",
                &format!("link.overrides.{t}"),
                format!("target `{t}` is not supported"),
            );
        }
    }
    if let Some(c) = &link.coordinator {
        match resolve(c) {
            Some(id) => m.coordinator = Some(id),
            None => r.warn(format!(
                "link.coordinator `{c}` matches no migrated agent: no coordinator written"
            )),
        }
    }
}

fn convert_orchestration(
    o: &armadai_core::orchestration::OrchestrationConfig,
    resolve: &dyn Fn(&str) -> Option<String>,
    policy: &mut Policy,
    r: &mut Report,
) -> Option<Flow> {
    const S: &str = "orchestration";
    let known = |name: &str, r: &mut Report| -> Option<String> {
        let id = resolve(name);
        if id.is_none() {
            r.warn(format!(
                "orchestration references unknown agent `{name}`: omitted from the flow"
            ));
        }
        id
    };
    let mut steps = BTreeMap::new();
    let mut links: Vec<Link> = Vec::new();
    let step = |id: &str, steps: &mut BTreeMap<String, Step>| {
        steps.entry(id.to_string()).or_insert_with(|| Step {
            agent: id.to_string(),
            states: Vec::new(),
            on_failure: None,
        });
    };
    let link = |from: &str, to: &str, links: &mut Vec<Link>| {
        let l = Link {
            from: from.to_string(),
            to: to.to_string(),
            when: None,
        };
        if from != to && !links.contains(&l) {
            links.push(l);
        }
    };
    let coordinator = o.coordinator.as_deref().and_then(|c| known(c, r));
    if let Some(c) = &coordinator {
        step(c, &mut steps);
    }
    for (i, team) in o.teams.iter().enumerate() {
        let lead = team.lead.as_deref().and_then(|l| known(l, r));
        if let Some(l) = &lead {
            step(l, &mut steps);
            if let Some(c) = &coordinator {
                link(c, l, &mut links);
            }
        }
        for m in team.agents.iter().filter_map(|m| known(m, r)) {
            step(&m, &mut steps);
            if let Some(parent) = lead.as_ref().or(coordinator.as_ref()) {
                link(parent, &m, &mut links);
            }
        }
        let subject = format!("orchestration.teams[{i}]");
        for (field, present) in [
            ("pattern", team.pattern.is_some()),
            ("max_rounds", team.max_rounds.is_some()),
            ("max_laps", team.max_laps.is_some()),
            ("consensus_threshold", team.consensus_threshold.is_some()),
        ] {
            if present {
                r.drop(&subject, field, NO_ENGINE);
            }
        }
    }

    policy.delegation = match o.policy {
        PolicyMode::Off => Delegation::Off,
        PolicyMode::Strict => Delegation::Strict,
    };
    for f in &o.free_agents {
        // not a project agent: a built-in of the target (Claude Code's `Explore`), kept
        let id = resolve(f).unwrap_or_else(|| f.clone());
        if !policy.free_agents.contains(&id) {
            policy.free_agents.push(id);
        }
    }

    for (field, present) in [
        ("enabled", o.enabled),
        ("pattern", o.pattern != OrchestrationPattern::Direct),
        ("routes", !o.routes.is_empty()),
        ("max_depth", o.max_depth.is_some()),
        ("max_iterations", o.max_iterations.is_some()),
        ("max_concurrency", o.max_concurrency.is_some()),
        ("timeout", o.timeout.is_some()),
        ("max_rounds", o.max_rounds.is_some()),
        ("max_laps", o.max_laps.is_some()),
        ("consensus_threshold", o.consensus_threshold.is_some()),
        ("token_budget", o.token_budget.is_some()),
        ("cost_limit", o.cost_limit.is_some()),
    ] {
        if present {
            r.drop(S, field, NO_ENGINE);
        }
    }

    if steps.is_empty() {
        if o.coordinator.is_some() || !o.teams.is_empty() {
            r.warn("orchestration names no migrated agent: no flow written");
        }
        return None;
    }
    Some(Flow {
        description: Some("Delegations of the legacy `orchestration:` section".into()),
        steps,
        links,
    })
}

fn project_dead_sections(old: &OldProject, r: &mut Report) {
    let c = &old.config;
    let d = &c.defaults;
    for (field, present, reason) in [
        ("sources", !c.sources.is_empty(), NO_READER),
        (
            "defaults",
            d.mode.is_some() || d.orchestration.is_some(),
            NO_ENGINE,
        ),
        ("routing", c.routing.is_some(), NO_ENGINE),
        (
            "registries",
            c.registries.is_some(),
            "not part of the project format",
        ),
    ] {
        if present {
            r.drop("project", field, reason);
        }
    }
}

fn skill_description(id: &str, declared: Option<&str>, body: &str, r: &mut Report) -> String {
    let d = match declared.filter(|d| !d.trim().is_empty()) {
        Some(d) => d.to_string(),
        None => {
            r.warn(format!(
                "skill `{id}`: description derived from the first sentence of its body"
            ));
            first_line(body, MAX_DERIVED_DESCRIPTION).unwrap_or_else(|| id.to_string())
        }
    };
    if d.chars().count() > MAX_SKILL_DESCRIPTION {
        r.warn(format!(
            "skill `{id}`: description cut to {MAX_SKILL_DESCRIPTION} characters"
        ));
        return d.chars().take(MAX_SKILL_DESCRIPTION).collect();
    }
    d
}

/// The legacy `SKILL.md` with `name` set to the id and `tools`/`version` dropped;
/// every other key is kept (`SkillFront::extra`).
fn convert_skill(id: &str, text: &str, dir: &Path, r: &mut Report) -> (SkillFront, String) {
    let (yaml, body) = match armadai_model::split_frontmatter(text) {
        Some((y, b, _)) => (y, b.to_string()),
        None => ("", text.to_string()),
    };
    let mut map: Mapping = match serde_yaml_ng::from_str::<Option<Mapping>>(yaml) {
        Ok(m) => m.unwrap_or_default(),
        Err(e) => {
            r.warn(format!(
                "skill `{id}`: frontmatter of {} unreadable ({e}); only its body is kept",
                dir.join("SKILL.md").display()
            ));
            Mapping::new()
        }
    };
    for key in ["tools", "version"] {
        if map.remove(key).is_some() {
            r.drop(id, key, "not part of the skill standard");
        }
    }
    let old_name = map.remove("name");
    if old_name
        .as_ref()
        .and_then(Value::as_str)
        .is_some_and(|n| n != id)
    {
        r.warn(format!(
            "skill `{id}`: name {:?} replaced by its directory id",
            old_name
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        ));
    }
    let declared = map.remove("description");
    let declared = declared.as_ref().and_then(Value::as_str);
    let description = skill_description(id, declared, &body, r);
    let mut front: SkillFront = match serde_yaml_ng::from_value(Value::Mapping(map)) {
        Ok(f) => f,
        Err(e) => {
            r.warn(format!(
                "skill `{id}`: frontmatter keys not kept ({e}); review the backup"
            ));
            SkillFront::default()
        }
    };
    front.name = Some(id.to_string());
    front.description = Some(description);
    (front, body)
}

#[cfg(test)]
mod tests;
