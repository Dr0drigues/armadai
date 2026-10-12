//! Semantic layer: symbol table and cross-references.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::TARGETS;
use crate::diag::{Diagnostic, Severity, locate, locate_key};
use crate::load::{Kind, Origins, Project, load_project};

const MAX_SKILL_DESCRIPTION: usize = 1024;

pub fn has_errors(ds: &[Diagnostic]) -> bool {
    ds.iter().any(|d| d.severity == Severity::Error)
}

/// Loader diagnostics plus, when a project could be loaded, the semantic ones.
pub fn validate(root: &Path) -> Vec<Diagnostic> {
    let loaded = load_project(root);
    let mut ds = loaded.diagnostics;
    if let Some(p) = &loaded.project {
        ds.extend(check(p, &loaded.origins));
    }
    ds
}

fn valid_id(id: &str) -> bool {
    let mut cs = id.chars();
    id.len() <= 64
        && cs
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// "did you mean" when an id is within edit distance 2, else the known ids.
fn hint_for(token: &str, known: &BTreeSet<&str>) -> Option<String> {
    if let Some(near) = known.iter().find(|k| edit_distance(token, k) <= 2) {
        return Some(format!("did you mean `{near}`?"));
    }
    if known.is_empty() {
        return None;
    }
    Some(format!(
        "known: {}",
        known.iter().copied().collect::<Vec<_>>().join(", ")
    ))
}

struct Out<'a> {
    origins: &'a Origins,
    ds: Vec<Diagnostic>,
}

impl Out<'_> {
    /// Emit a diagnostic of `kind`/`id`, positioned on the first whole-word `token`
    /// (`key` = locate it as a mapping key instead).
    fn emit(
        &mut self,
        (kind, id): (Kind, &str),
        severity: Severity,
        code: &'static str,
        token: Option<(&str, bool)>,
        message: String,
        hint: Option<String>,
    ) {
        let Some((file, text)) = self.origins.files.get(&(kind, id.to_string())) else {
            return;
        };
        let (line, column) = match token {
            Some((t, false)) => locate(text, t),
            Some((t, true)) => locate_key(text, t),
            None => (None, None),
        };
        self.ds.push(Diagnostic {
            severity,
            code,
            file: file.clone(),
            path: String::new(),
            line,
            column,
            message,
            hint,
        });
    }

    fn err(
        &mut self,
        at: (Kind, &str),
        code: &'static str,
        token: Option<(&str, bool)>,
        message: String,
        hint: Option<String>,
    ) {
        self.emit(at, Severity::Error, code, token, message, hint);
    }

    fn unknown_ref(
        &mut self,
        at: (Kind, &str),
        code: &'static str,
        what: &str,
        token: &str,
        known: &BTreeSet<&str>,
    ) {
        self.err(
            at,
            code,
            Some((token, false)),
            format!("unknown {what} `{token}`"),
            hint_for(token, known),
        );
    }

    fn native_keys<'k>(&mut self, at: (Kind, &str), keys: impl Iterator<Item = &'k String>) {
        for k in keys.filter(|k| !TARGETS.contains(&k.as_str())) {
            self.unknown_target(at, k);
        }
    }

    fn unknown_target(&mut self, at: (Kind, &str), t: &str) {
        self.err(
            at,
            "unknown-target",
            Some((t, false)),
            format!("unknown target `{t}`"),
            Some(format!("supported: {}", TARGETS.join(", "))),
        );
    }
}

pub fn check(project: &Project, origins: &Origins) -> Vec<Diagnostic> {
    let mut o = Out {
        origins,
        ds: Vec::new(),
    };
    let agents: BTreeSet<&str> = project.agents.keys().map(String::as_str).collect();
    let skills: BTreeSet<&str> = project.skills.keys().map(String::as_str).collect();

    for (kind, ids) in [
        (Kind::Agent, project.agents.keys().collect::<Vec<_>>()),
        (Kind::Skill, project.skills.keys().collect()),
        (Kind::Flow, project.flows.keys().collect()),
        (Kind::Automation, project.automations.keys().collect()),
    ] {
        for id in ids.into_iter().filter(|id| !valid_id(id)) {
            o.err(
                (kind, id),
                "bad-id",
                None,
                format!("`{id}` is not a valid id"),
                Some("ids match [a-z0-9][a-z0-9-]{0,63}".into()),
            );
        }
    }

    // manifest
    let m = &project.manifest;
    let man = (Kind::Manifest, "project");
    for t in m.targets.iter().filter(|t| !TARGETS.contains(&t.as_str())) {
        o.unknown_target(man, t);
    }
    for t in m.outputs.keys().filter(|t| !TARGETS.contains(&t.as_str())) {
        o.unknown_target(man, t);
    }
    o.native_keys(man, m.native.keys());
    for a in m.coordinator.iter().chain(&m.policy.free_agents) {
        if !agents.contains(a.as_str()) {
            o.unknown_ref(man, "unknown-agent", "agent", a, &agents);
        }
    }

    // agents
    for (id, a) in &project.agents {
        let at = (Kind::Agent, id.as_str());
        if a.front.description.trim().is_empty() {
            o.err(
                at,
                "empty-description",
                Some(("description", true)),
                "agent description is empty".into(),
                None,
            );
        }
        if let Some(t) = a.front.temperature.filter(|t| !t.is_finite()) {
            o.err(
                at,
                "invalid-temperature",
                Some(("temperature", true)),
                format!("temperature must be a finite number, got {t}"),
                None,
            );
        }
        for s in a
            .front
            .skills
            .iter()
            .filter(|s| !skills.contains(s.as_str()))
        {
            o.unknown_ref(at, "unknown-skill", "skill", s, &skills);
        }
        o.native_keys(at, a.front.native.keys());
    }

    // skills
    for (id, s) in &project.skills {
        let at = (Kind::Skill, id.as_str());
        if let Some(n) = s.front.name.as_deref().filter(|n| n != id) {
            o.err(
                at,
                "skill-name-mismatch",
                Some((n, false)),
                format!("skill name `{n}` differs from its folder `{id}`"),
                Some(format!("use `name: {id}`")),
            );
        }
        match s.front.description.as_deref() {
            None => o.err(
                at,
                "skill-missing-description",
                None,
                "skill has no description".into(),
                None,
            ),
            Some(d) if d.trim().is_empty() => o.err(
                at,
                "skill-missing-description",
                Some(("description", true)),
                "skill description is empty".into(),
                None,
            ),
            Some(d) if d.chars().count() > MAX_SKILL_DESCRIPTION => o.err(
                at,
                "skill-description-too-long",
                Some(("description", true)),
                format!(
                    "skill description has {} characters (max {MAX_SKILL_DESCRIPTION})",
                    d.chars().count()
                ),
                None,
            ),
            Some(_) => {}
        }
    }

    // flows
    for (id, f) in &project.flows {
        let at = (Kind::Flow, id.as_str());
        if f.steps.is_empty() {
            o.err(at, "empty-flow", None, "flow has no step".into(), None);
            continue;
        }
        let steps: BTreeSet<&str> = f.steps.keys().map(String::as_str).collect();
        for (sid, s) in &f.steps {
            if !agents.contains(s.agent.as_str()) {
                o.unknown_ref(at, "unknown-agent", "agent", &s.agent, &agents);
            }
            let mut seen = BTreeSet::new();
            for st in &s.states {
                if matches!(st.as_str(), "done" | "failed") || !seen.insert(st) {
                    o.err(
                        at,
                        "duplicate-step-state",
                        Some((st, false)),
                        format!("state `{st}` of step `{sid}` is built in or repeated"),
                        None,
                    );
                }
            }
            if let Some(t) = s.on_failure.as_deref().filter(|t| !steps.contains(t)) {
                o.unknown_ref(at, "unknown-step", "step", t, &steps);
            }
        }

        // failure paths: on_failure + links `when: failed`, per source step
        let mut failure: BTreeMap<&str, usize> = BTreeMap::new();
        for (sid, s) in &f.steps {
            if s.on_failure.is_some() {
                *failure.entry(sid).or_default() += 1;
            }
        }
        for l in &f.links {
            for end in [&l.from, &l.to] {
                if !steps.contains(end.as_str()) {
                    o.unknown_ref(at, "unknown-step", "step", end, &steps);
                }
            }
            if l.from == l.to {
                o.err(
                    at,
                    "self-link",
                    Some((&l.from, false)),
                    format!("step `{}` links to itself", l.from),
                    None,
                );
            }
            if let (Some(w), Some(src)) = (l.when.as_deref(), f.steps.get(&l.from)) {
                if w != "done" && w != "failed" && !src.states.iter().any(|s| s == w) {
                    let known: BTreeSet<&str> = ["done", "failed"]
                        .into_iter()
                        .chain(src.states.iter().map(String::as_str))
                        .collect();
                    o.unknown_ref(at, "unknown-state", "state", w, &known);
                }
                if w == "failed" {
                    *failure.entry(&l.from).or_default() += 1;
                }
            }
        }
        for (sid, n) in failure.into_iter().filter(|(_, n)| *n > 1) {
            o.err(
                at,
                "duplicate-failure-path",
                Some((sid, false)),
                format!("step `{sid}` has {n} failure paths (at most one)"),
                Some("keep either `on_failure` or one `when: failed` link".into()),
            );
        }

        // reachability; `on_failure` counts as a link
        let mut edges: Vec<(&str, &str)> = f
            .links
            .iter()
            .filter(|l| steps.contains(l.from.as_str()) && steps.contains(l.to.as_str()))
            .map(|l| (l.from.as_str(), l.to.as_str()))
            .collect();
        for (sid, s) in &f.steps {
            if let Some(t) = s.on_failure.as_deref().filter(|t| steps.contains(t)) {
                edges.push((sid, t));
            }
        }
        let entries: Vec<&str> = steps
            .iter()
            .copied()
            .filter(|s| !edges.iter().any(|(_, to)| to == s))
            .collect();
        if entries.is_empty() {
            o.emit(
                at,
                Severity::Warning,
                "no-entry-step",
                None,
                "every step has an incoming link: the flow can never start".into(),
                None,
            );
        } else {
            let mut reached: BTreeSet<&str> = entries.iter().copied().collect();
            let mut todo = entries;
            while let Some(s) = todo.pop() {
                for (_, to) in edges.iter().filter(|(from, _)| *from == s) {
                    if reached.insert(to) {
                        todo.push(to);
                    }
                }
            }
            for s in steps.difference(&reached) {
                o.emit(
                    at,
                    Severity::Warning,
                    "unreachable-step",
                    Some((s, false)),
                    format!("step `{s}` is not reachable from any entry step"),
                    None,
                );
            }
        }
    }

    // automations
    for (id, a) in &project.automations {
        let at = (Kind::Automation, id.as_str());
        if a.run.trim().is_empty() {
            o.err(
                at,
                "empty-command",
                Some(("run", true)),
                "`run` is empty".into(),
                None,
            );
        }
        o.native_keys(at, a.native.keys());
    }
    o.ds
}
