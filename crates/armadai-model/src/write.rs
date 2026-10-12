//! Canonical writer: stable key order, expanded flows, one trailing newline.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::load::{Agent, Flow, Project, Skill};
use crate::types::{Automation, FlowFile, Manifest};

fn yaml<T: Serialize>(v: &T) -> String {
    // Our types hold only strings, numbers and YAML values: serialization cannot fail.
    serde_yaml_ng::to_string(v).expect("canonical types serialize to YAML")
}

fn with_frontmatter<T: Serialize>(front: &T, body: &str) -> String {
    let mut out = format!("---\n{}---\n{body}", yaml(front));
    if !body.is_empty() && !body.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn render_manifest(m: &Manifest) -> String {
    yaml(m)
}

/// `id` is the file name and is not written (no `id` key in the frontmatter).
pub fn render_agent(_id: &str, agent: &Agent) -> String {
    with_frontmatter(&agent.front, &agent.body)
}

pub fn render_skill(skill: &Skill) -> String {
    with_frontmatter(&skill.front, &skill.body)
}

/// Always the expanded form (`steps` + `links`), never `sequence`.
pub fn render_flow(f: &Flow) -> String {
    yaml(&FlowFile {
        description: f.description.clone(),
        steps: f.steps.clone(),
        links: f.links.clone(),
        sequence: Vec::new(),
    })
}

pub fn render_automation(a: &Automation) -> String {
    yaml(a)
}

/// Write `project` under `root/.armadai/`; returns the written paths, sorted.
pub fn write_project(project: &Project, root: &Path) -> std::io::Result<Vec<PathBuf>> {
    // ids become file names: refuse anything that could escape `.armadai/` (before writing)
    let ids = (project.agents.keys())
        .chain(project.skills.keys())
        .chain(project.flows.keys())
        .chain(project.automations.keys());
    if let Some(bad) = ids.into_iter().find(|id| !crate::is_valid_id(id)) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("`{bad}` is not a valid id"),
        ));
    }
    let base = root.join(".armadai");
    let mut files: Vec<(PathBuf, String)> = vec![(
        base.join("project.yaml"),
        render_manifest(&project.manifest),
    )];
    for (id, a) in &project.agents {
        files.push((base.join(format!("agents/{id}.md")), render_agent(id, a)));
    }
    for (id, s) in &project.skills {
        files.push((base.join(format!("skills/{id}/SKILL.md")), render_skill(s)));
    }
    for (id, f) in &project.flows {
        files.push((base.join(format!("flows/{id}.yaml")), render_flow(f)));
    }
    for (id, a) in &project.automations {
        files.push((
            base.join(format!("automations/{id}.yaml")),
            render_automation(a),
        ));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, text) in &files {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, text)?;
    }
    Ok(files.into_iter().map(|(p, _)| p).collect())
}
