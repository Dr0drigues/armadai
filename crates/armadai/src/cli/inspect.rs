//! `armadai inspect <id>`: one agent of the project, as declared, and the flows
//! it takes part in.

pub fn execute(id: String) -> anyhow::Result<()> {
    let project = crate::cli::list::load_for_reading()?;
    let Some(agent) = project.agents.get(&id) else {
        let known: Vec<&str> = project.agents.keys().map(String::as_str).collect();
        anyhow::bail!(
            "no agent `{id}` in the project (agents: {})",
            if known.is_empty() {
                "none".into()
            } else {
                known.join(", ")
            }
        );
    };
    let f = &agent.front;
    let h = crate::cli::style::header();
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    anstream::println!("{h}Agent:{h:#} {a}{id}{a:#}");
    anstream::println!("{m}Source: .armadai/agents/{id}.md{m:#}\n");
    let field = |name: &str, value: String| {
        anstream::println!("{m}  {name:<13}{m:#} {value}");
    };
    field("Description:", f.description.clone());
    if let Some(t) = &f.title {
        field("Title:", t.clone());
    }
    if project.manifest.coordinator.as_deref() == Some(id.as_str()) {
        field("Role:", "coordinator".into());
    }
    field("Tier:", crate::cli::list::tier_str(f.tier).into());
    if let Some(t) = f.temperature {
        field("Temperature:", t.to_string());
    }
    if !f.tags.is_empty() {
        field("Tags:", f.tags.join(", "));
    }
    if !f.skills.is_empty() {
        field("Skills:", f.skills.join(", "));
    }
    let caps = armadai_model::yaml::to_string(&f.capabilities)?;
    if caps.trim() != "{}" {
        field("Capabilities:", caps.trim().replace('\n', ", "));
    }
    if !f.native.is_empty() {
        let targets: Vec<&str> = f.native.keys().map(String::as_str).collect();
        field("Native:", targets.join(", "));
    }
    let flows: Vec<&str> = (project.flows.iter())
        .filter(|(_, fl)| fl.steps.values().any(|s| s.agent == id))
        .map(|(fid, _)| fid.as_str())
        .collect();
    if !flows.is_empty() {
        field("Flows:", flows.join(", "));
    }
    anstream::println!("\n{h}## Prompt{h:#}");
    for line in agent.body.lines() {
        println!("  {line}");
    }
    Ok(())
}
