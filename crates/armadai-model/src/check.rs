//! Semantic layer: symbol table and cross-references.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::TARGETS;
use crate::diag::{Diagnostic, Severity, locate, locate_key};
use crate::load::{Kind, Origins, Project, load_project};

const MAX_SKILL_DESCRIPTION: usize = 1024;

pub fn has_errors(ds: &[Diagnostic]) -> bool {
    ds.iter().any(|d| d.severity == Severity::Error)
}

/// Loader diagnostics plus, when a project could be loaded, the semantic ones.
pub fn validate(root: &Path) -> Vec<Diagnostic> {
    checked(root).2
}

fn checked(root: &Path) -> (Option<Project>, Origins, Vec<Diagnostic>) {
    let loaded = load_project(root);
    let mut ds = loaded.diagnostics;
    if let Some(p) = &loaded.project {
        ds.extend(check(p, &loaded.origins));
    }
    (loaded.project, loaded.origins, ds)
}

/// The project at `root`, or every diagnostic when at least one is an error
/// (warnings alone do not refuse).
pub fn load_checked(root: &Path) -> Result<(Project, Origins), Vec<Diagnostic>> {
    match checked(root) {
        (Some(p), origins, ds) if !has_errors(&ds) => Ok((p, origins)),
        (_, _, ds) => Err(ds),
    }
}

/// `[a-z0-9][a-z0-9-]{0,63}`
pub fn is_valid_id(id: &str) -> bool {
    let mut cs = id.chars();
    id.len() <= 64
        && cs
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Optimal string alignment distance (a swap of two letters costs 1).
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let sub = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + sub);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// "did you mean" when an id is within edit distance 2, else the known ids.
fn hint_for(token: &str, known: &BTreeSet<&str>) -> Option<String> {
    // closest id (ties: alphabetical, the set is sorted), within a bound that grows with length
    let bound = (token.chars().count() / 3).max(1);
    let best = known
        .iter()
        .map(|k| (edit_distance(token, k), *k))
        .min_by_key(|(d, _)| *d);
    if let Some((_, near)) = best.filter(|(d, _)| *d <= bound) {
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
        // A diagnostic without a known origin is kept, just without a position.
        let origin = self.origins.files.get(&(kind, id.to_string()));
        let (line, column) = match (origin, token) {
            (Some((_, text)), Some((t, false))) => locate(text, t),
            (Some((_, text)), Some((t, true))) => locate_key(text, t),
            _ => (None, None),
        };
        let file = origin.map_or_else(|| PathBuf::from(id), |(f, _)| f.clone());
        self.ds.push(Diagnostic {
            severity,
            code,
            file,
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
        for id in ids.into_iter().filter(|id| !is_valid_id(id)) {
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
    for a in m
        .coordinator
        .iter()
        .filter(|a| !agents.contains(a.as_str()))
    {
        o.unknown_ref(man, "unknown-agent", "agent", a, &agents);
    }
    // a free agent may be one the target provides itself (Claude Code's `Explore`, ...)
    for a in (m.policy.free_agents.iter()).filter(|a| !agents.contains(a.as_str())) {
        let hint = hint_for(a, &agents)
            .filter(|h| h.starts_with("did you mean"))
            .unwrap_or_else(|| {
                "fine if the target provides it (Claude Code: Explore, Plan, general-purpose)"
                    .into()
            });
        o.emit(
            man,
            Severity::Warning,
            "unknown-free-agent",
            Some((a, false)),
            format!("free agent `{a}` is not an agent of this project"),
            Some(hint),
        );
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
            if s.on_failure.as_deref() == Some(sid.as_str()) {
                o.err(
                    at,
                    "self-link",
                    Some(("on_failure", true)),
                    format!("step `{sid}` has itself as `on_failure`"),
                    None,
                );
            }
            if !is_valid_id(sid) {
                o.err(
                    at,
                    "bad-id",
                    Some((sid, false)),
                    format!("step id `{sid}` is not a valid id"),
                    Some("ids match [a-z0-9][a-z0-9-]{0,63}".into()),
                );
            }
        }

        // failure paths: on_failure + links `when: failed`, per source step
        let mut failure: BTreeMap<&str, usize> = BTreeMap::new();
        for (sid, s) in &f.steps {
            if s.on_failure.is_some() {
                *failure.entry(sid).or_default() += 1;
            }
        }
        let mut seen_links = BTreeSet::new();
        for l in &f.links {
            let key = (&l.from, &l.to, l.when.as_deref().unwrap_or("done"));
            if !seen_links.insert(key) {
                o.err(
                    at,
                    "duplicate-link",
                    Some((&l.from, false)),
                    format!("link `{}` -> `{}` is declared twice", l.from, l.to),
                    None,
                );
            }
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
