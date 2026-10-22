//! Canonical writer: stable key order, expanded flows, one trailing newline.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::load::{Agent, Command, Flow, Instance, Project, Skill, Template};
use crate::types::{Automation, FlowFile, Manifest, NativeValue, TemplateFront};

fn yaml<T: Serialize>(v: &T) -> String {
    // Our types hold only strings, numbers and YAML values: serialization cannot fail.
    crate::yaml::to_string(v).expect("canonical types serialize to YAML")
}

fn with_frontmatter<T: Serialize>(front: &T, body: &str) -> String {
    framed(&yaml(front), body)
}

fn framed(yaml: &str, body: &str) -> String {
    let mut out = format!("---\n{yaml}---\n{body}");
    if !body.is_empty() && !body.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn render_manifest(m: &Manifest) -> String {
    yaml(m)
}

/// `id` is the file name and is not written (no `id` key in the frontmatter).
/// An instance is written in its instance form, never flattened.
pub fn render_agent(_id: &str, agent: &Agent) -> String {
    match &agent.instance {
        Some(i) => with_frontmatter(&InstanceFront(i), &i.body),
        None => with_frontmatter(&agent.front, &agent.body),
    }
}

/// An empty frontmatter is written `---\n---\n`, not `{}`.
pub fn render_template(t: &Template) -> String {
    let front = if t.front == TemplateFront::default() {
        String::new()
    } else {
        yaml(&t.front)
    };
    framed(&front, &t.body)
}

/// Top-level keys of an agent frontmatter, in the order the writer puts them.
const AGENT_KEYS: [&str; 9] = [
    "description",
    "title",
    "tier",
    "temperature",
    "tags",
    "skills",
    "slots",
    "capabilities",
    "native",
];

/// `from`, `vars` (when any), then the instance's own keys in the agent key order.
struct InstanceFront<'a>(&'a Instance);

impl Serialize for InstanceFront<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let i = self.0;
        let rank = |k: &str| {
            AGENT_KEYS
                .iter()
                .position(|a| *a == k)
                .unwrap_or(AGENT_KEYS.len())
        };
        let mut own: Vec<_> = i.front.iter().collect();
        own.sort_by_key(|(k, _)| rank(k));
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("from", &i.from)?;
        if !i.vars.is_empty() {
            m.serialize_entry("vars", &i.vars)?;
        }
        for (k, v) in own {
            // NativeValue writes numbers plainly under `arbitrary_precision`
            m.serialize_entry(k, &NativeValue(v.clone()))?;
        }
        m.end()
    }
}

pub fn render_skill(skill: &Skill) -> String {
    with_frontmatter(&skill.front, &skill.body)
}

/// Always the expanded form (`steps` + `links`), never `sequence`.
pub fn render_flow(f: &Flow) -> String {
    yaml(&FlowFile {
        description: f.description.clone(),
        start: f.start.clone(),
        steps: f.steps.clone(),
        links: f.links.clone(),
        sequence: Vec::new(),
    })
}

pub fn render_command(c: &Command) -> String {
    with_frontmatter(&c.front, &c.body)
}

pub fn render_automation(a: &Automation) -> String {
    yaml(a)
}

/// Write `project` under `root/.armadai/`; returns the written paths, sorted.
pub fn write_project(project: &Project, root: &Path) -> std::io::Result<Vec<PathBuf>> {
    // ids become file names: refuse anything that could escape `.armadai/` (before writing)
    let ids = (project.agents.keys())
        .chain(project.templates.keys())
        .chain(project.skills.keys())
        .chain(project.flows.keys())
        .chain(project.automations.keys())
        .chain(project.commands.keys());
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
    for (id, t) in &project.templates {
        files.push((base.join(format!("templates/{id}.md")), render_template(t)));
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
    for (id, c) in &project.commands {
        files.push((base.join(format!("commands/{id}.md")), render_command(c)));
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

#[cfg(test)]
mod tests {
    #[test]
    fn agent_keys_are_the_agent_front_fields() {
        let schema = serde_json::to_value(schemars::schema_for!(crate::types::AgentFront)).unwrap();
        let mut fields: Vec<&str> = (schema["properties"].as_object().unwrap().keys())
            .map(String::as_str)
            .filter(|k| !matches!(*k, "from" | "vars"))
            .collect();
        fields.sort();
        let mut keys = super::AGENT_KEYS.to_vec();
        keys.sort();
        assert_eq!(keys, fields);
    }
}
