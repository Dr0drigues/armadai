//! JSON Schemas of the v1 file formats, generated from the types.

use schemars::{JsonSchema, schema_for};

use crate::types::{AgentFront, Automation, FlowFile, Manifest, SkillFront};

fn render<T: JsonSchema>() -> String {
    let mut s = serde_json::to_string_pretty(&schema_for!(T)).expect("schema serializes");
    s.push('\n');
    s
}

/// (file name, JSON) of every schema, as committed under `schemas/v1/`.
pub fn schemas() -> Vec<(&'static str, String)> {
    vec![
        ("project.schema.json", render::<Manifest>()),
        ("agent.schema.json", render::<AgentFront>()),
        ("skill.schema.json", render::<SkillFront>()),
        ("flow.schema.json", render::<FlowFile>()),
        ("automation.schema.json", render::<Automation>()),
    ]
}
