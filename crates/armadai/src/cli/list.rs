//! `armadai list`: the agents, skills, flows and automations of the project.
//!
//! Read-only: a project with errors is still listed (diagnostics on stderr), as far
//! as it loads.

use armadai_model::{Project, Tier};

/// The project at or above the current directory, as far as it loads; every
/// diagnostic goes to stderr.
pub fn load_for_reading() -> anyhow::Result<Project> {
    let root = crate::compile::find_project(&std::env::current_dir()?)?;
    let loaded = armadai_model::load_project(&root);
    let mut ds = loaded.diagnostics;
    if let Some(p) = &loaded.project {
        ds.extend(armadai_model::check(p, &loaded.origins));
    }
    for d in &ds {
        let s = crate::cli::style::warn();
        anstream::eprintln!("{s}{}{s:#}", d.render());
    }
    loaded
        .project
        .ok_or_else(|| anyhow::anyhow!("the project manifest could not be read"))
}

pub fn tier_str(t: Option<Tier>) -> &'static str {
    match t {
        Some(Tier::Fast) => "fast",
        Some(Tier::Pro) => "pro",
        Some(Tier::Max) => "max",
        None => "-",
    }
}

pub async fn execute(tags: Option<Vec<String>>) -> anyhow::Result<()> {
    let project = load_for_reading()?;
    let agents: Vec<_> = (project.agents.iter())
        .filter(|(_, a)| {
            tags.as_ref()
                .is_none_or(|t| t.iter().any(|t| a.front.tags.contains(t)))
        })
        .collect();

    let h = crate::cli::style::header();
    let m = crate::cli::style::muted();
    let acc = crate::cli::style::accent();
    if agents.is_empty() {
        anstream::println!("{m}No agents found.{m:#}");
    } else {
        let w = agents
            .iter()
            .map(|(id, _)| id.len())
            .max()
            .unwrap_or(2)
            .max(2);
        anstream::println!("{h}  {:<w$}  TIER  TAGS  DESCRIPTION{h:#}", "ID");
        for (id, a) in &agents {
            let role = if project.manifest.coordinator.as_deref() == Some(id.as_str()) {
                " (coordinator)"
            } else {
                ""
            };
            let tags = if a.front.tags.is_empty() {
                "-".to_string()
            } else {
                a.front.tags.join(", ")
            };
            anstream::println!(
                "  {acc}{:<w$}{acc:#}  {:<4}  {tags}  {}{role}",
                id,
                tier_str(a.front.tier),
                a.front.description.lines().next().unwrap_or_default(),
            );
        }
        anstream::println!("\n{m}  {} agent(s) found.{m:#}", agents.len());
    }
    for (title, ids) in [
        ("Skills", project.skills.keys().collect::<Vec<_>>()),
        ("Flows", project.flows.keys().collect()),
        ("Automations", project.automations.keys().collect()),
    ] {
        if !ids.is_empty() {
            let ids: Vec<&str> = ids.into_iter().map(String::as_str).collect();
            anstream::println!("{h}{title}:{h:#} {}", ids.join(", "));
        }
    }
    Ok(())
}
