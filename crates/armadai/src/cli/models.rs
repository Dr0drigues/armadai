use clap::Subcommand;

use std::path::{Path, PathBuf};

use armadai_core::model_aliases::resolve_alias;
use armadai_core::project_registry;

#[derive(Subcommand)]
pub enum ModelsAction {
    /// Check the `native.<target>.model` ids of the project's agents for deprecated ones
    Check {
        /// Check all registered projects
        #[arg(long)]
        all: bool,
        /// Remove stale projects from the registry (with --all)
        #[arg(long)]
        prune: bool,
    },
    /// Replace deprecated `native.<target>.model` ids in the agent files
    Update {
        /// Update all registered projects
        #[arg(long)]
        all: bool,
    },
    /// List registered projects
    List,
}

pub async fn execute(action: ModelsAction) -> anyhow::Result<()> {
    match action {
        ModelsAction::Check { all, prune } => run(all, prune, false),
        ModelsAction::Update { all } => run(all, false, true),
        ModelsAction::List => list(),
    }
}

/// A deprecated model id in an agent's `native.<target>.model` (canonical agents
/// carry a tier; a model id only lives there).
#[derive(Debug, Clone, PartialEq)]
struct Finding {
    agent: String,
    target: String,
    current: String,
    replacement: String,
}

impl Finding {
    /// A `latest:*` replacement is a tier, not an id: reported, not rewritten.
    fn rewritable(&self) -> bool {
        !self.replacement.starts_with("latest")
    }
}

fn findings(root: &Path) -> anyhow::Result<Vec<Finding>> {
    let (project, _) = armadai_model::load_checked(root).map_err(|ds| {
        anyhow::anyhow!(
            "{} is invalid ({} diagnostic(s)): run `armadai validate`",
            root.display(),
            ds.len()
        )
    })?;
    let mut out = Vec::new();
    for (id, agent) in &project.agents {
        for (target, native) in &agent.front.native {
            let Some(current) = native.get("model").and_then(|m| m.as_str()) else {
                continue;
            };
            if let Some(replacement) = resolve_alias(current) {
                out.push(Finding {
                    agent: id.clone(),
                    target: target.clone(),
                    current: current.to_string(),
                    replacement,
                });
            }
        }
    }
    Ok(out)
}

/// Replace `current` by `replacement` where it is the whole value of a `model:` key
/// (block `model: x` at the start of a line, or flow `{ model: x }` / `, model: x`),
/// bare or quoted; never inside a comment, another key's value, or a longer id.
fn replace_model_values(front: &str, current: &str, replacement: &str) -> (String, usize) {
    let mut out = String::with_capacity(front.len());
    let mut n = 0;
    for line in front.split_inclusive('\n') {
        let mut rest = line;
        let mut done = String::new();
        while let Some(i) = rest.find("model") {
            let before = rest[..i].trim_end_matches([' ', '\t']);
            let key_position = (done.is_empty() && before.is_empty())
                || before.ends_with('{')
                || before.ends_with(',');
            let after = &rest[i + "model".len()..];
            let value = after
                .trim_start_matches([' ', '\t'])
                .strip_prefix(':')
                .map(|v| v.trim_start_matches([' ', '\t']));
            let hit = key_position
                && value.is_some_and(|v| {
                    let (q, v) = match v.chars().next() {
                        Some(c @ ('\'' | '"')) => (Some(c), &v[1..]),
                        _ => (None, v),
                    };
                    v.strip_prefix(current).is_some_and(|tail| {
                        let tail = match q {
                            Some(q) => match tail.strip_prefix(q) {
                                Some(t) => t,
                                None => return false,
                            },
                            None => tail,
                        };
                        let tail = tail.trim_start_matches([' ', '\t']);
                        tail.is_empty() || tail.starts_with([',', '}', '#', '\r', '\n'])
                    })
                });
            if !hit {
                done.push_str(&rest[..i + "model".len()]);
                rest = &rest[i + "model".len()..];
                continue;
            }
            let v = value.unwrap_or_default();
            let start = rest.len() - v.len();
            let quoted = usize::from(v.starts_with(['\'', '"']));
            done.push_str(&rest[..start + quoted]);
            done.push_str(replacement);
            rest = &rest[start + quoted + current.len()..];
            n += 1;
        }
        out.push_str(&done);
        out.push_str(rest);
    }
    (out, n)
}

/// Replace the deprecated ids of every agent of `root`, in memory first: written
/// only when each replacement landed on its `model:` value and the project still
/// validates; the previous files are restored otherwise.
fn apply(root: &Path, findings: &[Finding]) -> anyhow::Result<usize> {
    let mut by_agent: std::collections::BTreeMap<&str, Vec<&Finding>> = Default::default();
    for f in findings.iter().filter(|f| f.rewritable()) {
        by_agent.entry(&f.agent).or_default().push(f);
    }
    let mut planned = Vec::new();
    for (agent, fs) in &by_agent {
        let path = root.join(".armadai/agents").join(format!("{agent}.md"));
        let text = std::fs::read_to_string(&path)?;
        let (yaml, _, _) = armadai_model::split_frontmatter(&text)
            .ok_or_else(|| anyhow::anyhow!("{}: no frontmatter", path.display()))?;
        let start = yaml.as_ptr() as usize - text.as_ptr() as usize;
        let mut front = yaml.to_string();
        let mut seen = std::collections::BTreeSet::new();
        // the same id pinned for two targets is one replacement of both
        for f in fs.iter().filter(|f| seen.insert(f.current.as_str())) {
            let (next, hits) = replace_model_values(&front, &f.current, &f.replacement);
            if hits == 0 {
                anyhow::bail!(
                    "{}: `{}` not found as a `model:` value; nothing was rewritten",
                    path.display(),
                    f.current
                );
            }
            front = next;
        }
        let new = format!("{}{front}{}", &text[..start], &text[start + yaml.len()..]);
        planned.push((path, text, new, fs.len()));
    }
    for (path, _, new, _) in &planned {
        std::fs::write(path, new)?;
    }
    if armadai_model::load_checked(root).is_err() {
        for (path, old, _, _) in &planned {
            std::fs::write(path, old)?;
        }
        anyhow::bail!(
            "{}: the project would no longer validate; nothing was rewritten",
            root.display()
        );
    }
    let mut n = 0;
    for (path, _, _, count) in &planned {
        n += count;
        let o = crate::cli::style::ok();
        anstream::println!(
            "{o}  updated {}: {count} replacement(s){o:#}",
            path.display()
        );
    }
    Ok(n)
}

fn print_findings(findings: &[Finding]) {
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    let w = crate::cli::style::warn();
    let o = crate::cli::style::ok();
    for f in findings {
        let note = if f.rewritable() {
            String::new()
        } else {
            format!(
                " {m}(a tier: set `tier:` and drop native.{}.model){m:#}",
                f.target
            )
        };
        anstream::println!(
            "  {a}{}{a:#} {m}[native.{}.model]{m:#}: {w}{}{w:#} -> {o}{}{o:#}{note}",
            f.agent,
            f.target,
            f.current,
            f.replacement
        );
    }
}

/// The projects to look at: the current one, or every registered one.
fn projects(all: bool, prune: bool) -> anyhow::Result<Vec<PathBuf>> {
    if !all {
        return Ok(vec![crate::compile::find_project(
            &std::env::current_dir()?
        )?]);
    }
    let mut registry = project_registry::load();
    if prune {
        let pruned = project_registry::prune_stale(&mut registry);
        if !pruned.is_empty() {
            let m = crate::cli::style::muted();
            anstream::println!("{m}Pruned {} stale project(s):{m:#}", pruned.len());
            for p in &pruned {
                anstream::println!("  - {p}");
            }
            project_registry::save(&registry)?;
        }
    }
    Ok(registry
        .projects
        .iter()
        .map(|e| PathBuf::from(&e.path))
        .collect())
}

fn run(all: bool, prune: bool, update: bool) -> anyhow::Result<()> {
    let roots = projects(all, prune)?;
    if roots.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!(
            "{m}No registered projects. Run `armadai link` in a project first.{m:#}"
        );
        return Ok(());
    }
    let (mut found, mut updated, mut failed) = (0, 0, false);
    for root in &roots {
        if !root.join(".armadai/project.yaml").is_file() {
            let w = crate::cli::style::warn();
            anstream::eprintln!(
                "{w}  warn: {}: legacy format, run `armadai migrate`{w:#}",
                root.display()
            );
            continue;
        }
        let fs = match findings(root) {
            Ok(f) => f,
            Err(e) => {
                let w = crate::cli::style::warn();
                anstream::eprintln!("{w}  warn: {e}{w:#}");
                failed = true;
                continue;
            }
        };
        if fs.is_empty() {
            continue;
        }
        if all {
            let h = crate::cli::style::header();
            anstream::println!("{h}{}:{h:#}", root.display());
        }
        print_findings(&fs);
        found += fs.len();
        if update {
            match apply(root, &fs) {
                Ok(n) => updated += n,
                Err(e) => {
                    let er = crate::cli::style::err();
                    anstream::eprintln!("{er}  error: {e}{er:#}");
                    failed = true;
                }
            }
        }
    }
    if found == 0 {
        let o = crate::cli::style::ok();
        anstream::println!("{o}All models are up to date.{o:#}");
    } else if update {
        let o = crate::cli::style::ok();
        anstream::println!("{o}\n{updated} model(s) updated.{o:#}");
    } else {
        let w = crate::cli::style::warn();
        let all_flag = if all { " --all" } else { "" };
        anstream::println!(
            "\n{w}{found} deprecated model(s) found. Run `armadai models update{all_flag}` to fix.{w:#}"
        );
    }
    if failed {
        anyhow::bail!("one or more project(s) could not be checked or updated (see above)");
    }
    Ok(())
}

fn list() -> anyhow::Result<()> {
    let registry = project_registry::load();

    if registry.projects.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}No registered projects.{m:#}");
        anstream::println!("{m}Projects are auto-registered when you run `armadai link`.{m:#}");
        return Ok(());
    }

    let h = crate::cli::style::header();
    anstream::println!("{h}Registered projects:{h:#}\n");
    for entry in &registry.projects {
        let a = crate::cli::style::accent();
        let m = crate::cli::style::muted();
        anstream::println!(
            "  {a}{}{a:#}  {m}(last seen: {}){m:#}",
            entry.path,
            entry.last_seen
        );
    }
    let m = crate::cli::style::muted();
    anstream::println!("\n{m}{} project(s) total.{m:#}", registry.projects.len());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::replace_model_values;

    #[test]
    fn only_model_values_are_replaced_as_whole_tokens() {
        let front = "# gpt-4-turbo is deprecated, see model: gpt-4-turbo\r\n\
                     description: 'moves off gpt-4-turbo; model: gpt-4-turbo'\r\n\
                     native:\r\n  codex:\r\n    model: gpt-4-turbo\r\n  \
                     opencode: { model: \"gpt-4-turbo\", temperature: 1 }\r\n  \
                     copilot:\r\n    model: 'gpt-4-turbo' # pinned\r\n  \
                     claude:\r\n    model: gpt-4-turbo-preview\r\n";
        let (out, n) = replace_model_values(front, "gpt-4-turbo", "gpt-4o");
        assert_eq!(n, 3, "{out}");
        assert!(out.contains("    model: gpt-4o\r\n"), "{out}");
        assert!(
            out.contains("{ model: \"gpt-4o\", temperature: 1 }"),
            "{out}"
        );
        assert!(out.contains("    model: 'gpt-4o' # pinned\r\n"), "{out}");
        assert!(
            out.contains("model: gpt-4-turbo-preview\r\n"),
            "a longer id is another id"
        );
        assert!(out.starts_with("# gpt-4-turbo is deprecated, see model: gpt-4-turbo\r\n"));
        assert!(out.contains("description: 'moves off gpt-4-turbo; model: gpt-4-turbo'"));
    }

    #[test]
    fn a_prefix_alias_does_not_touch_a_longer_id() {
        let front = "native:\n  codex: { model: gpt-4o-mini }\n  opencode:\n    model: gpt-4\n";
        let (out, n) = replace_model_values(front, "gpt-4", "gpt-4o");
        assert_eq!(n, 1);
        assert!(
            out.contains("{ model: gpt-4o-mini }") && out.contains("    model: gpt-4o\n"),
            "{out}"
        );
    }
}
