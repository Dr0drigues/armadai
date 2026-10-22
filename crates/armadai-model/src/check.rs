//! Semantic layer: symbol table and cross-references.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::TARGETS;
use crate::diag::{Diagnostic, Severity, locate, locate_key};
use crate::load::{Kind, Origins, Project, load_project};

const MAX_SKILL_DESCRIPTION: usize = 1024;

/// Names of the built-in commands and skills of the targets, sorted. Sources: Claude Code
/// `code.claude.com/docs/en/commands.md` (read on 2026-10-21); OpenCode
/// `packages/web/src/content/docs/tui.mdx` at commit 681156c8.
pub const BUILTIN_COMMANDS: &[&str] = &[
    "add-dir",
    "adddir",
    "advisor",
    "agents",
    "allowed-tools",
    "android",
    "app",
    "artifacts",
    "autocompact",
    "autofix-pr",
    "background",
    "bashes",
    "batch",
    "bg",
    "branch",
    "btw",
    "bug",
    "cd",
    "checkpoint",
    "checkup",
    "chrome",
    "claude-api",
    "claude-in-chrome",
    "clear",
    "code-review",
    "color",
    "compact",
    "config",
    "connect",
    "context",
    "continue",
    "copy",
    "cost",
    "debug",
    "deep-research",
    "design",
    "desktop",
    "details",
    "diff",
    "doctor",
    "editor",
    "effort",
    "exit",
    "export",
    "extra-usage",
    "fast",
    "feedback",
    "fewer-permission-prompts",
    "focus",
    "fork",
    "goal",
    "heapdump",
    "help",
    "hooks",
    "ide",
    "import",
    "init",
    "insights",
    "install-github-app",
    "install-slack-app",
    "ios",
    "keybindings",
    "list-agents",
    "login",
    "logout",
    "loop",
    "mcp",
    "memory",
    "mobile",
    "model",
    "models",
    "new",
    "output-style",
    "passes",
    "peers",
    "permissions",
    "plan",
    "plugin",
    "powerup",
    "pr-comments",
    "privacy-settings",
    "proactive",
    "q",
    "quit",
    "radio",
    "rate-limit-options",
    "rc",
    "recap",
    "redo",
    "release-notes",
    "reload-plugins",
    "reload-skills",
    "remote-control",
    "remote-env",
    "rename",
    "reset",
    "resume",
    "review",
    "rewind",
    "routines",
    "run",
    "sandbox",
    "schedule",
    "scroll-speed",
    "security-review",
    "sessions",
    "settings",
    "setup-bedrock",
    "setup-vertex",
    "share",
    "simplify",
    "skill-doctor",
    "skills",
    "stats",
    "status",
    "statusline",
    "stickers",
    "stop",
    "subtask",
    "summarize",
    "tasks",
    "team-onboarding",
    "teleport",
    "terminal-setup",
    "theme",
    "themes",
    "thinking",
    "tp",
    "tui",
    "ultraplan",
    "ultrareview",
    "undo",
    "update-config",
    "upgrade",
    "usage",
    "usage-credits",
    "verify",
    "vim",
    "voice",
    "web-setup",
    "workflows",
];

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

/// The steps a flow starts from: `start` when the flow declares it. Otherwise a
/// heuristic: a candidate is a step no unconditional link
/// (`when` absent or `done`) leads to: a conditional link (`when: rejected`,
/// `when: failed`) and `on_failure` never make a step a non-entry by themselves.
/// Among the candidates, one another candidate reaches (by any edge) without being
/// reached back is not an entry: `ship`, reached from `write` only `when:
/// approved`, is where a review loop ends, not where it starts.
/// Links naming an unknown step are ignored (reported elsewhere).
pub fn entry_steps(flow: &crate::load::Flow) -> Vec<String> {
    // a declared start is the entry, and the only one (an unknown one: none)
    if let Some(s) = &flow.start {
        return flow
            .steps
            .contains_key(s)
            .then(|| s.clone())
            .into_iter()
            .collect();
    }
    let known = |l: &&crate::types::Link| {
        flow.steps.contains_key(&l.from) && flow.steps.contains_key(&l.to)
    };
    let into: BTreeSet<&str> = (flow.links.iter().filter(known))
        .filter(|l| l.when.as_deref().is_none_or(|w| w == "done"))
        .map(|l| l.to.as_str())
        .collect();
    let edges: Vec<(&str, &str)> = (flow.links.iter().filter(known))
        .map(|l| (l.from.as_str(), l.to.as_str()))
        .chain(flow.steps.iter().filter_map(|(sid, s)| {
            (s.on_failure.as_deref())
                .filter(|t| flow.steps.contains_key(*t))
                .map(|t| (sid.as_str(), t))
        }))
        .collect();
    let reach = |from: &str| -> BTreeSet<&str> {
        let mut seen = BTreeSet::new();
        let mut todo = vec![from];
        while let Some(s) = todo.pop() {
            for (_, to) in edges.iter().filter(|(f, _)| *f == s) {
                if seen.insert(*to) {
                    todo.push(to);
                }
            }
        }
        seen
    };
    let candidates: Vec<&str> = (flow.steps.keys().map(String::as_str))
        .filter(|s| !into.contains(s))
        .collect();
    let reached: BTreeMap<&str, BTreeSet<&str>> =
        candidates.iter().map(|c| (*c, reach(c))).collect();
    (candidates.iter())
        .filter(|c| {
            !candidates
                .iter()
                .any(|d| d != *c && reached[d].contains(*c) && !reached[*c].contains(d))
        })
        .map(|c| c.to_string())
        .collect()
}

/// "did you mean" when an id is within edit distance 2, else the known ids.
pub(crate) fn hint_for(token: &str, known: &BTreeSet<&str>) -> Option<String> {
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
        let pos = match (origin, token) {
            (Some((_, text)), Some((t, false))) => locate(text, t),
            (Some((_, text)), Some((t, true))) => locate_key(text, t),
            _ => (None, None),
        };
        self.emit_pos((kind, id), severity, code, pos, message, hint);
    }

    /// (line, column) of byte `at` of `body`, when `body` is the end of the file of `kind`/`id`.
    fn body_pos(
        &self,
        (kind, id): (Kind, &str),
        body: &str,
        at: usize,
    ) -> (Option<usize>, Option<usize>) {
        let Some((_, text)) = self.origins.files.get(&(kind, id.to_string())) else {
            return (None, None);
        };
        let start = text.len().checked_sub(body.len());
        let Some(start) = start.filter(|&s| text.get(s..) == Some(body)) else {
            return (None, None);
        };
        let before = &text[..start + at];
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        (Some(line), Some(col))
    }

    /// Emit a diagnostic of `kind`/`id` at a known position.
    fn emit_pos(
        &mut self,
        (kind, id): (Kind, &str),
        severity: Severity,
        code: &'static str,
        (line, column): (Option<usize>, Option<usize>),
        message: String,
        hint: Option<String>,
    ) {
        let origin = self.origins.files.get(&(kind, id.to_string()));
        let file = origin.map_or_else(|| PathBuf::from(id), |(f, _)| f.clone());
        // a template error is met once per instance: report it once
        let seen = |d: &Diagnostic| {
            (d.code, &d.file, d.line, d.column, &d.message) == (code, &file, line, column, &message)
        };
        if kind == Kind::Template && self.ds.iter().any(seen) {
            return;
        }
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
        (Kind::Command, project.commands.keys().collect()),
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
    // an output is written and unlinked by link: inside the project, never into its
    // sources (`.armadai/`) nor `.git/`, never the project root itself
    for out in m.outputs.values() {
        use std::path::Component;
        let p = Path::new(out);
        let first = p.components().next();
        let bad = p.is_absolute()
            || p.components()
                .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
            || p.components().all(|c| matches!(c, Component::CurDir))
            || first.is_some_and(|c| c.as_os_str() == ".armadai" || c.as_os_str() == ".git");
        if bad {
            o.err(
                man,
                "bad-output",
                Some((out, false)),
                format!("output `{out}` must be a folder inside the project, outside `.armadai/` and `.git/`"),
                Some("use a relative path such as `.claude` or `tools/claude`; `--output` can point elsewhere for one run".into()),
            );
        }
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

    for id in project
        .templates
        .keys()
        .filter(|id| agents.contains(id.as_str()))
    {
        o.err(
            (Kind::Template, id),
            "template-agent-collision",
            None,
            format!("template `{id}` has the same id as agent `{id}`"),
            Some("rename the template or the agent".into()),
        );
    }

    // agents
    for (id, a) in &project.agents {
        // an instance's key it does not set itself comes from (and is reported on) its template
        let at = |key: &str| match &a.instance {
            Some(i) if !i.front.contains_key(key) => (Kind::Template, i.from.as_str()),
            _ => (Kind::Agent, id.as_str()),
        };
        if a.front.description.trim().is_empty() {
            o.err(
                at("description"),
                "empty-description",
                Some(("description", true)),
                "agent description is empty".into(),
                None,
            );
        }
        if let Some(t) = a.front.temperature.filter(|t| !t.is_finite()) {
            o.err(
                at("temperature"),
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
            o.unknown_ref(at("skills"), "unknown-skill", "skill", s, &skills);
        }
        for s in a.front.slots.iter().filter(|s| !is_valid_id(s)) {
            o.err(
                at("slots"),
                "bad-id",
                Some((s, false)),
                format!("slot name `{s}` is not a valid id"),
                Some("ids match [a-z0-9][a-z0-9-]{0,63}".into()),
            );
        }
        o.native_keys(at("native"), a.front.native.keys());
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
        let slot_key = Some(("armadai-slot", true));
        let slots = crate::slots::filled_by(s);
        if slots == [""] {
            o.err(
                at,
                "bad-slot",
                slot_key,
                "`armadai-slot` is empty".into(),
                Some(
                    "name the slots this skill fills, such as `armadai-slot: code-standards`"
                        .into(),
                ),
            );
        } else {
            if slots.contains(&"") {
                o.err(
                    at,
                    "bad-slot",
                    slot_key,
                    "`armadai-slot` has an empty slot name (two commas in a row, or a comma at an end)".into(),
                    Some("separate slot names with single commas, such as `armadai-slot: code-standards, security`".into()),
                );
            }
            for n in (slots.into_iter()).filter(|n| !n.is_empty() && !is_valid_id(n)) {
                o.err(
                    at,
                    "bad-slot",
                    slot_key,
                    format!("slot name `{n}` in `armadai-slot` is not a valid id"),
                    Some("ids match [a-z0-9][a-z0-9-]{0,63}".into()),
                );
            }
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

        // reachability from the entry steps; every edge counts, `on_failure` included
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
        if let Some(s) = f.start.as_deref().filter(|s| !steps.contains(s)) {
            o.unknown_ref(at, "unknown-step", "step", s, &steps);
            continue;
        }
        let entry_ids = entry_steps(f);
        let entries: Vec<&str> = entry_ids.iter().map(String::as_str).collect();
        if entries.is_empty() {
            o.emit(
                at,
                Severity::Warning,
                "no-entry-step",
                None,
                "every step has an incoming unconditional link: the flow can never start".into(),
                None,
            );
        } else {
            // an entry the heuristic took although a link leads to it: only a guess
            if f.start.is_none() && entries.iter().any(|e| edges.iter().any(|(_, to)| to == e)) {
                o.emit(
                    at,
                    Severity::Warning,
                    "ambiguous-entry",
                    None,
                    format!(
                        "an entry step has an incoming link: the flow is taken to begin at {}; \
                         declare `start:` to say where the flow begins",
                        entries.join(", ")
                    ),
                    None,
                );
            }
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

    // commands
    for (id, c) in &project.commands {
        check_command(&mut o, id, c, &skills);
    }
    o.ds
}

/// Tokens of `body` whose meaning differs between the targets: `$<digits>` and
/// `$ARGUMENTS[...]`, each distinct token once with its first byte offset, in order.
fn positional_tokens(body: &str) -> Vec<(usize, &str)> {
    let mut out: Vec<(usize, &str)> = Vec::new();
    for (i, _) in body.match_indices('$') {
        let rest = &body[i + 1..];
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let len = if digits > 0 {
            1 + digits
        } else if let Some(tail) = rest.strip_prefix("ARGUMENTS[") {
            // up to the closing `]` when there is one on the line
            let close = tail
                .find([']', '\n'])
                .filter(|&j| tail[j..].starts_with(']'));
            1 + "ARGUMENTS[".len() + close.map_or(0, |j| j + 1)
        } else {
            continue;
        };
        let t = &body[i..i + len];
        if !out.iter().any(|(_, u)| *u == t) {
            out.push((i, t));
        }
    }
    out
}

fn check_command(o: &mut Out, id: &str, c: &crate::load::Command, skills: &BTreeSet<&str>) {
    let at = (Kind::Command, id);
    if c.front.description.trim().is_empty() {
        o.err(
            at,
            "command-empty-description",
            Some(("description", true)),
            "command description is empty".into(),
            None,
        );
    }
    if c.front
        .argument_hint
        .as_deref()
        .is_some_and(|h| h.trim().is_empty())
    {
        o.err(
            at,
            "command-empty-argument-hint",
            Some(("argument_hint", true)),
            "command `argument_hint` is empty".into(),
            Some("remove it, or describe the expected input, such as `<idea>`".into()),
        );
    }
    if c.body.trim().is_empty() {
        o.err(
            at,
            "command-empty-body",
            None,
            "command body is empty".into(),
            Some("write the text the command sends, citing `$ARGUMENTS`".into()),
        );
    }
    for (i, t) in positional_tokens(&c.body) {
        let message = match t.strip_prefix("$ARGUMENTS[") {
            Some(rest) => format!(
                "`{t}` means something else in each target: Claude Code reads the argument \
                 at index {}, OpenCode replaces `$ARGUMENTS` with the whole input and keeps `[{rest}`",
                rest.trim_end_matches(']')
            ),
            None => format!(
                "`{t}` would be replaced by both Claude Code and OpenCode \
                 (`$<digit>` is a positional argument, indexed from 0 in one and from 1 in the other)"
            ),
        };
        let pos = o.body_pos(at, &c.body, i);
        o.emit_pos(
            at,
            Severity::Error,
            "command-positional-argument",
            pos,
            message,
            Some("only `$ARGUMENTS` is portable: rephrase the text (an amount: `5 USD`)".into()),
        );
    }
    let shell_hint =
        || Some("remove it: ArmadAI never compiles shell execution at command time".into());
    if let Some(i) = c.body.find("!`") {
        let pos = o.body_pos(at, &c.body, i);
        o.emit_pos(
            at,
            Severity::Error,
            "command-shell-expansion",
            pos,
            "`!` before a backtick runs a shell command when the command is typed".into(),
            shell_hint(),
        );
    }
    // byte offset of the first line opening a ```! block
    let block = (c.body.split_inclusive('\n'))
        .scan(0, |off, l| {
            let start = *off + l.len() - l.trim_start().len();
            *off += l.len();
            Some(start)
        })
        .find(|&i| c.body[i..].starts_with("```!"));
    if let Some(i) = block {
        let pos = o.body_pos(at, &c.body, i);
        o.emit_pos(
            at,
            Severity::Error,
            "command-shell-expansion",
            pos,
            "a line starting with ```! runs a shell block when the command is typed".into(),
            shell_hint(),
        );
    }
    if skills.contains(id) {
        o.err(
            at,
            "command-shadows-skill",
            None,
            format!("command `{id}` has the same id as skill `{id}`"),
            Some(
                "Claude Code would run the skill and ignore the command: rename one of them".into(),
            ),
        );
    }
    if BUILTIN_COMMANDS.binary_search(&id).is_ok() {
        o.emit(
            at,
            Severity::Warning,
            "command-shadows-builtin",
            None,
            format!("command `{id}` has the name of a built-in command of Claude Code or OpenCode"),
            Some("one of them hides the other: pick another name".into()),
        );
    }
}
